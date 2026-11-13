//! The zone where a team can be placed, painted by a shader: see
//! `shaders/reach_zone.wgsl`. A grid draped over the band around the reachable
//! road segments carries each vertex's distance to the nearest one; the shader
//! cuts the band at the half width, so the zone is a single surface.

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use scenario::{Pos, Scenario};

const REACH_SHADER_HANDLE: Handle<Shader> = uuid_handle!("3b9e7a10-c4d2-4f61-a58e-92d01e7b6c33");

/// Spacing of the draped grid, metres.
const GRID_M: f32 = 20.0;
const LIFT_M: f32 = crate::rings::RING_LIFT_M;
const PULL: Vec4 = Vec4::new(0.04, 40.0, 220.0, 0.0);

#[derive(Clone, Copy, ShaderType)]
pub struct ReachUniform {
    color: Vec4,
    band: Vec4,
    pull: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ReachMaterial {
    #[uniform(0)]
    pub uniform: ReachUniform,
}

impl ReachMaterial {
    /// `half_m`: half the width of the band, metres.
    pub fn new(rgb: [f32; 3], alpha: f32, half_m: f32) -> Self {
        ReachMaterial { uniform: ReachUniform { color: Vec4::new(rgb[0], rgb[1], rgb[2], alpha), band: Vec4::new(half_m, 1.0, 0.0, 0.0), pull: PULL } }
    }
}

impl Material for ReachMaterial {
    // no depth prepass and no shadows (was `MaterialPlugin { prepass_enabled, shadows_enabled }` before Bevy 0.17)
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn vertex_shader() -> ShaderRef {
        REACH_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> ShaderRef {
        REACH_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        -2.0
    }
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct ReachPlugin;

impl Plugin for ReachPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, REACH_SHADER_HANDLE, "shaders/reach_zone.wesl");
        app.add_plugins(MaterialPlugin::<ReachMaterial>::default());
    }
}

fn segment_distance(a: Pos, b: Pos, p: Pos) -> f32 {
    let (ex, ey) = (b.x - a.x, b.y - a.y);
    let t = (((p.x - a.x) * ex + (p.y - a.y) * ey) / (ex * ex + ey * ey).max(1e-6)).clamp(0.0, 1.0);
    let (dx, dy) = (a.x + t * ex - p.x, a.y + t * ey - p.y);
    (dx * dx + dy * dy).sqrt()
}

/// The grid over the `segments`' surroundings, draped on the terrain, with
/// each vertex's distance to the nearest segment in the first texture
/// coordinate's x, held within `reach_m`. `None` if there is nothing to draw.
pub fn band_mesh(scn: &Scenario, segments: &[[Pos; 2]], reach_m: f32) -> Option<Mesh> {
    let reach = reach_m;
    let (w, h) = (scn.world.width_m, scn.world.height_m);
    let (nx, ny) = ((w / GRID_M).ceil() as usize, (h / GRID_M).ceil() as usize);
    let stride = nx + 1;
    let mut dist = vec![reach; stride * (ny + 1)];
    let mut touched = vec![false; dist.len()];
    for s in segments {
        let lo = Pos { x: s[0].x.min(s[1].x) - reach, y: s[0].y.min(s[1].y) - reach };
        let hi = Pos { x: s[0].x.max(s[1].x) + reach, y: s[0].y.max(s[1].y) + reach };
        let (i0, i1) = ((lo.x / GRID_M).floor().max(0.0) as usize, ((hi.x / GRID_M).ceil() as usize).min(nx));
        let (j0, j1) = ((lo.y / GRID_M).floor().max(0.0) as usize, ((hi.y / GRID_M).ceil() as usize).min(ny));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = Pos { x: (i as f32 * GRID_M).min(w), y: (j as f32 * GRID_M).min(h) };
                let d = segment_distance(s[0], s[1], p);
                let k = j * stride + i;
                if d < dist[k] {
                    dist[k] = d;
                    touched[k] = true;
                }
            }
        }
    }
    let mut positions = Vec::with_capacity(dist.len());
    for j in 0..=ny {
        for i in 0..=nx {
            let p = Pos { x: (i as f32 * GRID_M).min(w), y: (j as f32 * GRID_M).min(h) };
            positions.push([p.x, scn.terrain.height_at(p) + LIFT_M, -p.y]);
        }
    }
    // only the cells a road's surroundings touch go into the mesh
    let mut indices = vec![];
    for j in 0..ny {
        for i in 0..nx {
            let at = |i: usize, j: usize| j * stride + i;
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            if touched[a] || touched[b] || touched[c] || touched[d] {
                indices.extend([a, c, b, a, d, c].map(|v| v as u32));
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    let uvs: Vec<[f32; 2]> = dist.iter().map(|&d| [d, 0.0]).collect();
    // drawn only (`kiosk::marks`): for the card alone, not kept nor copied for
    // the renderer (the road network's mark is 7 MB)
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; positions.len()]);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_indices(Indices::U32(indices));
    Some(m)
}

// ---- the route of a move ---------------------------------------------------

const ROUTE_SHADER_HANDLE: Handle<Shader> = uuid_handle!("5c1490ab-7e3d-41f8-b2a6-0d9e44c1f7a2");
/// Spacing of the sparse grid along a route, metres.
const ROUTE_GRID_M: f32 = 10.0;

#[derive(Clone, Copy, ShaderType)]
pub struct RouteUniform {
    color: Vec4,
    line: Vec4,
    pull: Vec4,
    look: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct RouteMaterial {
    #[uniform(0)]
    pub uniform: RouteUniform,
}

impl RouteMaterial {
    /// `core_m`, `outline_m`: half widths, metres.
    pub fn new(rgb: [f32; 3], core_m: f32, outline_m: f32) -> Self {
        RouteMaterial {
            uniform: RouteUniform {
                color: Vec4::new(rgb[0], rgb[1], rgb[2], 1.0),
                line: Vec4::new(core_m, outline_m, 60.0, 90.0),
                pull: Vec4::new(0.04, 40.0, 220.0, 0.0),
                look: Vec4::new(1.0, 1.0, 0.0, 0.0),
            },
        }
    }

