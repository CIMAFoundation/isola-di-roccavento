//! The sea: a separate animated mesh over the water, rather than the flat
//! vertex-coloured patch [`crate::terrain_mesh`] paints under the waterline.
//!
//! The render DEM is clamped at 0 m (`data/spotorno_render_terrain.json`:
//! `elev_min: 0.0`) — there is no real bathymetry in this dataset, only a
//! flat sea floor. So "depth" here is not measured depth: it is distance to
//! the nearest dry ground, probed outward from each water vertex at mesh-build
//! time ([`distance_to_shore`]). That is what actually varies near a real
//! coastline — a Ligurian shelf is narrow — and gives the shallow/deep colour
//! split and the shoreline foam something honest to key off, without
//! pretending to know a seabed this data does not have.
//!
//! Waves are a vertex-shader displacement — see `shaders/water.wgsl` — driven
//! by the same [`fire::Weather`] wind the smoke and embers use, so a
//! wind shift from the Wildfire panel chops the sea up exactly when it
//! shifts the fire.

use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::shader::ShaderRef;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use scenario::{Pos, Scenario};

use crate::sim::Sim;
use crate::sky::SunState;

const WATER_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("2c6e88aa-401d-47cc-8f19-55dd66ee77ff");

/// Lattice spacing for the water mesh: coarser than the 5 m terrain posting,
/// since the wave shapes it carries (tens of metres) do not need it and the
/// fine chop is a fragment-shader noise wobble, not geometry (see the wgsl).
const WATER_STRIDE: usize = if crate::quality::WEBGL2 { 12 } else { 3 };
const WATER_CHUNK: usize = 64;

/// Must agree with `terrain_mesh::ground_color`'s own sea threshold: this is
/// the elevation below which the ground layer paints "sea", so the water
/// mesh has to cover exactly that footprint or a ring of one or the other
/// colour would show at the coast.
pub(crate) const WATER_ELEV_MAX: f32 = 0.6;

/// Flat base height for the (unwaved) lattice; the vertex shader adds the
/// swell on top of this every frame. Has to clear both `WATER_ELEV_MAX`
/// (ground under the water can sit up to 0.6 m) *and* the deepest wave
/// trough the shader can produce (see the amplitude clamp in
/// `shaders/water.wgsl`) — undershoot either and the terrain pokes through
/// the animated surface, which reads as a patch of dark, static "shadow"
/// sitting in the water. Shipped once at 0.35 m, which cleared neither.
const WATER_BASE_Y: f32 = 0.8;

/// How far past the window edge the sea keeps its full [`WATER_STRIDE`]
/// resolution, and where the open sea ends. The island sits well inside the
/// window, so everything outside the window is sea: one surface, one shader,
/// so no seam can show at the horizon.
const SEA_EXTEND_M: f32 = 700.0;
const OPEN_SEA_STRIDE_M: f32 = if crate::quality::WEBGL2 { 600.0 } else { 150.0 };
/// Half-extent of the open sea's lattice and the radius it is trimmed to.
const SEA_OUT_M: f32 = 32_000.0;
const OPEN_SEA_RADIUS_M: f32 = 30_000.0;

/// One rectangular water lattice: origin, cell counts and (possibly
/// non-round) cell size. The open-sea bands derive their spacing from their
/// own span rather than taking [`OPEN_SEA_STRIDE_M`] literally, so each band
/// ends *exactly* on the inner mesh's boundary — a lattice that merely came
/// close would leave either a strip of missing sea or a strip of two water
/// surfaces z-fighting all along the join.
struct Patch {
    x0: f32,
    y0: f32,
    cols: usize,
    rows: usize,
    sx: f32,
    sy: f32,
    /// Whether vertices carry a real [`distance_to_shore`]. Only the near
    /// patch does: the probe is 48 terrain samples a vertex, and what it
    /// buys — the shallow-water colour and the shoreline foam — is a
    /// metres-wide band nothing ever sees from 3 km out.
    probe_shore: bool,
}

impl Patch {
    /// A patch spanning `[x0, x1] x [y0, y1]` at close to `target` spacing.
    fn spanning(x0: f32, x1: f32, y0: f32, y1: f32, target: f32, probe_shore: bool) -> Patch {
        let nx = ((x1 - x0) / target).ceil().max(1.0);
        let ny = ((y1 - y0) / target).ceil().max(1.0);
        Patch {
            x0,
            y0,
            cols: nx as usize + 1,
            rows: ny as usize + 1,
            sx: (x1 - x0) / nx,
            sy: (y1 - y0) / ny,
            probe_shore,
        }
    }
}

