//! `KIOSK_SNAP_BENCH`: what a frame costs, measured past the window.
//!
//! A hidden window on macOS is presented at the compositor's pace (about
//! 57 Hz), so its frame rate says nothing about the renderer. Measured per
//! frame instead:
//! - `main`: the main schedule (`First` to `Last`), the game's own systems;
//! - `render thread`: the render schedule less `ManageViews`, where the wait
//!   for the swap-chain texture (the throttle) happens; `graph` is its
//!   `render_system` alone (encoding and submit);
//! - `gpu`: between stage-boundary timestamps of two empty passes submitted
//!   first and last in the frame's `RenderGraph` schedule (Apple GPUs take no
//!   other kind).
//!
//! The GPU lowers its clock until the work fills the paced frame, so at the
//! real size `gpu` is only an upper bound and barely moves between versions.
//! To compare GPU costs, render at several times the pixels (`KIOSK_SIZE`)
//! until the frame rate falls below the pacing: the `mean` frame time is then
//! the true cost, and it grows about linearly with the pixels.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::prelude::*;

use bevy::render::renderer::{render_system, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue};
use bevy::render::{Render, RenderApp, RenderSystems};

/// Milliseconds per frame, drained by `snap::drive`.
#[derive(Resource, Clone, Default)]
pub struct Costs {
    pub cpu: Arc<Mutex<Vec<f32>>>,
    /// GPU time between the graph's first and last pass
    pub gpu_pass: Arc<Mutex<Vec<f32>>>,
    /// the render thread's `render_system` (graph encoding and submit)
    pub encode: Arc<Mutex<Vec<f32>>>,
    /// the render schedule, extracted commands to cleanup, less `ManageViews`
    pub render: Arc<Mutex<Vec<f32>>>,
    /// GPU time of the passes bracketed with [`PassStamps`] (the ray-marched
    /// fire and smoke, `fire_volumetric`)
    pub volume: Arc<Mutex<Vec<f32>>>,
}

#[derive(Resource)]
struct GpuProbe {
    start: Option<Instant>,
    encode: Arc<Mutex<Vec<f32>>>,
    frame: Option<Instant>,
    before_views: Option<f32>,
    render: Arc<Mutex<Vec<f32>>>,
}

#[derive(Resource, Default)]
struct CpuStart(Option<Instant>);

pub struct BenchPlugin;

impl Plugin for BenchPlugin {
    fn build(&self, app: &mut App) {
        let costs = Costs::default();
        app.insert_resource(costs.clone())
            .init_resource::<CpuStart>()
            .add_systems(First, |mut s: ResMut<CpuStart>| s.0 = Some(Instant::now()))
            .add_systems(Last, |s: Res<CpuStart>, c: Res<Costs>| {
                if let Some(t) = s.0 {
                    c.cpu.lock().unwrap().push(t.elapsed().as_secs_f32() * 1e3);
                }
            });
    }

    // the render device exists only once the renderer has started
    fn finish(&self, app: &mut App) {
        let costs = app.world().resource::<Costs>().clone();
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        let device = render.world().resource::<RenderDevice>().clone();
        if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            let period = render.world().resource::<bevy::render::renderer::RenderQueue>().get_timestamp_period();
            let stamps = Stamps::new(&device, period, costs.gpu_pass.clone());
            render.insert_resource(PassStamps(Arc::new(Stamps::new(&device, period, costs.volume.clone()))));
            render.insert_resource(StampsRes(Arc::new(stamps)));
            // Each stamp is its own submission: the first before the cameras'
            // command buffers (submitted in `Submit`), the last after them.
            render.add_systems(RenderGraph, (
                (|st: Res<StampsRes>, d: Res<RenderDevice>, q: Res<RenderQueue>| stamp(&st.0, &d, &q, false)).in_set(RenderGraphSystems::Begin),
                (|st: Res<StampsRes>, d: Res<RenderDevice>, q: Res<RenderQueue>| stamp(&st.0, &d, &q, true)).in_set(RenderGraphSystems::Finish),
            ));
        } else {
            warn!("bench: no GPU timestamps on this device");
        }
        render
            .insert_resource(GpuProbe { start: None, frame: None, before_views: None, encode: costs.encode.clone(), render: costs.render.clone() })
            .add_systems(Render, (|mut p: ResMut<GpuProbe>| p.start = Some(Instant::now())).in_set(RenderSystems::Render).before(render_system))
            .add_systems(Render, (|mut p: ResMut<GpuProbe>| p.frame = Some(Instant::now())).in_set(RenderSystems::ExtractCommands))
            // the swap-chain texture is acquired in `ManageViews`: the throttle, left out
            .add_systems(Render, (|mut p: ResMut<GpuProbe>| p.before_views = p.frame.map(|t| t.elapsed().as_secs_f32())).in_set(RenderSystems::PrepareAssets))
            .add_systems(Render, (|mut p: ResMut<GpuProbe>| p.frame = Some(Instant::now())).in_set(RenderSystems::Queue))
            .add_systems(Render, (|mut p: ResMut<GpuProbe>| {
                if let Some(t) = p.frame.take() {
                    let before = p.before_views.take().unwrap_or(0.0);
                    p.render.lock().unwrap().push((before + t.elapsed().as_secs_f32()) * 1e3);
                }
            }).in_set(RenderSystems::Cleanup))
            .add_systems(Render, (|mut p: ResMut<GpuProbe>, st: Option<Res<StampsRes>>, ps: Option<Res<PassStamps>>| {
                if let Some(st) = st {
                    st.0.map_pending();
                }
                if let Some(ps) = ps {
                    ps.0.map_pending();
                }
                if let Some(t) = p.start.take() {
                    p.encode.lock().unwrap().push(t.elapsed().as_secs_f32() * 1e3);
                }
            }).in_set(RenderSystems::Render).after(render_system));
    }
}

