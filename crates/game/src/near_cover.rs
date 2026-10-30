//! Ground cover near the camera: grass tufts drawn by the GPU.
//!
//! Far away the meadow is a pattern the terrain shader paints on the ground
//! (`shaders/terrain.wgsl`), and the trees are merged meshes ([`crate::vegetation`]).
//! Between those, within a few tens of metres, the ground is flat: the pattern
//! has no height. This layer fills it with real geometry that costs no memory
//! per plant: a fixed lattice of cells built once, whose vertex shader
//! (`shaders/near_cover.wgsl`) places every blade from the cell's world
//! position, the terrain height, the fuel under it and the fire's state, and
//! whose fragment shader lights it like the rest of the scene.
//!
//! The data the shader reads: a height texture (the 5 m render grid), a cover
//! map on the fire grid (tuft density, palette; the alpha holds
//! the fire's state and is rewritten as it burns) and the road mask, which
//! keeps blades off the asphalt and the shore. Nothing here decides anything
//! about the game: it is only a picture of the fuel.

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::light::{NotShadowCaster};
use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::camera::primitives::Aabb;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use fire::CellFire;
use scenario::{Cell, Scenario};

use crate::camera::{AuxiliaryCamera, OrbitCamera};
use crate::quality::Graphics;
use crate::sim::Sim;

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("6c41d90b-3a57-4e12-9f08-b3d752ae1c74");
const COVER_MAP: Handle<Image> = uuid_handle!("71d20e5c-9b84-4a36-8c1f-6a09e3b2d547");

/// Cell edge of the grass lattice and blades per tuft. A tuft is a clump of
/// blades fanning out of one point, which is the unit a meadow is made of at
/// this scale; the cell is smaller than the tuft's spread so tufts overlap.
const TUFT_CELL_M: f32 = 0.45;
const BLADES: u32 = 12;
/// The layer is nested lattices by distance from the EYE (not from the point
/// looked at): ring `k` draws the tufts whose straight-line distance to the
/// camera lies in `BANDS_M[k-1]..BANDS_M[k]`, with cells as coarse as that
/// distance allows for the vertex budget. What is nearer the camera is drawn
/// finer wherever it is on screen; beyond the last band the painted tufts of
/// the terrain shader are all there is. A ring the camera is too high to reach
/// is not drawn at all.
pub const BANDS_M: [f32; RINGS] = [50.0, 140.0, 400.0];
const RINGS: usize = 3;
/// Real seconds between rewrites of the fire channel of the cover map.
const FIRE_REFRESH_S: f32 = 0.25;

#[derive(Clone, Copy, ShaderType, Default)]
struct Params {
    /// Lattice centre (world x, north y), draw radius, grass cell.
    focus: Vec4,
    /// Grass lattice side in cells, terrain height_m, terrain posting, tactical amount.
    lattice: Vec4,
    /// Simulated seconds, layer scale.
    fire: Vec4,
    /// 1 / world width, 1 / world height.
    rect: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct NearCoverExtension {
    #[uniform(100)]
    params: Params,
    #[texture(101, filterable = false)]
    height: Handle<Image>,
    #[texture(102)]
    #[sampler(103)]
    cover: Handle<Image>,
    #[texture(104)]
    #[sampler(105)]
    road_mask: Handle<Image>,
}

impl NearCoverExtension {
    pub fn tactical(&self) -> f32 {
        self.params.lattice.w
    }
    pub fn set_tactical(&mut self, amount: f32) {
        self.params.lattice.w = amount;
    }
}

pub type NearCoverMaterial = ExtendedMaterial<StandardMaterial, NearCoverExtension>;

impl MaterialExtension for NearCoverExtension {
    // no depth prepass and no shadows (was `MaterialPlugin { prepass_enabled, shadows_enabled }` before Bevy 0.17)
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn vertex_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }
}

pub struct NearCoverPlugin;

impl Plugin for NearCoverPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SHADER_HANDLE, "shaders/near_cover.wesl");
        // No prepass and no shadow pass: both would run the stock vertex
        // stage on a mesh whose positions are lattice indices. The layer is
        // too fine to cast shadows worth their cost; it still receives them.
        app.add_plugins(MaterialPlugin::<NearCoverMaterial>::default());
    }
}

#[derive(Resource)]
pub struct NearCover {
    /// One per ring, innermost first.
    materials: Vec<Handle<NearCoverMaterial>>,
    /// One per ring, like `materials`.
    rings: Vec<Entity>,
    /// What the fire channel was last written for: simulated seconds and the
    /// sim's generation (a restart changes the generation, not only the time).
    written: Option<(i64, u64)>,
    since_write_s: f32,
}

