//! The flames as a fluid (2026-11-07, user: «use some simplified
//! fluidodynamics to simulate flames»; asked which, «proviamo la 3d, poi
//! scaliamo nel caso»).
//!
//! A grid of [`N`] x [`N`] x [`K`] voxels, [`DX`] m a side and [`DZ`] m
//! up, counted from the drawn ground, round the point the camera looks at
//! (192 m a side, 32 m up). Each frame, on the GPU (`shaders/
//! fire_fluid.wesl`):
//!
//! 1. **advection** of velocity and heat, semi-Lagrangian (Stam), and the
//!    forces: the ground alight under the grid (the front map on the
//!    drawing clock, `fire_ahead`) breathes heat in, flickering; the heat
//!    lifts the air (buoyancy) against a drag; the wind holds the air at its
//!    speed, less inside the rise; the heat cools so that a flame's tip
//!    stands about at the flame height of the fire under it (twice Byram's
//!    length, as the volumes have always had it: `fire_view::vol_flame_m`);
//! 2. **vorticity confinement** (Fedkiw): the swirl a coarse grid loses,
//!    given back where it is warm;
//! 3. **pressure projection**: the divergence, [`JACOBI`] Jacobi sweeps,
//!    the gradient taken off. The rise draws the air in at its foot: the
//!    flames lean into the fire's own indraft and with the wind, without
//!    being told to.
//!
//! The march (`fire_march.wesl`, `flame_at`) reads the heat where the grid
//! is (faded over its last 40 m, and off where the flames are drawn at toy
//! scale), the noise tongues of before elsewhere; the tongues' noise stays
//! on as the detail finer than a voxel.
//!
//! The grid is fixed to the world and wraps: when the focus moves only the
//! voxels that come in start anew (with the wind's air), and it is never
//! copied. Nothing is stepped while no flame is in it, or the camera is
//! farther than [`FAR_M`] from the point it looks at.
//!
//! Off unless `KIOSK_FLUIDO=1` (in the browser `?fluido=1`), with the
//! volumes (`KIOSK_VOLUMI=1`). Drawing only: nothing of the game reads it.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_2d, texture_3d, texture_storage_3d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use crate::camera::OrbitCamera;
use crate::sim::Sim;

const SHADER: Handle<Shader> = uuid_handle!("5e1d9a73-2c84-4b6f-9d07-a38e1f6c52b4");

/// Voxels a side, and up.
pub const N: u32 = 128;
pub const K: u32 = 32;
/// Metres a voxel spans across and up.
pub const DX: f32 = 1.5;
pub const DZ: f32 = 1.0;
/// Jacobi sweeps a frame.
const JACOBI: usize = 24;
/// Past this distance from the camera to its focus (m) nothing is stepped:
/// the flames are a few pixels, and drawn at toy scale.
const FAR_M: f32 = 1600.0;

/// Whether the flames are a fluid: `KIOSK_FLUIDO=1`, `?fluido=1`, with the
/// volumes.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let asked = if cfg!(target_arch = "wasm32") { crate::quality::web_param("fluido") } else { std::env::var("KIOSK_FLUIDO").ok() };
        crate::fire_volumetric::on() && asked.as_deref() == Some("1")
    })
}

/// The knobs (`KIOSK_FLUIDO_SET=nome=valore,…`, as `KIOSK_VOL_SET`).
#[derive(Clone, Copy, Debug)]
pub struct FluidSettings {
    /// Buoyancy at heat 1 (m/s²).
    pub buoyancy: f32,
    /// How fast the wind takes the air back (1/s).
    pub hold: f32,
    /// Confinement.
    pub swirl: f32,
    /// The rise's drag (1/s).
    pub drag: f32,
    /// The source's height (m) and heat.
    pub source_m: f32,
    pub heat: f32,
    /// The heat at the flame's tip, and the speed its top is reckoned at.
    pub tip: f32,
    pub rise: f32,
    /// How far the air bends the tongues (1: as far as it carries them).
    pub lean: f32,
}

