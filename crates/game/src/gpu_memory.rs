//! What the scene holds on the GPU, printed to the log (on the web, the
//! browser console) a few seconds after start and then every minute.
//!
//! A diagnosis (2026-11-01, an iPhone 13 whose page dies): iOS charged the
//! page 1241 MB, about 780 MB of it outside its own memory, which is where the
//! WebGL textures and buffers are counted. Listed here: every texture made from
//! an `Image` (size, format, mip levels), the slabs of Bevy's mesh allocator
//! (allocated and used) and, from wgpu's own counters, how many buffers and
//! textures exist in all, render targets and the game's own buffers included.
//! Every 600 frames, the assets added or changed per frame by type (meshes by
//! size): on WebGL2 each is a new GPU buffer or a write into a shared one, the
//! churn found on 2026-11-01 (`streamed_mesh`, `web_gpu`). Every 3600, what
//! the main world still keeps of meshes and images (wasm memory on the web).
//! wgpu's `hal.buffers` falls on WebGL2 by one per `write_buffer`: its staging
//! copies are vectors made without being counted, not GL buffers.
//! Off unless `KIOSK_GPU_MEMORY=1` (desktop) or `?gpu=1` (web).

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};

pub struct GpuMemoryPlugin;

impl Plugin for GpuMemoryPlugin {
    fn build(&self, app: &mut App) {
        if !wanted() {
            return;
        }
        app.init_resource::<Churn>().add_systems(
            Last,
            (
                churn_meshes,
                churn::<Image>,
                churn::<StandardMaterial>,
                churn::<crate::materials::TerrainMaterial>,
                churn::<crate::materials::StructureMaterial>,
                churn::<crate::flat::FlatMaterial>,
                churn::<crate::sea::WaterMaterial>,
                churn::<crate::tape::TapeMaterial>,
                churn::<crate::reach_zone::ReachMaterial>,
                churn::<crate::reach_zone::RouteMaterial>,
                churn::<crate::fire_shader::FireMaterial>,
                churn::<crate::fire_shader::SmokeMaterial>,
                churn::<crate::fire_shader::FireGroundMaterial>,
                churn::<crate::forecast_zone::ZoneMaterial>,
                churn::<crate::sky::SkyMaterial>,
                churn::<crate::near_cover::NearCoverMaterial>,
            ),
        );
        app.add_systems(Last, report_churn.after(churn_meshes));
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render.init_resource::<Clock>().add_systems(Render, report.in_set(RenderSystems::Cleanup));
    }
}

/// Assets added or changed in the main world, by type (and meshes by size):
/// each one is prepared again for the GPU, a new buffer or texture on WebGL2.
#[derive(Resource, Default)]
struct Churn {
    frames: u32,
    counts: HashMap<String, [u32; 3]>,
}

fn kind<A: Asset>(e: &AssetEvent<A>) -> Option<usize> {
    match e {
        AssetEvent::Added { .. } => Some(0),
        AssetEvent::Modified { .. } => Some(1),
        AssetEvent::Removed { .. } => Some(2),
        _ => None,
    }
}

fn churn<A: Asset>(mut events: MessageReader<AssetEvent<A>>, mut churn: ResMut<Churn>) {
    let name = std::any::type_name::<A>().rsplit("::").next().unwrap_or("?").to_string();
    for e in events.read() {
        if let Some(k) = kind(e) {
            churn.counts.entry(name.clone()).or_default()[k] += 1;
        }
    }
}

fn churn_meshes(mut events: MessageReader<AssetEvent<Mesh>>, meshes: Res<Assets<Mesh>>, mut churn: ResMut<Churn>) {
    for e in events.read() {
        let Some(k) = kind(e) else { continue };
        let name = match e {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => meshes
                .get(*id)
                .and_then(|m| m.try_attribute(Mesh::ATTRIBUTE_POSITION).ok().map(|p| (p.len(), m)))
                .map(|(n, m)| format!("Mesh {n} v {} attrs", m.try_attributes().map(|a| a.count()).unwrap_or(0)))
                .unwrap_or_else(|| "Mesh ?".into()),
            _ => "Mesh".into(),
        };
        churn.counts.entry(name).or_default()[k] += 1;
    }
}

fn report_churn(mut churn: ResMut<Churn>, meshes: Res<Assets<Mesh>>, images: Res<Assets<Image>>) {
    churn.frames += 1;
    if churn.frames % 600 != 0 {
        return;
    }
    // what the main world (the wasm memory) still holds of what went to the GPU
    if churn.frames % 3600 == 0 {
        let (mut n, mut bytes) = (0, 0);
        for (_, m) in meshes.iter() {
            let Ok(attrs) = m.try_attributes() else { continue };
            n += 1;
            bytes += attrs.map(|(_, v)| v.get_bytes().len()).sum::<usize>();
            bytes += m.try_indices().map(|i| i.len() * 4).unwrap_or(0);
        }
        let (mut ni, mut ibytes) = (0, 0);
        for (_, i) in images.iter() {
            if let Some(d) = &i.data {
                ni += 1;
                ibytes += d.len();
            }
        }
        info!(
            "cpu copies: {n} of {} meshes {:.1} MiB, {ni} of {} images {:.1} MiB",
            meshes.len(), mib(bytes as u64), images.len(), mib(ibytes as u64)
        );
    }
    let mut rows: Vec<_> = churn.counts.drain().collect();
    rows.sort_by(|a, b| (b.1[0] + b.1[1]).cmp(&(a.1[0] + a.1[1])));
    let line: Vec<String> = rows
        .iter()
        .take(16)
        .map(|(n, c)| format!("{n}: +{:.2} ~{:.2} -{:.2}", c[0] as f32 / 600.0, c[1] as f32 / 600.0, c[2] as f32 / 600.0))
        .collect();
    info!("asset churn per frame (added ~modified -removed): {}", line.join("; "));
}