/// Elevation for a point that may be outside the real terrain grid: the real
/// bilinear field inside the window, `far_terrain`'s continuation beyond it
/// — so the "is this wet" test agrees with the ground the extension actually
/// sits on instead of just clamping to the window edge's own elevation.
fn elev_at(scn: &Scenario, p: Pos) -> f32 {
    if scn.world.contains(p) {
        scn.terrain.height_at(p)
    } else {
        // outside the window there is only sea
        -1.0
    }
}

/// Whether `p` is open water: the sea mesh's own wet test, for anything that
/// floats.
pub fn is_water(scn: &Scenario, p: Pos) -> bool {
    elev_at(scn, p) <= WATER_ELEV_MAX
}

/// Distance to the nearest dry ground probed outward, metres, saturating at
/// the last probe ring (160 m).
pub fn shore_distance(scn: &Scenario, p: Pos) -> f32 {
    distance_to_shore(scn, p)
}

/// The wave height at a Bevy-space XZ and time: the water shader's vertex
/// displacement, so a floating thing rides the same swell. `drift` is the
/// downwind unit direction in Bevy XZ and `speed` the wind in m/s.
pub fn wave_height(x: f32, z: f32, t: f32, drift: Vec2, speed: f32) -> f32 {
    // the near sea's lattice spacing: the water shader fades each wave by it
    let stride = 5.0 * WATER_STRIDE as f32;
    let fade = |wavelength: f32| {
        let k = ((wavelength - 2.0 * stride) / (2.0 * stride)).clamp(0.0, 1.0);
        k * k * (3.0 - 2.0 * k)
    };
    let dir0 = drift;
    let perp = Vec2::new(-dir0.y, dir0.x);
    let dir1 = (dir0 * 0.6 + perp * 0.5).normalize_or_zero();
    let p = Vec2::new(x, z);
    let amp = (speed * 0.045).clamp(0.03, 0.5);
    let s = 0.7 + speed * 0.05;
    fade(114.0) * amp * (p.dot(dir0) * 0.055 + t * s).sin()
        + fade(57.0) * amp * 0.5 * (p.dot(dir1) * 0.11 + t * s * 1.5 + 1.7).sin()
        + fade(30.0) * amp * 0.25 * (p.dot(dir0) * 0.21 - t * s * 2.1 + 4.1).sin()
}

/// The wind as the water sees it: downwind unit direction in Bevy XZ, and m/s.
pub fn wind_drift(sim: &Sim) -> (Vec2, f32) {
    let weather = sim.fire.weather();
    let from = (weather.wind_dir_deg as f32).to_radians();
    (Vec2::new(-from.sin(), from.cos()), weather.wind_speed_kmh as f32 / 3.6)
}

/// Whether a lattice vertex (in Bevy space, as pushed into the mesh) is
/// within [`OPEN_SEA_RADIUS_M`] of the window's centre. Cells outside are
/// dropped, which rounds the sea's outer boundary off into a disc.
fn inside_horizon(scn: &Scenario, p: [f32; 3]) -> bool {
    let dx = p[0] - scn.terrain.width_m * 0.5;
    let dz = -p[2] - scn.terrain.height_m * 0.5;
    dx * dx + dz * dz <= OPEN_SEA_RADIUS_M * OPEN_SEA_RADIUS_M
}

/// Probe ring radii for [`distance_to_shore`], metres. Past the last ring, a
/// water point is "open water" for shading purposes — the shallow/deep split
/// only needs to resolve the first hundred-odd metres off a beach.
const SHORE_PROBE_RADII: [f32; 6] = [10.0, 20.0, 40.0, 70.0, 110.0, 160.0];
const SHORE_PROBE_DIRS: usize = 8;

#[derive(Component)]
struct SeaMesh;

#[derive(Resource)]
struct WaterHandle(Handle<WaterMaterial>);

#[derive(Clone, Copy, Default, ShaderType)]
pub struct WaterUniform {
    pub wind: Vec4,
    pub sun_dir: Vec4,
    pub sun_color: Vec4,
    pub deep_color: Vec4,
    pub shallow_color: Vec4,
    /// World metres to ground-mask coordinates (`ground_mask::RoadMask::rect`).
    pub mask_rect: Vec4,
    /// x: tactical desaturation (`tactical.rs`).
    pub tactical: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WaterMaterial {
    #[uniform(0)]
    pub uniform: WaterUniform,
    /// The ground mask, whose alpha is the signed distance to the waterline.
    #[texture(1)]
    #[sampler(2)]
    pub shore: Option<Handle<Image>>,
}

impl Material for WaterMaterial {
    // no depth prepass and no shadows (was `MaterialPlugin { prepass_enabled, shadows_enabled }` before Bevy 0.17)
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn vertex_shader() -> ShaderRef {
        WATER_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        WATER_SHADER_HANDLE.into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct SeaPlugin;

impl Plugin for SeaPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, WATER_SHADER_HANDLE, "shaders/water.wesl");
        // The vertex stage displaces the lattice and the prepass would draw the
        // flat one: wherever a wave sits below the flat plane its fragment then
        // fails the depth test against its own prepass depth, and the swell
        // shows as bands of missing water. The sea draws in the main pass only.
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default())
            .add_systems(OnEnter(crate::AppState::Playing), spawn_sea)
            .add_systems(Update, update_water.run_if(bevy::prelude::in_state(crate::AppState::Playing)));
    }
}