impl Default for FluidSettings {
    fn default() -> Self {
        FluidSettings { buoyancy: 40.0, hold: 0.8, swirl: 0.35, drag: 0.9, source_m: 2.5, heat: 1.0, tip: 0.3, rise: 3.5, lean: 1.6 }
    }
}

impl FluidSettings {
    fn from_env() -> Self {
        let mut s = FluidSettings::default();
        if let Ok(spec) = std::env::var("KIOSK_FLUIDO_SET") {
            for kv in spec.split(',') {
                let Some((k, v)) = kv.split_once('=') else { continue };
                let Ok(v) = v.trim().parse::<f32>() else { continue };
                match k.trim() {
                    "galleggiamento" => s.buoyancy = v,
                    "vento" => s.hold = v,
                    "vortici" => s.swirl = v,
                    "attrito" => s.drag = v,
                    "sorgente" => s.source_m = v,
                    "calore" => s.heat = v,
                    "punta" => s.tip = v.clamp(0.02, 0.9),
                    "salita" => s.rise = v,
                    "piega" => s.lean = v,
                    _ => warn!("KIOSK_FLUIDO_SET: {k}?"),
                }
            }
        }
        s
    }
}

/// What the render world needs this frame; `None` while nothing is stepped.
#[derive(Resource, Clone, Copy, Default)]
pub struct FluidFrame {
    pub frame: Option<FluidNow>,
}

#[derive(Clone, Copy)]
pub struct FluidNow {
    /// The grid's first voxel (world index, x and Bevy z).
    pub origin: IVec2,
    pub wind: Vec2,
    pub dt: f32,
    pub time: f32,
    /// The front map's clock (`fire_ahead`).
    pub front_rel: f32,
    pub settings: FluidSettings,
}

impl FluidNow {
    /// For the march: the grid's corner (m), its voxel, 1; then voxels a
    /// side, up, the voxel's height and how far the air bends the tongues.
    pub fn march(&self) -> (Vec4, Vec4) {
        (
            Vec4::new(self.origin.x as f32 * DX, self.origin.y as f32 * DX, DX, 1.0),
            Vec4::new(N as f32, K as f32, DZ, self.settings.lean),
        )
    }
}

pub struct FireFluidPlugin;

impl Plugin for FireFluidPlugin {
    fn build(&self, app: &mut App) {
        if !on() {
            return;
        }
        crate::load_wesl!(app, SHADER, "shaders/fire_fluid.wesl");
        app.init_resource::<FluidFrame>()
            .insert_resource(Settings(FluidSettings::from_env()))
            .add_systems(Update, focus.after(crate::fire_ahead::update).run_if(in_state(crate::AppState::Playing)));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init)
            .add_systems(ExtractSchedule, extract)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources));
    }
}

#[derive(Resource)]
struct Settings(FluidSettings);

/// Where the grid stands this frame, and whether it is stepped.
fn focus(
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<Settings>,
    ahead: Res<crate::fire_ahead::FireAhead>,
    cams: Query<&OrbitCamera>,
    mut out: ResMut<FluidFrame>,
) {
    out.frame = None;
    let Some(cam) = cams.iter().next() else { return };
    if cam.distance > FAR_M || !crate::fire_ahead::on() {
        return;
    }
    let half = N as f32 * DX * 0.5;
    let origin = ((Vec2::new(cam.focus.x, cam.focus.z) - half) / DX).floor().as_ivec2();
    // a flame in the grid, or about to be
    let w = sim.scn.world;
    let (lo, hi) = (origin.as_vec2() * DX - 40.0, (origin + IVec2::splat(N as i32)).as_vec2() * DX + 40.0);
    let near = sim.fire.active_cells().iter().any(|c| {
        let p = w.centre_of(*c);
        let (x, z) = (p.x, -p.y);
        x >= lo.x && x <= hi.x && z >= lo.y && z <= hi.y
    });
    if !near {
        return;
    }
    let wind = crate::fire_view::smoke_wind(&sim);
    out.frame = Some(FluidNow {
        origin,
        wind: Vec2::new(wind.x, wind.y),
        dt: time.delta_secs().clamp(1.0 / 240.0, 1.0 / 30.0),
        time: time.elapsed_secs_wrapped(),
        front_rel: (ahead.clock - ahead.base) as f32,
        settings: settings.0,
    });
}