    /// A plain line (no flow of light) at `opacity`.
    pub fn plain(rgb: [f32; 3], core_m: f32, outline_m: f32, opacity: f32) -> Self {
        let mut m = Self::new(rgb, core_m, outline_m);
        m.uniform.look = Vec4::new(opacity, 0.0, 0.0, 0.0);
        m
    }
}

impl RouteMaterial {
    /// A plain line broken into dashes `period_m` long that march along it.
    pub fn dashed(rgb: [f32; 3], core_m: f32, outline_m: f32, opacity: f32, period_m: f32) -> Self {
        let mut m = Self::plain(rgb, core_m, outline_m, opacity);
        m.uniform.look.z = period_m;
        m
    }
}

impl Material for RouteMaterial {
    // no depth prepass and no shadows (was `MaterialPlugin { prepass_enabled, shadows_enabled }` before Bevy 0.17)
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn vertex_shader() -> ShaderRef {
        ROUTE_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> ShaderRef {
        ROUTE_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        -2.0
    }
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct RoutePlugin;

impl Plugin for RoutePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, ROUTE_SHADER_HANDLE, "shaders/route_line.wesl");
        app.add_plugins(MaterialPlugin::<RouteMaterial>::default());
    }
}

/// A sparse grid draped along `pts`, `reach_m` either side, each vertex with
/// its distance to the path and the distance along it (the first texture
/// coordinates). `None` for a path too short to draw.
pub fn route_mesh(scn: &Scenario, pts: &[Pos], reach_m: f32) -> Option<Mesh> {
    let mut along = 0.0f32;
    let segs: Vec<([Pos; 2], f32)> = pts.windows(2).map(|s| {
        let at = along;
        along += ((s[1].x - s[0].x).powi(2) + (s[1].y - s[0].y).powi(2)).sqrt();
        ([s[0], s[1]], at)
    }).collect();
    sparse_mesh(scn, &segs, reach_m, ROUTE_GRID_M)
}

/// The same for loose `segments` (each with where it starts along its path),
/// on a grid of `grid_m`.
pub fn sparse_mesh(scn: &Scenario, segments: &[([Pos; 2], f32)], reach_m: f32, grid_m: f32) -> Option<Mesh> {
    use std::collections::HashMap;
    let (w, h) = (scn.world.width_m, scn.world.height_m);
    let (nx, ny) = ((w / grid_m).ceil() as i64, (h / grid_m).ceil() as i64);
    let mut best: HashMap<(i64, i64), (f32, f32)> = HashMap::new();
    for (s, along) in segments.iter().map(|(s, a)| (s, *a)) {
        let (a, b) = (s[0], s[1]);
        let (ex, ey) = (b.x - a.x, b.y - a.y);
        let len2 = (ex * ex + ey * ey).max(1e-6);
        let len = len2.sqrt();
        let (i0, i1) = (((a.x.min(b.x) - reach_m) / grid_m).floor().max(0.0) as i64, (((a.x.max(b.x) + reach_m) / grid_m).ceil() as i64).min(nx));
        let (j0, j1) = (((a.y.min(b.y) - reach_m) / grid_m).floor().max(0.0) as i64, (((a.y.max(b.y) + reach_m) / grid_m).ceil() as i64).min(ny));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = Pos { x: (i as f32 * grid_m).min(w), y: (j as f32 * grid_m).min(h) };
                let t = (((p.x - a.x) * ex + (p.y - a.y) * ey) / len2).clamp(0.0, 1.0);
                let (dx, dy) = (a.x + t * ex - p.x, a.y + t * ey - p.y);
                let d = (dx * dx + dy * dy).sqrt();
                let e = best.entry((i, j)).or_insert((reach_m, 0.0));
                if d < e.0 {
                    *e = (d, along + t * len);
                }
            }
        }
    }
    if best.is_empty() {
        return None;
    }
    let mut index: HashMap<(i64, i64), u32> = HashMap::new();
    let (mut positions, mut uvs) = (vec![], vec![]);
    let mut vertex = |i: i64, j: i64, best: &HashMap<(i64, i64), (f32, f32)>| -> u32 {
        *index.entry((i, j)).or_insert_with(|| {
            let p = Pos { x: (i as f32 * grid_m).min(w), y: (j as f32 * grid_m).min(h) };
            let (d, s) = best.get(&(i, j)).copied().unwrap_or((reach_m, 0.0));
            positions.push([p.x, scn.terrain.height_at(p) + LIFT_M, -p.y]);
            uvs.push([d, s]);
            positions.len() as u32 - 1
        })
    };
    let mut indices = vec![];
    let cells: std::collections::HashSet<(i64, i64)> = best.keys().flat_map(|&(i, j)| [(i, j), (i - 1, j), (i, j - 1), (i - 1, j - 1)]).collect();
    for (i, j) in cells {
        if i < 0 || j < 0 || i >= nx || j >= ny {
            continue;
        }
        let (a, b, c, d) = (vertex(i, j, &best), vertex(i + 1, j, &best), vertex(i + 1, j + 1, &best), vertex(i, j + 1, &best));
        indices.extend([a, c, b, a, d, c]);
    }
    // drawn only (`kiosk::marks`): for the card alone, not kept nor copied for
    // the renderer (the road network's mark is 7 MB)
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; positions.len()]);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_indices(Indices::U32(indices));
    Some(m)
}