/// Grass: tuft density, palette. Group by fuel class.
fn cover_of(fuel: i32) -> [u8; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (tuft, palette) = match fuel {
        1..=2 => (1.0, 0.0),
        3 => (0.8, 0.0),
        4 => (0.75, 0.3),
        5 | 6 => (0.7, 0.3),
        7..=9 => (0.7, 0.6),
        10..=12 => (0.55, 1.0),
        _ => (0.0, 0.0),
    };
    [q(tuft), 0, q(palette)]
}

/// Cells without fuel are most of the island: open dry ground, not a
/// courtyard. Away from houses, yards, roads, fields and water they get a thin
/// straw grass, so the ground between the stands is not bare.
fn bare_cover(scn: &Scenario, c: Cell) -> [u8; 3] {
    let Some(cv) = &scn.cover else { return [0; 3] };
    let p = scn.world.centre_of(c);
    let natural = [(0.0, 0.0), (7.0, 7.0), (-7.0, 7.0), (7.0, -7.0), (-7.0, -7.0)]
        .into_iter()
        .all(|(dx, dy)| cv.is_natural(scenario::Pos { x: p.x + dx, y: p.y + dy }));
    if !natural || crate::terrain_mesh::built_nearby(scn, c) > 0.02 {
        return [0; 3];
    }
    [(0.45f32 * 255.0) as u8, 0, 0]
}

pub fn spawn(
    mut commands: Commands,
    sim: Res<Sim>,
    gfx: Res<Graphics>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<NearCoverMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if gfx.cover_m <= 0.0 {
        return;
    }
    let scn = &sim.scn;
    let (t, w) = (&scn.terrain, &scn.world);

    let mut elev = Vec::with_capacity(t.elev.len() * 4);
    for h in &t.elev {
        elev.extend_from_slice(&h.to_le_bytes());
    }
    let height = images.add(Image::new(
        Extent3d { width: t.cols as u32, height: t.rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        elev,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    ));

    // Row 0 at the south, like the road mask, so that v = y / height.
    let mut data = vec![0u8; w.fire_cols * w.fire_rows * 4];
    for r in 0..w.fire_rows {
        for c in 0..w.fire_cols {
            let cell = Cell { row: r, col: c };
            let fuel = scn.fuel_at(cell);
            let [a, b, p] = if fuel == 0 { bare_cover(scn, cell) } else { cover_of(fuel) };
            let i = ((w.fire_rows - 1 - r) * w.fire_cols + c) * 4;
            data[i..i + 3].copy_from_slice(&[a, b, p]);
        }
    }
    let mut cover = Image::new(
        Extent3d { width: w.fire_cols as u32, height: w.fire_rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        // the main world rewrites the fire channel as the fire moves
        RenderAssetUsages::default(),
    );
    // Nearest: the fire state is a code, not a quantity to blend.
    cover.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });
    let _ = images.insert(&COVER_MAP, cover);

    // cells per ring side: `cover_m` is the inner ring's radius at scale 1 for a
    // single lattice; rings are cheaper, so fewer cells each
    let n_tuft = (1.3 * gfx.cover_m / TUFT_CELL_M).ceil() as usize;
    let make = || NearCoverMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            metallic: 0.0,
            reflectance: 0.03,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: NearCoverExtension {
            params: Params {
                focus: Vec4::new(0.0, 0.0, 1.0, TUFT_CELL_M),
                lattice: Vec4::new(n_tuft as f32, t.height_m, t.posting, 0.0),
                fire: Vec4::new(0.0, 1.0, 0.0, 0.0),
                rect: Vec4::new(1.0 / w.width_m, 1.0 / w.height_m, 0.0, 0.0),
            },
            height: height.clone(),
            cover: COVER_MAP,
            road_mask: crate::ground_mask::ROAD_MASK,
        },
    };
    let ring_materials: Vec<_> = (0..RINGS).map(|_| materials.add(make())).collect();

    let mut ring_entities = Vec::new();
    let _root = commands
        .spawn((Transform::default(), Visibility::default(), Name::new("near_cover")))
        .with_children(|p| {
            let mesh = meshes.add(tuft_mesh(n_tuft));
            for material in &ring_materials {
                let id = p.spawn((
                    (Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())),
                    // positions are lattice indices, not metres: the bounds
                    // are meaningless, the shader keeps everything near the
                    // camera
                    Aabb::from_min_max(Vec3::splat(-1.0e6), Vec3::splat(1.0e6)),
                    // and Bevy (0.20) must not recompute them from the mesh
                    bevy::camera::visibility::NoAutoAabb,
                    NotShadowCaster,
                )).id();
                ring_entities.push(id);
            }
        })
        .id();
    info!("near_cover: spawned, {RINGS} rings of {n_tuft} cells");
    commands.insert_resource(NearCover { materials: ring_materials, rings: ring_entities, written: None, since_write_s: 0.0 });
}