// --- render world -----------------------------------------------------------

#[derive(Clone, Copy, ShaderType, Default)]
struct Uniform {
    origin: IVec4,
    prev: IVec4,
    step: Vec4,
    wind: Vec4,
    look: Vec4,
    map: Vec4,
    flame: Vec4,
}

fn extract(mut commands: Commands, frame: Extract<Option<Res<FluidFrame>>>, sim: Extract<Option<Res<Sim>>>, volumes: Extract<Option<Res<crate::fire_volumetric::VolumeSettings>>>) {
    let (Some(frame), Some(sim)) = (frame.as_ref(), sim.as_ref()) else {
        commands.insert_resource(FluidFrame::default());
        return;
    };
    commands.insert_resource(**frame);
    let w = sim.scn.world;
    let height = volumes.as_ref().map_or(1.0, |v| v.flame_height);
    commands.insert_resource(World { map: Vec4::new(1.0 / w.width_m, 1.0 / w.height_m, w.cellsize, 0.0), height });
}

#[derive(Resource, Clone, Copy)]
struct World {
    map: Vec4,
    height: f32,
}

/// A 3D texture of the grid and its view.
fn grid(device: &RenderDevice, label: &'static str, format: TextureFormat, size: Extent3d) -> (Texture, TextureView) {
    let t = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let v = t.create_view(&TextureViewDescriptor::default());
    (t, v)
}

#[derive(Resource)]
pub struct FluidGpu {
    layout: BindGroupLayoutDescriptor,
    /// advect, curl, confine, divergence, jacobi, project
    pipelines: [CachedComputePipelineId; 6],
    state: [(Texture, TextureView); 2],
    curl: (Texture, TextureView),
    div: (Texture, TextureView),
    pressure: [(Texture, TextureView); 2],
    dummy_vec: (Texture, TextureView),
    dummy_scalar: (Texture, TextureView),
    pub sampler: Sampler,
    uniform: UniformBuffer<Uniform>,
    /// The state the last step left: the march reads it.
    latest: AtomicU32,
    /// The frame last stepped (`prepare` counts them), so two views step once.
    stepped: AtomicU32,
    frame: u32,
    prev: Option<IVec2>,
    /// Whether this frame steps.
    pub active: bool,
}

impl FluidGpu {
    /// The state the march reads: the last step's.
    pub fn latest(&self) -> &TextureView {
        &self.state[self.latest.load(Ordering::Relaxed) as usize].1
    }
}

fn init(mut commands: Commands, device: Res<RenderDevice>, pipeline_cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "fire_fluid_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<Uniform>(false),
                texture_3d(TextureSampleType::Float { filterable: false }),
                texture_3d(TextureSampleType::Float { filterable: false }),
                texture_storage_3d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                texture_storage_3d(TextureFormat::R32Float, StorageTextureAccess::WriteOnly),
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let pipelines = ["advect", "curl", "confine", "divergence", "jacobi", "project"].map(|entry| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("fire_fluid_{entry}").into()),
            layout: vec![layout.clone()],
            immediate_size: 0,
            shader: SHADER,
            shader_defs: vec![],
            entry_point: Some(entry.into()),
            zero_initialize_workgroup_memory: false,
            constants: vec![],
        })
    });
    let size = Extent3d { width: N, height: K, depth_or_array_layers: N };
    let one = Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("fire_fluid_sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(FluidGpu {
        layout,
        pipelines,
        state: [grid(&device, "fire_fluid_state_a", TextureFormat::Rgba16Float, size), grid(&device, "fire_fluid_state_b", TextureFormat::Rgba16Float, size)],
        curl: grid(&device, "fire_fluid_curl", TextureFormat::Rgba16Float, size),
        div: grid(&device, "fire_fluid_div", TextureFormat::R32Float, size),
        pressure: [grid(&device, "fire_fluid_p_a", TextureFormat::R32Float, size), grid(&device, "fire_fluid_p_b", TextureFormat::R32Float, size)],
        dummy_vec: grid(&device, "fire_fluid_dummy_vec", TextureFormat::Rgba16Float, one),
        dummy_scalar: grid(&device, "fire_fluid_dummy_scalar", TextureFormat::R32Float, one),
        sampler,
        uniform: UniformBuffer::default(),
        latest: AtomicU32::new(0),
        stepped: AtomicU32::new(u32::MAX),
        frame: 0,
        prev: None,
        active: false,
    });
}