/// Straight-line distance to the nearest ground the terrain colours as dry
/// (see the module doc): a cheap stand-in for bathymetric depth, probed
/// outward in rings rather than solved as a real distance transform, because
/// the coastline here is short enough that it does not need one.
fn distance_to_shore(scn: &Scenario, p: Pos) -> f32 {
    for &r in &SHORE_PROBE_RADII {
        for k in 0..SHORE_PROBE_DIRS {
            let a = k as f32 / SHORE_PROBE_DIRS as f32 * std::f32::consts::TAU;
            let probe = Pos {
                x: p.x + r * a.cos(),
                y: p.y + r * a.sin(),
            };
            // `elev_at`, not a bare `world.contains` check: a point already
            // out in the `SEA_EXTEND_M` extension is by definition outside
            // the real window, so the old `!contains(probe)` test tripped on
            // the very first, smallest probe ring in every direction and
            // reported the whole extension as if it were lapping the shore.
            if elev_at(scn, probe) > WATER_ELEV_MAX {
                return r;
            }
        }
    }
    SHORE_PROBE_RADII[SHORE_PROBE_RADII.len() - 1]
}

fn spawn_sea(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    let scn = &sim.scn;
    let t = &scn.terrain;
    let stride_m = t.posting * WATER_STRIDE as f32;

    let handle = materials.add(WaterMaterial {
        uniform: WaterUniform {
            wind: Vec4::ZERO,
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            sun_color: Vec4::ONE,
            deep_color: Vec4::new(0.02, 0.09, 0.20, 1.0),
            shallow_color: Vec4::new(0.09, 0.42, 0.40, 1.0),
            mask_rect: Vec4::new(1.0 / scn.world.width_m, 1.0 / scn.world.height_m, 0.0, 0.0),
            tactical: Vec4::ZERO,
        },
        shore: Some(crate::ground_mask::ROAD_MASK),
    });
    commands.insert_resource(WaterHandle(handle.clone()));

    // The near sea at full resolution over the window and a margin, then four
    // bands of open sea around it. The east and west bands run the full
    // height, so the corners belong to them and no band has to meet another
    // one diagonally.
    let (inner_x0, inner_x1) = (-SEA_EXTEND_M, t.width_m + SEA_EXTEND_M);
    let (inner_y0, inner_y1) = (-SEA_EXTEND_M, t.height_m + SEA_EXTEND_M);
    let (cx, cy) = (t.width_m * 0.5, t.height_m * 0.5);
    let (outer_x0, outer_x1) = (cx - SEA_OUT_M, cx + SEA_OUT_M);
    let (outer_y0, outer_y1) = (cy - SEA_OUT_M, cy + SEA_OUT_M);
    let s = OPEN_SEA_STRIDE_M;
    let patches = [
        Patch::spanning(inner_x0, inner_x1, inner_y0, inner_y1, stride_m, true),
        Patch::spanning(outer_x0, inner_x0, outer_y0, outer_y1, s, false),
        Patch::spanning(inner_x1, outer_x1, outer_y0, outer_y1, s, false),
        Patch::spanning(inner_x0, inner_x1, outer_y0, inner_y0, s, false),
        Patch::spanning(inner_x0, inner_x1, inner_y1, outer_y1, s, false),
    ];

    let (mut count, mut tri_count) = (0usize, 0usize);
    for patch in &patches {
        let (c, t) = spawn_patch(patch, scn, &mut commands, &mut meshes, &handle);
        count += c;
        tri_count += t;
    }

    info!(
        "sea: {count} chunks, {} k triangles @ {stride_m} m lattice",
        tri_count / 1000
    );
}