/// Every blade of the lattice: `n * n` cells of `per` blades, each `verts` vertices.
fn tuft_mesh(n: usize) -> Mesh {
    let mut b = LatticeBuilder::default();
    for iz in 0..n {
        for ix in 0..n {
            for k in 0..BLADES {
                b.blade(ix as f32, iz as f32, k as f32);
            }
        }
    }
    b.finish()
}

#[derive(Default)]
struct LatticeBuilder {
    pos: Vec<[f32; 3]>,
    shape: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl LatticeBuilder {
    fn vert(&mut self, cell: [f32; 3], side: f32, t: f32, uv_y: f32) {
        self.pos.push(cell);
        self.shape.push([side, t, 0.0]);
        self.uv.push([side * 0.5 + 0.5, uv_y]);
    }

    /// One blade: two quads' worth of width at the root and the middle, a tip.
    fn blade(&mut self, ix: f32, iz: f32, k: f32) {
        let o = self.pos.len() as u32;
        let c = [ix, iz, k];
        self.vert(c, -1.0, 0.0, 0.0);
        self.vert(c, 1.0, 0.0, 0.0);
        self.vert(c, -1.0, 0.5, 0.5);
        self.vert(c, 1.0, 0.5, 0.5);
        self.vert(c, 0.0, 1.0, 1.0);
        self.idx.extend([o, o + 1, o + 2, o + 1, o + 3, o + 2, o + 2, o + 3, o + 4]);
    }

    fn finish(self) -> Mesh {
        let n = self.pos.len();
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.shape)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; n])
            .with_inserted_indices(Indices::U32(self.idx))
    }
}

/// Follow the camera, show the layer only when it can be seen, and keep the
/// fire channel of the cover map up to date.
pub fn update(
    time: Res<Time>,
    sim: Res<Sim>,
    gfx: Res<Graphics>,
    state: Option<ResMut<NearCover>>,
    camera: Query<&GlobalTransform, (With<OrbitCamera>, Without<AuxiliaryCamera>)>,
    mut visibility: Query<&mut Visibility>,
    mut materials: ResMut<Assets<NearCoverMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut state) = state else { return };


    state.since_write_s += time.delta_secs();
    let key = (sim.time_s(), sim.generation);
    if state.written != Some(key) && (state.written.is_none() || state.since_write_s >= FIRE_REFRESH_S) {
        state.written = Some(key);
        state.since_write_s = 0.0;
        let w = sim.scn.world;
        let (st, cleared) = (sim.fire.state(), sim.fire.cleared());
        // 0 untouched, 128 burning, 255 burnt, 195 cleared to bare ground: the
        // shaders tell them apart by thresholds.
        {
            let Some(mut image) = images.get_mut(&COVER_MAP) else { return };
            for r in 0..w.fire_rows {
                for c in 0..w.fire_cols {
                    let i = r * w.fire_cols + c;
                    let code = match st[i] {
                        _ if cleared[i] => 195,
                        CellFire::Unburnt => 0,
                        CellFire::Burning => 128,
                        CellFire::Burnt => 255,
                    };
                    image.data.as_mut().expect("CPU copy")[((w.fire_rows - 1 - r) * w.fire_cols + c) * 4 + 3] = code;
                }
            }
        }
    }

    let Some(cam) = camera.iter().next() else { return };
    let (eye, fwd) = (cam.translation(), cam.forward());
    let ground = sim.scn.terrain.height_at(scenario::Pos { x: eye.x, y: -eye.z });
    let height = (eye.y - ground).max(0.0);
    let eye_xz = Vec2::new(eye.x, -eye.z);
    let ahead = Vec2::new(fwd.x, -fwd.z).normalize_or_zero();
    let n = ((1.3 * gfx.cover_m / TUFT_CELL_M).ceil()).max(1.0);
    let mut lo = 0.0;
    for (k, &hi) in BANDS_M.iter().enumerate() {
        // a ring nothing of which is within its band's reach is not drawn
        let reach = gfx.cover_m > 0.0 && height < hi;
        if let Ok(mut v) = visibility.get_mut(state.rings[k]) {
            *v = if reach { Visibility::Inherited } else { Visibility::Hidden };
        }
        if reach {
            // the square lattice reaches 1.25 x `hi` around its centre, and the
            // centre sits a quarter of `hi` ahead of the eye: the disc of radius
            // `hi` around the eye is inside it, with the room spent in front
            let cell = 2.5 * hi / n;
            let centre = eye_xz + ahead * 0.25 * hi;
            if let Some(mut m) = materials.get_mut(&state.materials[k]) {
                let p = &mut m.extension.params;
                p.focus = Vec4::new(centre.x, centre.y, 0.5 * n * cell, cell);
                p.fire = Vec4::new(sim.time_s() as f32, cell / TUFT_CELL_M, lo, hi);
            }
        }
        lo = hi;
    }
}