fn prepare(frame: Res<FluidFrame>, world: Option<Res<World>>, mut gpu: ResMut<FluidGpu>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    gpu.frame = gpu.frame.wrapping_add(1);
    let (Some(now), Some(world)) = (frame.frame, world) else {
        gpu.active = false;
        // a gap: the grid starts anew where it is next stepped
        gpu.prev = None;
        return;
    };
    let s = now.settings;
    // nothing carried over a gap: last frame's grid as far away as can be
    let prev = gpu.prev.unwrap_or(now.origin + IVec2::splat(1 << 20));
    gpu.prev = Some(now.origin);
    gpu.active = true;
    gpu.uniform.set(Uniform {
        origin: IVec4::new(now.origin.x, now.origin.y, N as i32, K as i32),
        prev: IVec4::new(prev.x, prev.y, 0, 0),
        step: Vec4::new(DX, DZ, now.dt, now.time),
        wind: Vec4::new(now.wind.x, now.wind.y, s.buoyancy, s.hold),
        look: Vec4::new(s.swirl, s.drag, s.source_m, s.heat),
        map: world.map.truncate().extend(now.front_rel),
        flame: Vec4::new(crate::fire_view::FLAMING_S, world.height, s.tip, s.rise),
    });
    gpu.uniform.write_buffer(&device, &queue);
}

/// Steps the fluid once a frame (the first view to ask), before the march.
pub fn step(encoder: &mut CommandEncoder, device: &RenderDevice, gpu: &FluidGpu, pipeline_cache: &PipelineCache, images: &RenderAssets<GpuImage>) {
    if !gpu.active || gpu.stepped.swap(gpu.frame, Ordering::Relaxed) == gpu.frame {
        return;
    }
    let Some(ps) = gpu.pipelines.iter().map(|id| pipeline_cache.get_compute_pipeline(*id)).collect::<Option<Vec<_>>>() else { return };
    let (Some(front), Some(plume)) = (images.get(crate::fire_ahead::FRONT_MAP.id()), images.get(crate::fire_volumetric::PLUME_MAP.id())) else { return };
    let Some(uniform) = gpu.uniform.binding() else { return };
    let layout = pipeline_cache.get_bind_group_layout(&gpu.layout);
    let group = |a: &TextureView, b: &TextureView, out_vec: &TextureView, out_scalar: &TextureView| {
        device.create_bind_group(
            "fire_fluid_group",
            &layout,
            &BindGroupEntries::sequential((uniform.clone(), a, b, out_vec, out_scalar, &front.texture_view, &plume.texture_view)),
        )
    };
    let x = gpu.latest.load(Ordering::Relaxed) as usize;
    let y = 1 - x;
    let (sx, sy) = (&gpu.state[x].1, &gpu.state[y].1);
    let (c, d) = (&gpu.curl.1, &gpu.div.1);
    let (dv, ds) = (&gpu.dummy_vec.1, &gpu.dummy_scalar.1);
    let groups = [
        (0, group(sx, d, sy, ds)),
        (1, group(sy, d, c, ds)),
        (2, group(sy, c, sx, ds)),
        (3, group(sx, c, dv, d)),
    ];
    let wg = (N.div_ceil(4), K.div_ceil(4), N.div_ceil(4));
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor { label: Some("fire_fluid"), ..default() });
    for (p, g) in &groups {
        pass.set_pipeline(ps[*p]);
        pass.set_bind_group(0, g, &[]);
        pass.dispatch_workgroups(wg.0, wg.1, wg.2);
    }
    // pressure, warm-started from the last frame's
    let pa = &gpu.pressure[0].1;
    let pb = &gpu.pressure[1].1;
    let sweeps = [group(d, pa, dv, pb), group(d, pb, dv, pa)];
    pass.set_pipeline(ps[4]);
    for i in 0..JACOBI {
        pass.set_bind_group(0, &sweeps[i % 2], &[]);
        pass.dispatch_workgroups(wg.0, wg.1, wg.2);
    }
    // JACOBI even: the last sweep wrote `pa`
    let last = if JACOBI % 2 == 0 { pa } else { pb };
    let proj = group(sx, last, sy, ds);
    pass.set_pipeline(ps[5]);
    pass.set_bind_group(0, &proj, &[]);
    pass.dispatch_workgroups(wg.0, wg.1, wg.2);
    drop(pass);
    gpu.latest.store(y as u32, Ordering::Relaxed);
}