fn wanted() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"location".into())
            .and_then(|l| js_sys::Reflect::get(&l, &"search".into()))
            .ok()
            .and_then(|s| s.as_string())
            .is_some_and(|q| q.contains("gpu=1"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("KIOSK_GPU_MEMORY").is_ok_and(|v| v == "1")
    }
}

#[derive(Resource, Default)]
struct Clock {
    frames: u32,
}

const FIRST_FRAME: u32 = 300;
const EVERY: u32 = 3600;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / 1048576.0
}

/// Bytes of a texture with all its mip levels, layers and samples.
fn texture_bytes(size: bevy::render::render_resource::Extent3d, format: TextureFormat, mips: u32, samples: u32, is_3d: bool) -> u64 {
    let (bw, bh) = format.block_dimensions();
    // depth formats have no copy size: count them at 4 bytes a texel
    let block = format.block_copy_size(None).unwrap_or(4) as u64;
    let mut total = 0;
    for level in 0..mips.max(1) {
        let w = (size.width >> level).max(1).div_ceil(bw) as u64;
        let h = (size.height >> level).max(1).div_ceil(bh) as u64;
        let d = if is_3d { (size.depth_or_array_layers >> level).max(1) } else { size.depth_or_array_layers } as u64;
        total += w * h * d * block;
    }
    total * samples.max(1) as u64
}

fn report(
    mut clock: ResMut<Clock>,
    images: Res<RenderAssets<GpuImage>>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    device: Res<RenderDevice>,
) {
    clock.frames += 1;
    if clock.frames < FIRST_FRAME || (clock.frames - FIRST_FRAME) % EVERY != 0 {
        return;
    }
    let mut rows: Vec<(u64, String)> = images
        .iter()
        .map(|(_, image)| {
            let d = &image.texture_descriptor;
            let bytes = texture_bytes(d.size, d.format, d.mip_level_count, d.sample_count, d.dimension == bevy::render::render_resource::TextureDimension::D3);
            let name = format!(
                "{}x{}x{} {:?} mips {}{}",
                d.size.width, d.size.height, d.size.depth_or_array_layers, d.format, d.mip_level_count,
                d.label.map(|l| format!(" «{l}»")).unwrap_or_default()
            );
            (bytes, name)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let image_total: u64 = rows.iter().map(|r| r.0).sum();

    // Each slab is one buffer shared by several meshes: count it once.
    let mut slabs: HashMap<String, (u64, u64)> = HashMap::new();
    let mut heavy: Vec<(u64, String)> = Vec::new();
    for (id, mesh) in meshes.iter() {
        let index_count = match &mesh.buffer_info {
            bevy::render::mesh::RenderMeshBufferInfo::Indexed { count, .. } => *count as u64,
            _ => 0,
        };
        let stride0 = mesh.layout.0.layout().array_stride;
        heavy.push((
            mesh.vertex_count as u64 * stride0 + index_count * 4,
            format!("{} vertices × {} B, {} indices", mesh.vertex_count, stride0, index_count),
        ));
        // the slices count elements: vertices of the mesh's stride, indices of 4 bytes at most
        let stride = mesh.layout.0.layout().array_stride;
        for (slice, size) in [(allocator.mesh_vertex_slice(&id), stride), (allocator.mesh_index_slice(&id), 4)] {
            let Some(slice) = slice else { continue };
            let key = format!("{:?}", slice.buffer.id());
            let used = (slice.range.end - slice.range.start) as u64 * size;
            let entry = slabs.entry(key).or_insert((slice.buffer.size(), 0));
            entry.1 += used;
        }
    }
    let (slab_size, slab_used) = slabs.values().fold((0, 0), |a, s| (a.0 + s.0, a.1 + s.1));

    let counters = device.wgpu_device().get_internal_counters();
    info!(
        "gpu memory: {} textures from images {:.1} MiB; {} mesh slabs {:.1} MiB ({:.1} MiB used) for {} meshes; wgpu alive: {} buffers, {} textures",
        rows.len(), mib(image_total), slabs.len(), mib(slab_size), mib(slab_used), meshes.iter().count(),
        counters.hal.buffers.read(), counters.hal.textures.read(),
    );
    heavy.sort_by(|a, b| b.0.cmp(&a.0));
    let same: HashMap<&str, (u32, u64)> = heavy.iter().fold(HashMap::new(), |mut m, (b, n)| {
        let e = m.entry(n.as_str()).or_insert((0, 0));
        e.0 += 1;
        e.1 += b;
        m
    });
    let mut groups: Vec<_> = same.into_iter().collect();
    groups.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (name, (n, bytes)) in groups.iter().take(12) {
        info!("gpu memory:   {:7.1} MiB  {n} × mesh {name}", mib(*bytes));
    }
    for (bytes, name) in rows.iter().take(15) {
        info!("gpu memory:   {:7.1} MiB  {name}", mib(*bytes));
    }
}