/// Mesh one lattice, chunked so the water still culls in pieces.
fn spawn_patch(
    patch: &Patch,
    scn: &Scenario,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    handle: &Handle<WaterMaterial>,
) -> (usize, usize) {
    let (coarse_cols, coarse_rows) = (patch.cols, patch.rows);
    let chunks_x = (coarse_cols - 1).div_ceil(WATER_CHUNK);
    let chunks_y = (coarse_rows - 1).div_ceil(WATER_CHUNK);
    let (mut count, mut tri_count) = (0usize, 0usize);

    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let c0 = cx * WATER_CHUNK;
            let r0 = cy * WATER_CHUNK;
            let cn = (WATER_CHUNK + 1).min(coarse_cols - c0);
            let rn = (WATER_CHUNK + 1).min(coarse_rows - r0);
            if cn < 2 || rn < 2 {
                continue;
            }

            let mut positions = Vec::with_capacity(cn * rn);
            let mut normals = Vec::with_capacity(cn * rn);
            let mut colors = Vec::with_capacity(cn * rn);
            let mut uvs = Vec::with_capacity(cn * rn);
            let mut elev = Vec::with_capacity(cn * rn);

            for r in 0..rn {
                for c in 0..cn {
                    let gx = patch.x0 + (c0 + c) as f32 * patch.sx;
                    let gy = patch.y0 + (r0 + r) as f32 * patch.sy;
                    let e = elev_at(scn, Pos { x: gx, y: gy });
                    elev.push(e);

                    positions.push([gx, WATER_BASE_Y, -gy]);
                    normals.push([0.0, 1.0, 0.0]);
                    uvs.push([gx / 200.0, gy / 200.0]);
                    let depth = if patch.probe_shore {
                        distance_to_shore(scn, Pos { x: gx, y: gy })
                    } else {
                        SHORE_PROBE_RADII[SHORE_PROBE_RADII.len() - 1]
                    };
                    colors.push([depth, patch.sx, 0.0, 1.0]);
                }
            }

            let mut indices = Vec::new();
            for r in 0..rn - 1 {
                for c in 0..cn - 1 {
                    let i00 = r * cn + c;
                    let i10 = i00 + 1;
                    let i01 = i00 + cn;
                    let i11 = i01 + 1;
                    // Any wet corner: the water runs a cell in under the shore and
                    // the 5 m terrain cuts the waterline, instead of the lattice's
                    // 15 m staircase. The plane (0.8 m, swell down to 0.3 m) is
                    // below every land node (0.62 m and up) only where the ground
                    // is wet, so inland it is simply hidden.
                    let wet = [i00, i10, i01, i11]
                        .iter()
                        .any(|&i| elev[i] <= WATER_ELEV_MAX);
                    if !wet || !inside_horizon(scn, positions[i00]) {
                        continue;
                    }
                    let (a, b, cc, d) = (i00 as u32, i10 as u32, i01 as u32, i11 as u32);
                    indices.extend_from_slice(&[a, cc, b, b, cc, d]);
                    tri_count += 2;
                }
            }
            if indices.is_empty() {
                continue;
            }

            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                // Waves animate in the shader; no CPU reader or mesh updates.
                RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_indices(Indices::U32(indices));
            mesh.compress_indices();

            commands.spawn((
                (Mesh3d(meshes.add(mesh)), MeshMaterial3d(handle.clone())),
                SeaMesh,
                // The water shader computes its own lighting from scratch
                // (see `shaders/water.wgsl`) and never samples the scene's
                // shadow maps, so casting/receiving would only cost a draw
                // and risk an odd wavy shadow on the coastline for nothing.
                NotShadowCaster,
                NotShadowReceiver,
            ));
            count += 1;
        }
    }

    (count, tri_count)
}

fn update_water(
    sim: Res<Sim>,
    sun: Res<SunState>,
    handle: Option<Res<WaterHandle>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    let Some(handle) = handle else { return };
    let Some(now) = materials.get(&handle.0).map(|m| m.uniform) else {
        return;
    };

    // Same "blows FROM" convention as the smoke and embers (`fire_view`):
    // drift is the direction the wind carries things, the opposite bearing.
    let (drift, wind_ms) = wind_drift(&sim);

    let wind = Vec4::new(drift.x, drift.y, wind_ms, 0.0);
    let sun_dir = sun.direction.extend(sun.day_gate);
    // `.a` rides along as the water's own brightness gate (see
    // `shaders/water.wgsl`) — `sun.brightness`, not `sun.day_gate`, so the
    // sea tracks the same illuminance curve the rest of the scene is lit
    // from instead of dimming early on a perfectly sunny afternoon.
    let sun_color = sun.color.extend(sun.brightness);
    // Written only when it moved (a material changed is a new GPU buffer on
    // WebGL2): the sun's direction is not compared, the shader takes it from
    // the light (`water.wesl`); colours and gates by `sky::SLOW_EPS`.
    let eps = crate::sky::SLOW_EPS;
    let stale = now.wind != wind
        || (now.sun_dir.w - sun_dir.w).abs() > eps
        || (now.sun_color - sun_color).abs().max_element() > eps
        || (!crate::quality::WEBGL2 && now.sun_dir != sun_dir);
    if stale {
        if let Some(mut mat) = materials.get_mut(&handle.0) {
            mat.uniform.wind = wind;
            mat.uniform.sun_dir = sun_dir;
            mat.uniform.sun_color = sun_color;
        }
    }
}