#[cfg(test)]
mod gpu_tests {
    use super::{Uniform, DX, DZ, JACOBI};
    use bevy::prelude::*;
    use bevy::render::render_resource::encase;

    const S: u32 = 32;

    fn half_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| half::f16::from_f32(*x).to_le_bytes()).collect()
    }

    /// The fluid's own passes, run as the game runs them, on a grid of
    /// 32 x 32 x 32 across a band alight 20 m wide (flames
    /// 10 m tall, wind 2 m/s across it), without a window; then the heat
    /// and the rise by height.
    /// Needs a GPU: skipped where there is none.
    #[test]
    #[ignore = "needs a GPU; cargo test --release -p game fluid_rises -- --ignored --nocapture"]
    fn fluid_rises() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU");
            return;
        };
        let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fire_fluid.wesl").into()),
        });
        use wgpu::{BindingType as B, ShaderStages as St, TextureFormat as F};
        let tex3 = B::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false };
        let tex2 = B::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let store = |format| B::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D3 };
        let types = [B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, tex3, tex3, store(F::Rgba16Float), store(F::R32Float), tex2, tex2];
        let entries: Vec<_> = types.iter().enumerate().map(|(i, ty)| wgpu::BindGroupLayoutEntry { binding: i as u32, visibility: St::COMPUTE, ty: *ty, count: None }).collect();
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipes: Vec<_> = ["advect", "curl", "confine", "divergence", "jacobi", "project"]
            .iter()
            .map(|e| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(e),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(e),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        let grid = |format, size: u32, k: u32, extra| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: size, height: k, depth_or_array_layers: size },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | extra,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            (t, v)
        };
        let src = wgpu::TextureUsages::COPY_SRC;
        let state = [grid(F::Rgba16Float, S, S, src), grid(F::Rgba16Float, S, S, src)];
        let curl = grid(F::Rgba16Float, S, S, src);
        let div = grid(F::R32Float, S, S, src);
        let pres = [grid(F::R32Float, S, S, src), grid(F::R32Float, S, S, src)];
        let dv = grid(F::Rgba16Float, 1, 1, src);
        let ds = grid(F::R32Float, 1, 1, src);
        // a map of 40 x 1 texels over a world 400 m wide: `texel(column)`
        let map2 = |texel: &dyn Fn(u32) -> [f32; 4]| {
            let data: Vec<f32> = (0..40).flat_map(texel).collect();
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: 40, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: F::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                t.as_image_copy(),
                &half_bytes(&data),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * 40), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 40, height: 1, depth_or_array_layers: 1 },
            );
            t.create_view(&Default::default())
        };
        // a band 20 m wide across the grid (x 340-360 m), alight 30 s ago;
        // flames 10 m tall
        let band = |c: u32| c == 34 || c == 35;
        let front = map2(&|c| if band(c) { [-30.0, 5.0, 0.0, 1.0] } else { [30_000.0, 0.0, 0.0, 0.0] });
        let plume = map2(&|c| if band(c) { [0.5, 1.0, 10.0, 0.0] } else { [0.0; 4] });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let s = super::FluidSettings::from_env();
        // the grid centred on the band (x 340-360 m)
        let origin = IVec2::new((350.0 / DX) as i32 - S as i32 / 2, 100);
        let dt = 1.0 / 30.0;
        let mut latest = 0usize;
        let steps = 120;
        for step in 0..steps {
            let prev = if step == 0 { origin + IVec2::splat(1 << 20) } else { origin };
            let u = Uniform {
                origin: IVec4::new(origin.x, origin.y, S as i32, S as i32),
                prev: IVec4::new(prev.x, prev.y, 0, 0),
                step: Vec4::new(DX, DZ, dt, step as f32 * dt),
                wind: Vec4::new(2.0, 0.0, s.buoyancy, s.hold),
                look: Vec4::new(s.swirl, s.drag, s.source_m, s.heat),
                map: Vec4::new(1.0 / 400.0, 1.0 / 400.0, 20.0, 0.0),
                flame: Vec4::new(crate::fire_view::FLAMING_S, 1.0, s.tip, s.rise),
            };
            let mut bytes = encase::UniformBuffer::new(Vec::<u8>::new());
            bytes.write(&u).unwrap();
            queue.write_buffer(&ubuf, 0, bytes.as_ref());
            let group = |a: &wgpu::TextureView, b: &wgpu::TextureView, ov: &wgpu::TextureView, os: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(a) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(b) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(ov) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(os) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&front) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&plume) },
                    ],
                })
            };
            let (x, y) = (latest, 1 - latest);
            let (sx, sy) = (&state[x].1, &state[y].1);
            let (c, d) = (&curl.1, &div.1);
            let gs = [group(sx, d, sy, &ds.1), group(sy, d, c, &ds.1), group(sy, c, sx, &ds.1), group(sx, c, &dv.1, d)];
            let sweeps = [group(d, &pres[0].1, &dv.1, &pres[1].1), group(d, &pres[1].1, &dv.1, &pres[0].1)];
            let last = if JACOBI % 2 == 0 { &pres[0].1 } else { &pres[1].1 };
            let proj = group(sx, last, sy, &ds.1);
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                let wg = S / 4;
                for (i, g) in gs.iter().enumerate() {
                    pass.set_pipeline(&pipes[i]);
                    pass.set_bind_group(0, g, &[]);
                    pass.dispatch_workgroups(wg, wg, wg);
                }
                pass.set_pipeline(&pipes[4]);
                for i in 0..JACOBI {
                    pass.set_bind_group(0, &sweeps[i % 2], &[]);
                    pass.dispatch_workgroups(wg, wg, wg);
                }
                pass.set_pipeline(&pipes[5]);
                pass.set_bind_group(0, &proj, &[]);
                pass.dispatch_workgroups(wg, wg, wg);
            }
            queue.submit([enc.finish()]);
            latest = y;
        }
        // read the state back
        let row = S * 8;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * S * S) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            state[latest].0.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &out, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(S) } },
            wgpu::Extent3d { width: S, height: S, depth_or_array_layers: S },
        );
        queue.submit([enc.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = out.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, k: u32, z: u32, c: usize| {
            let i = ((z * S + k) * S + x) as usize * 8 + c * 2;
            half::f16::from_le_bytes([data[i], data[i + 1]]).to_f32()
        };
        println!(" quota  calore medio  calore max  salita media  salita max");
        let mut top_heat = 0.0f32;
        for k in 0..S {
            let (mut sum, mut max, mut vs, mut vmax) = (0.0f32, 0.0f32, 0.0f32, f32::MIN);
            for z in 0..S {
                for x in 0..S {
                    let h = at(x, k, z, 3);
                    let v = at(x, k, z, 1);
                    sum += h;
                    max = max.max(h);
                    vs += v;
                    vmax = vmax.max(v);
                }
            }
            let n = (S * S) as f32;
            if k * DZ as u32 >= 8 {
                top_heat = top_heat.max(max);
            }
            println!("{:>5.1} m {:>11.3} {:>11.3} {:>11.2} {:>11.2}", k as f32 * DZ, sum / n, max, vs / n, vmax);
        }
        assert!(top_heat > 0.3, "no flame reaches 8 m");
    }
}

#[cfg(test)]
mod tests {
    /// The shader is valid WGSL (naga, as wgpu checks it on the desktop).
    #[test]
    fn the_shader_validates() {
        let src = include_str!("shaders/fire_fluid.wesl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        for entry in ["advect", "curl", "confine", "divergence", "jacobi", "project"] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
    }
}