struct Stamps {
    set: wgpu::QuerySet,
    target: wgpu::TextureView,
    resolve: wgpu::Buffer,
    /// read-back buffers and whether each is free
    ring: Vec<(Arc<wgpu::Buffer>, Arc<std::sync::atomic::AtomicBool>)>,
    next: std::sync::atomic::AtomicUsize,
    period_ns: f32,
    out: Arc<Mutex<Vec<f32>>>,
    /// ring slots copied into this frame, mapped once it is submitted
    pending: Mutex<Vec<usize>>,
}

impl Stamps {
    fn new(device: &RenderDevice, period_ns: f32, out: Arc<Mutex<Vec<f32>>>) -> Self {
        let dev = device.wgpu_device();
        let tex = dev.create_texture(&wgpu::TextureDescriptor {
            label: Some("bench_stamp"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let buf = |usage| dev.create_buffer(&wgpu::BufferDescriptor { label: Some("bench_stamp"), size: 16, usage, mapped_at_creation: false });
        Stamps {
            set: dev.create_query_set(&wgpu::QuerySetDescriptor { label: Some("bench_stamp"), ty: wgpu::QueryType::Timestamp, count: 2 }),
            target: tex.create_view(&Default::default()),
            resolve: buf(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            ring: (0..8).map(|_| (Arc::new(buf(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ)), Arc::new(std::sync::atomic::AtomicBool::new(true)))).collect(),
            next: Default::default(),
            period_ns,
            out,
            pending: Default::default(),
        }
    }

    /// Copies both stamps out for reading (once the frame is submitted).
    fn read_back(&self, enc: &mut wgpu::CommandEncoder) {
        use std::sync::atomic::Ordering;
        let n = self.next.fetch_add(1, Ordering::Relaxed) % self.ring.len();
        let (buf, free) = &self.ring[n];
        // still mapped from an earlier frame: skip this one
        if free.swap(false, Ordering::AcqRel) {
            enc.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
            enc.copy_buffer_to_buffer(&self.resolve, 0, buf, 0, 16);
            self.pending.lock().unwrap().push(n);
        }
    }

    fn map_pending(&self) {
        use std::sync::atomic::Ordering;
        for n in self.pending.lock().unwrap().drain(..) {
            let (buf, free) = &self.ring[n];
            let (b, free, out, period) = (buf.clone(), free.clone(), self.out.clone(), self.period_ns);
            buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                if r.is_ok() {
                    let t: [u64; 2] = {
                        let v = b.slice(..).get_mapped_range().expect("mapped for reading");
                        [u64::from_le_bytes(v[0..8].try_into().unwrap()), u64::from_le_bytes(v[8..16].try_into().unwrap())]
                    };
                    b.unmap();
                    if t[1] > t[0] {
                        out.lock().unwrap().push((t[1] - t[0]) as f32 * period * 1e-6);
                    }
                }
                free.store(true, Ordering::Release);
            });
        }
    }
}

#[derive(Resource, Clone)]
struct StampsRes(Arc<Stamps>);

/// Stamps for a run of passes inside a frame: [`Self::start`] before the
/// first, the last takes [`Self::end`], then [`Self::read_back`] on the
/// same encoder; the time between lands in [`Costs::volume`]. In the render
/// world only while the bench is on and the device has timestamps.
#[derive(Resource, Clone)]
pub struct PassStamps(Arc<Stamps>);

impl PassStamps {
    /// An empty pass whose end is the start of the time measured. A pass's
    /// own beginning will not do: on a tile-based GPU that is the start of
    /// its vertex stage, which runs while the passes before still shade
    /// (the main pass's fragments were counted in, about 1.6 ms at
    /// 4800x3000, 2026-11-05).
    ///
    /// What was observed on an M4 Pro, not more: wherever this pass sits
    /// among the three of the volumes, the time read is from before the
    /// first of them to the end of the one that takes [`Self::end`]. So
    /// the ends are right and their differences are each pass's cost; the
    /// origin was checked only by a march of one step (0.4 ms for 0.9
    /// million rays) and a composite with nothing to draw (0.34 ms).
    pub fn start(&self, enc: &mut wgpu::CommandEncoder) {
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bench_stamp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.0.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &self.0.set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(0),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    pub fn end(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites { query_set: &self.0.set, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(1) }
    }

    pub fn read_back(&self, enc: &mut wgpu::CommandEncoder) {
        self.0.read_back(enc);
    }
}

/// An empty render pass whose stage-boundary timestamp brackets the frame's
/// GPU work; the last one also copies both stamps out for reading.
fn stamp(st: &Stamps, device: &RenderDevice, queue: &RenderQueue, end: bool) {
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bench_stamp") });
    {
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bench_stamp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &st.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &st.set,
                beginning_of_pass_write_index: (!end).then_some(0),
                end_of_pass_write_index: end.then_some(1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    if end {
        st.read_back(&mut enc);
    }
    queue.submit([enc.finish()]);
}
