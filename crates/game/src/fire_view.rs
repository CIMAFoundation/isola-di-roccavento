//! Rendering the fire.
//!
//! The model runs on a 20 m raster; the fire is deliberately *not* drawn on
//! one. Everything here samples the CA through [`crate::field`], which
//! interpolates and noise-warps the cell fields, so the visible fire has
//! structure finer than the grid that produced it: ragged scar edges, flames
//! at sub-cell positions along the front, smoke and embers as free particles.
//!
//! Four things are drawn:
//!
//! - the **scar and the glowing band** on the ground, by the terrain's own
//!   pixel shader (`shaders/fire_ground.wesl`) from the fire map written here
//!   ([`update_fire_map`]);
//! - **flames**, only in the flaming band behind the front — a cell burns for
//!   20 minutes but flames for a few, and drawing the whole burning area as
//!   flame is what makes CA fire look like a spreading blob;
//! - **smoke**, which is what actually reads at commander altitude;
//! - **embers**, the mechanism that destroys houses ahead of the front.
//!
//! Flame size comes from the cell's own fireline intensity through Byram's
//! flame-length relation — the same number the exposure model uses to decide
//! what a cell can threaten.

use bevy::prelude::*;
use bevy::pbr::{FogFalloff, DistanceFog};
use bevy::camera::Exposure;
use fire::exposure::flame_length_m;
use fire::CellFire;
use scenario::Pos;

use crate::field::{noise, FireField};
use crate::streamed_mesh::{push_vertex, Shape, Streamed, StreamedFrame};
use crate::fire_shader::{FireMaterial, FireVolumeMaterial, SmokeMaterial, SmokeVolumeMaterial};
use crate::sim::Sim;
use crate::textures;

/// Fire map rewrite throttle (`update_fire_map`). At high time acceleration
/// the generation advances several times per frame.
const FIRE_MAP_MIN_INTERVAL_S: f32 = 0.15;

/// How long a cell actually *flames* after the front arrives. The model keeps
/// a cell alight for 20 minutes (`fire::BURNOUT_S`), which is residence time
/// including smouldering; the flaming zone of a real fire is a band of tens of
/// metres, not the whole burnt area.
/// 2026-11-04 (the user: «lo voglio più realistico»): from 900 s, which
/// drew a carpet of flames a hundred metres deep behind the front.
pub(crate) const FLAMING_S: f32 = 540.0;

/// Caps, all of them per-frame geometry budgets rather than physical limits.
const MAX_FLAME_CELLS: usize = if crate::quality::WEBGL2 { 600 } else { 3_500 };
/// 2026-11-05: from 1400; fewer, bigger puffs billow, many small ones haze.
const MAX_SMOKE: usize = if crate::quality::WEBGL2 { 100 } else { 900 };
const MAX_EMBERS: usize = if crate::quality::WEBGL2 { 60 } else { 525 };
/// Burning plants given flames of their own (`plant_torches`), the nearest
/// to the camera first, and how far from it they are looked for.
const MAX_TORCHES: usize = if crate::quality::WEBGL2 { 150 } else { 1_200 };
const TORCH_RADIUS_M: f32 = 1_400.0;

/// The front's tongues are drawn at toy scale so the fire reads from the
/// play camera (`update_flames`); close to the camera they shrink toward
/// their true flame length, where the burning plants carry their own
/// flames. Between these distances from the camera (m) the scale blends.
const TRUE_SCALE_NEAR_M: f32 = 220.0;
const TOY_SCALE_FAR_M: f32 = 750.0;
/// Past this distance from the camera (m) a cell keeps its first tongue
/// only, over the next 40 % of it (`CachedTongue::rank`).
const FAR_TONGUES_M: f32 = 450.0;

/// The flames as volumes (2026-11-04, `shaders/fire_volume.wesl`): near the
/// camera the front's tongues give way to ray-marched boxes over the
/// flaming band, which have thickness and break into licks and wisps.
/// WebGL2 has no 3D textures worth the cost on phones.
///
/// **Off since 2026-11-05**, an experiment: measured with `KIOSK_SNAP_BENCH`
/// at 4x the pixels the smoke volumes took the frame from 20 to 76 ms on
/// the play camera and from 26 to 136 ms at 700 m from the fire, the
/// flame volumes added 5–10 ms up close, and neither looked better than
/// the billboards (a dithered carpet of fire, a dark stain of smoke).
/// `KIOSK_VOLUMI=scatole` turns them on, for a comparison (`=1` is the
/// screen-space march, `fire_volumetric`).
pub const VOLUMES: bool = !crate::quality::WEBGL2;

/// Whether the volumes are drawn: only with `KIOSK_VOLUMI=scatole`, where built in.
pub fn volumes_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| VOLUMES && std::env::var("KIOSK_VOLUMI").is_ok_and(|v| v == "scatole"))
}

/// Whether the smoke is a volume: with the flames', unless `KIOSK_FUMO=0`
/// (the puffs, as on WebGL2).
pub fn smoke_volumes_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| volumes_on() && std::env::var("KIOSK_FUMO").map_or(true, |v| v != "0"))
}

/// The puffs' roll forward from an empty state (`update_flames`): steps and
/// seconds a step, a minute of plume.
const PREROLL_STEPS: usize = 45;
const PREROLL_DT_S: f32 = 1.0;

/// Metres a step of the march, up close (`KIOSK_VOL_PASSO`, for the bench).
fn vol_step_m() -> f32 {
    std::env::var("KIOSK_VOL_PASSO").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}
/// Nearer than `VOL_FULL_M` to the eye the volumes alone burn; past
/// `VOL_END_M` the tongues alone. Between, the tongues grow as the volumes
/// fade, so nothing pops.
const VOL_FULL_M: f32 = 300.0;
const VOL_END_M: f32 = 480.0;
/// Fire cells a volume box spans on a side.
const VOL_BLOCK: usize = 3;
/// The smoke as a volume (`shaders/smoke_volume.wesl`): a box per block of
/// `SMOKE_BLOCK` cells of smoking ground, sheared downwind. With it the
/// front spawns no puffs (the burning houses keep theirs). The plume's
/// top (m above the ground), its rise (m/s), the seconds a cell smokes
/// (as the puffs: three flaming times), and metres a step of the march.
const SMOKE_BLOCK: usize = 8;
const SMOKE_TOP_M: f32 = 380.0;
const SMOKE_RISE_MS: f32 = 8.0;
const SMOKE_S: f32 = FLAMING_S * 3.0;
const SMOKE_STEP_M: f32 = 15.0;
/// Extinction per metre at full density, noise period (m), gain.
const SMOKE_LOOK: Vec4 = Vec4::new(0.06, 170.0, 1.0, FLAMING_S);
/// The flames' height over every cell, metres as half floats, one texel per
/// fire cell laid out as the fire map (`ground_mask::GRASS_MAP`), written
/// with it (`update_fire_map`): the volumes read their local height from
/// it, so it runs smoothly across the boxes and follows the sim's own
/// intensity (`vol_flame_m`).
pub const FLAME_MAP: Handle<Image> = bevy::asset::uuid_handle!("b1c4e8a2-6f3d-4a97-8e21-7d5c9b0a3f16");
/// Metres of noise per period, periods risen per second, emission gain and
/// absorption per metre of dense flame (`VolumeParams::look`).
const VOL_LOOK: Vec4 = Vec4::new(1.0, 14.0, 0.9, 0.45);

/// As `CLIMB_S_PER_M` and `CROWN_SHARE` in `shaders/plant_common.wesl`:
/// seconds the fire takes to climb a metre of a plant, and the share of
/// its flaming time a point of the crown burns for. Keep them in step, or
/// the flames leave the crown before it chars.
const CLIMB_S_PER_M: f32 = 2.8;
const CROWN_SHARE: f32 = 0.5;
/// The share of a point's burn time its leaves flame for: the shader eats
/// them by about this point of their burn (`burn_gone` in
/// `plant_common.wesl`); after it the twigs only glow.
const FLAME_SHARE: f32 = 0.6;

/// Shape of the firebrand's flight, mirroring `compute_spotting` in
/// `propagator-core/src/kernel.rs` — the median landing distance is linear in
/// wind speed and grows with the source cell's fireline intensity through
/// plume lofting (`d ~ U * I^(1/3)`), concentrated downwind. This is a visual
/// echo only: the core decides where the fire actually spots, this only
/// decides where the ember on screen appears to land. Kept in step with the
/// core's constants so a low-wind creeping fire visibly throws embers a few
/// metres and a wind-driven crown fire throws them across a field, matching
/// what the model is doing underneath.
const SPOT_DISTANCE_REF_M: f32 = 100.0;
const SPOT_WIND_REF_KMH: f32 = 20.0;
const SPOT_FLI_REF: f32 = 10_000.0;
const SPOT_FLI_EXPONENT: f32 = 1.0 / 3.0;
const SPOT_ANISOTROPY: f32 = 5.0;

#[derive(Resource)]
pub struct FireView {
    /// Rewritten every frame in place (`streamed_mesh`): tongues, billboards.
    flames: Streamed,
    smoke_mesh: Streamed,
    sparks: Streamed,
    last_generation: u64,
    since_fire_map: f32,
    smoke: Vec<Particle>,
    embers: Vec<Particle>,
    seed: u64,
    /// Fractions of a puff and of an ember owed from earlier frames, so the
    /// spawn rate does not depend on the frame rate.
    owed: (f32, f32),
    /// Flame tongues of the fire front, rebuilt only when the sim moves.
    tongues: Vec<CachedTongue>,
    tongues_key: (u64, u32),
    /// Vertex count of the last flame mesh, to size the next allocation.
    last_flame_verts: usize,
    /// The burning plants of the frame (`plant_torches`), kept for its room.
    torches: Vec<crate::vegetation::torch::Torch>,
    /// The flame volumes (`update_volumes`): their mesh, the boxes of the
    /// last sim state, and the box the eye was in when the mesh was written.
    volumes: Option<Streamed>,
    boxes: Vec<VolumeBox>,
    boxes_key: Option<((u64, u32), IVec3)>,
    boxes_written: Option<Option<usize>>,
    /// The smoke volumes (`update_smoke_volumes`): mesh, material, the
    /// blocks of smoking ground (cell block row and column, ground range).
    smoke_volumes: Option<(Streamed, Handle<SmokeVolumeMaterial>)>,
    smoke_blocks: Vec<SmokeBlock>,
    smoke_key: Option<(u64, u32)>,
}

/// A block of smoking ground: its footprint centre (Bevy x, z), the
/// ground's range under it (m).
#[derive(Clone, Copy)]
struct SmokeBlock {
    x: f32,
    z: f32,
    g_lo: f32,
    g_hi: f32,
}

/// A volume of flame over a block of cells: an axis-aligned box in Bevy
/// space, the flame height (m) and heat it carries.
#[derive(Clone, Copy)]
struct VolumeBox {
    centre: Vec3,
    half: Vec3,
    flame_m: f32,
    heat: f32,
}

/// Everything about a front tongue that does not depend on the frame's clock
/// or camera: terrain height, hashes, intensities.
struct CachedTongue {
    foot: Vec3,
    flame_m: f32,
    /// The flame length the fire has (Byram), drawn up close.
    true_m: f32,
    ratio: f32,
    phase: f32,
    heat: f32,
    flaming: f32,
    /// Crown flare-up: height factor (1 = none), cycle offset, lift at peak.
    crown_h: f32,
    crown_cyc: f32,
    crown_lift: f32,
    /// Which of its cell's tongues this is: from afar only the first is
    /// drawn (`update_flames`).
    rank: u8,
}

/// A drifting particle: smoke puff or ember, which differ only in how they are
/// spawned, how they move and what texture they carry.
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    size: f32,
    /// Per-particle randomness, reused for rotation and shade.
    phase: f32,
    /// Set on the brief flash spawned where a firebrand lands on unburnt
    /// fuel — the visual echo of the core's own spotting model landing an
    /// ember (see `step_particles`). Ordinary embers leave this false.
    flare: bool,
}

pub fn setup(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut smoke_materials: ResMut<Assets<SmokeMaterial>>,
    mut fire_materials: ResMut<Assets<FireMaterial>>,
    volume_materials: Option<ResMut<Assets<FireVolumeMaterial>>>,
    smoke_volume_materials: Option<ResMut<Assets<SmokeVolumeMaterial>>>,
    mut images: ResMut<Assets<Image>>,
) {
    let flames = Streamed::new(&mut meshes, TONGUE, false);
    let smoke_mesh = Streamed::new(&mut meshes, BILLBOARD, true);
    let sparks = Streamed::new(&mut meshes, BILLBOARD, false);

    // with the volumes on only the burning houses play the film: at half
    // its side (a quarter on WebGL2, the phones'), 5 MB or 1 instead of 21
    let skip = if !crate::fire_volumetric::on() { 0 } else if crate::quality::WEBGL2 { 2 } else { 1 };
    let flame_tex = images.add(textures::flame_flipbook(skip));
    let puff_tex = images.add(textures::puff());
    let spark_tex = images.add(textures::spark());

    // Shader materials, additive: flames and embers are light, not surfaces.
    // Vertex colours run above 1.0 so the camera's bloom pass catches them,
    // and the fragment shader domain-warps the texture lookup so the flame
    // edge roils instead of sitting still.
    let flame_mat = fire_materials.add(FireMaterial { texture: flame_tex, mode: Vec4::X });
    let spark_mat = fire_materials.add(FireMaterial { texture: spark_tex, mode: Vec4::new(2.0, 0.0, 0.0, 0.0) });
    // Smoke occludes: alpha blending, and unlit so a plume does not flicker
    // as it crosses the terrain's shadow line.
    let smoke_mat = smoke_materials.add(SmokeMaterial { texture: puff_tex });

    commands.spawn((
        (Mesh3d(flames.handle.clone()), MeshMaterial3d(flame_mat)),
        // Mesh vertices change every frame; Bevy's cached bounds do not.
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
    ));
    commands.spawn((
        (Mesh3d(sparks.handle.clone()), MeshMaterial3d(spark_mat)),
        // Mesh vertices change every frame; Bevy's cached bounds do not.
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
    ));
    commands.spawn((
        (Mesh3d(smoke_mesh.handle.clone()), MeshMaterial3d(smoke_mat)),
        // Mesh vertices change every frame; Bevy's cached bounds do not.
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
    ));

    // The flame volumes: boxes over the band, the world's fire map and
    // heights to march through (`update_volumes`).
    let volumes = volume_materials.filter(|_| volumes_on()).map(|mut volume_materials| {
        let volumes = Streamed::new(&mut meshes, BOX, true);
        let (t, w) = (&sim.scn.terrain, &sim.scn.world);
        let (heights, elev_lo, elev_scale) = textures::heights(t);
        let mut flame_map = crate::ground_mask::grass_map(&sim.scn);
        flame_map.texture_descriptor.format = bevy::render::render_resource::TextureFormat::R16Float;
        flame_map.data = Some(vec![0u8; w.fire_cols * w.fire_rows * 2]);
        let _ = images.insert(&FLAME_MAP, flame_map);
        let noise = images.add(textures::noise_volume());
        let heights = images.add(heights);
        let mat = volume_materials.add(FireVolumeMaterial {
            params: crate::fire_shader::VolumeParams {
                rect: Vec4::new(1.0 / w.width_m, 1.0 / w.height_m, t.posting, t.height_m),
                fade: Vec4::new(FLAMING_S, VOL_FULL_M, VOL_END_M, vol_step_m()),
                look: VOL_LOOK,
                elev: Vec4::new(elev_lo, elev_scale, 0.0, 0.0),
            },
            noise: noise.clone(),
            fire_map: crate::ground_mask::GRASS_MAP,
            heights: heights.clone(),
            flame_map: FLAME_MAP,
        });
        commands.spawn((
            (Mesh3d(volumes.handle.clone()), MeshMaterial3d(mat)),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
        ));
        (volumes, noise, heights, elev_lo, elev_scale)
    });
    let smoke_volumes = match (volumes.as_ref(), smoke_volume_materials) {
        (Some((_, noise, heights, elev_lo, elev_scale)), Some(mut smoke_volume_materials)) if smoke_volumes_on() => {
            let (t, w) = (&sim.scn.terrain, &sim.scn.world);
            let mat = smoke_volume_materials.add(SmokeVolumeMaterial {
                params: crate::fire_shader::SmokeParams {
                    rect: Vec4::new(1.0 / w.width_m, 1.0 / w.height_m, t.posting, t.height_m),
                    plume: Vec4::new(SMOKE_TOP_M, SMOKE_RISE_MS, SMOKE_S, SMOKE_STEP_M),
                    wind: smoke_wind(&sim),
                    look: SMOKE_LOOK,
                    elev: Vec4::new(*elev_lo, *elev_scale, 0.0, 0.0),
                },
                noise: noise.clone(),
                fire_map: crate::ground_mask::GRASS_MAP,
                heights: heights.clone(),
                flame_map: FLAME_MAP,
            });
            let mesh = Streamed::new(&mut meshes, BOX, true);
            commands.spawn((
                (Mesh3d(mesh.handle.clone()), MeshMaterial3d(mat.clone())),
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
            ));
            Some((mesh, mat))
        }
        _ => None,
    };
    let volumes = volumes.map(|v| v.0);

    commands.insert_resource(FireView {
        flames,
        smoke_mesh,
        sparks,
        last_generation: u64::MAX,
        since_fire_map: FIRE_MAP_MIN_INTERVAL_S,
        smoke: Vec::new(),
        embers: Vec::new(),
        seed: 0x5EED_1E55,
        owed: (0.0, 0.0),
        tongues: Vec::new(),
        tongues_key: (u64::MAX, 0),
        last_flame_verts: 0,
        torches: Vec::new(),
        volumes,
        boxes: Vec::new(),
        boxes_key: None,
        boxes_written: None,
        smoke_volumes,
        smoke_blocks: Vec::new(),
        smoke_key: None,
    });
}

/// Clear the drifting particles when the sim restarts.
///
/// The overlay and the flame billboards are rebuilt from the fire state every
/// time it goes stale, so they need no help — the generation bump does it. Smoke
/// and embers are the exception: they are *simulated here*, carrying their own
/// position and age, and a plume left over from the old fire would go on
/// drifting over a landscape that never burnt.
pub fn reset(mut restarted: MessageReader<crate::sim::SimRestarted>, mut view: ResMut<FireView>) {
    if restarted.is_empty() {
        return;
    }
    restarted.clear();
    view.smoke.clear();
    view.embers.clear();
}

// --- fire map -------------------------------------------------------------

/// Writes what the fire did into the fire map the terrain's shader draws the
/// scar and the glow from (`shaders/fire_ground.wesl`): the meadow's texture,
/// one texel per fire cell, linearly filtered, so a sample interpolates as
/// [`FireField`] does. g: burnt (any state but unburnt); b: the arrival time
/// against now, times g (filtered, b / g is [`FireField::arrival`]'s mean
/// over the burnt corners); a: cut by the crews. r, the meadow, is left alone.
pub fn update_fire_map(
    sim: Res<Sim>,
    time: Res<Time>,
    mut view: ResMut<FireView>,
    mut images: ResMut<Assets<Image>>,
) {
    view.since_fire_map += time.delta_secs();
    if sim.generation == view.last_generation || view.since_fire_map < FIRE_MAP_MIN_INTERVAL_S {
        return;
    }
    let Some(mut image) = images.get_mut(&crate::ground_mask::GRASS_MAP) else { return };
    let Some(data) = image.data.as_mut() else { return };
    view.last_generation = sim.generation;
    view.since_fire_map = 0.0;
    let w = sim.scn.world;
    let now = sim.time_s() as f32;
    let (state, arrival, cleared) = (sim.fire.state(), sim.fire.arrival_times(), sim.fire.cleared());
    let f16 = crate::terrain_mesh::f16_bits;
    for r in 0..w.fire_rows {
        for c in 0..w.fire_cols {
            let i = r * w.fire_cols + c;
            let burnt = state[i] != CellFire::Unburnt;
            // half floats: ages past ~16 h all read as long cold
            let since = match (burnt, arrival[i]) {
                (false, _) => 0.0,
                (true, i32::MIN) => -60_000.0,
                (true, t) => (t as f32 - now).clamp(-60_000.0, 0.0),
            };
            let at = ((w.fire_rows - 1 - r) * w.fire_cols + c) * 8;
            data[at + 2..at + 4].copy_from_slice(&f16(f32::from(burnt)).to_le_bytes());
            data[at + 4..at + 6].copy_from_slice(&f16(since).to_le_bytes());
            data[at + 6..at + 8].copy_from_slice(&f16(f32::from(cleared[i])).to_le_bytes());
        }
    }
    drop(image);
    // the volumes' flame heights (`FLAME_MAP`), where something burns
    let Some(mut flame_map) = images.get_mut(&FLAME_MAP) else { return };
    let Some(data) = flame_map.data.as_mut() else { return };
    let intensity = sim.fire.intensity();
    for r in 0..w.fire_rows {
        for c in 0..w.fire_cols {
            let i = r * w.fire_cols + c;
            let h = if state[i] == CellFire::Unburnt { 0.0 } else { vol_flame_m(intensity[i]) };
            let at = ((w.fire_rows - 1 - r) * w.fire_cols + c) * 2;
            data[at..at + 2].copy_from_slice(&f16(h).to_le_bytes());
        }
    }
}

/// The flame volumes' height (m) over a cell of fireline intensity `fli`:
/// twice Byram's length, since the wisps reach past the mean flame and
/// the fire has to be seen from a hundred metres, with a floor for the
/// creeping fire.
pub(crate) fn vol_flame_m(fli: f32) -> f32 {
    (flame_length_m(fli) * 2.2).clamp(5.0, 30.0)
}

/// Rows per flame tongue ribbon.
const TONGUE_SEGMENTS: usize = 7;

/// A flame tongue as [`QuadBuilder::tongue`] lays it out: two vertices per
/// row, two triangles per segment.
const TONGUE: Shape = Shape { verts: 2 * (TONGUE_SEGMENTS as u32 + 1), indices: &TONGUE_INDICES };
const TONGUE_INDICES: [u32; TONGUE_SEGMENTS * 6] = {
    let mut out = [0u32; TONGUE_SEGMENTS * 6];
    let mut j = 0;
    while j < TONGUE_SEGMENTS {
        let a = j as u32 * 2;
        let k = j * 6;
        (out[k], out[k + 1], out[k + 2], out[k + 3], out[k + 4], out[k + 5]) = (a, a + 1, a + 3, a, a + 3, a + 2);
        j += 1;
    }
    out
};
/// A billboard as [`QuadBuilder::billboard`] lays it out.
const BILLBOARD: Shape = Shape { verts: 4, indices: &[0, 1, 2, 0, 2, 3] };
/// A volume box as [`QuadBuilder::volume`] lays it out: corner `i` at
/// (`i & 1`, `i >> 1 & 1`, `i >> 2 & 1`) of the box, every face wound
/// counter-clockwise seen from outside (the material culls the back).
const BOX: Shape = Shape {
    verts: 8,
    indices: &[
        1, 3, 7, 1, 7, 5, // +x
        0, 4, 6, 0, 6, 2, // -x
        2, 6, 7, 2, 7, 3, // +y
        0, 1, 5, 0, 5, 4, // -y
        4, 5, 7, 4, 7, 6, // +z
        0, 2, 3, 0, 3, 1, // -z
    ],
};

/// What a row of every tongue shares: its height share `s` (0 foot, 1 tip)
/// and powers, and the waves' phase offsets along
/// the tongue as (cos, sin): 4.5 s, 7 s and 3 s.
struct TongueRow {
    s: f32,
    s2: f32,
    s3: f32,
    w1: (f32, f32),
    w2: (f32, f32),
    w3: (f32, f32),
}

fn tongue_rows() -> &'static [TongueRow; TONGUE_SEGMENTS + 1] {
    static ROWS: std::sync::OnceLock<[TongueRow; TONGUE_SEGMENTS + 1]> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        std::array::from_fn(|j| {
            let s = j as f32 / TONGUE_SEGMENTS as f32;
            let cs = |k: f32| {
                let (sin, cos) = (k * s).sin_cos();
                (cos, sin)
            };
            TongueRow { s, s2: s * s, s3: s * s * s, w1: cs(4.5), w2: cs(7.0), w3: cs(3.0) }
        })
    })
}

/// Streamed geometry (flames, smoke, sparks), packed as it is made in the
/// order a streamed mesh takes (`streamed_mesh::push_vertex`): no arrays per
/// attribute to interleave afterwards, ~30 000 vertices a frame on a big fire.
struct QuadBuilder {
    bytes: Vec<u8>,
    verts: usize,
    /// Flames and embers read no normal (their shader never lights); only the
    /// smoke uses the slot, for its fog.
    normals: bool,
}

impl QuadBuilder {
    fn new(normals: bool, verts: usize) -> Self {
        QuadBuilder { bytes: Vec::with_capacity(verts * crate::streamed_mesh::stride(normals)), verts: 0, normals }
    }

    fn vertex(&mut self, p: Vec3, normal: [f32; 3], uv: [f32; 2], color: [f32; 4]) {
        push_vertex(&mut self.bytes, p.into(), self.normals.then_some(normal), uv, color);
        self.verts += 1;
    }

    /// A textured billboard. `right`/`up` are world-space half-extents;
    /// `taper` narrows the top edge, which is what gives a flame tongue its
    /// shape independently of the texture.
    /// `normal` goes in the normal slot, if the builder has one.
    #[allow(clippy::too_many_arguments)]
    fn billboard(
        &mut self,
        centre: Vec3,
        right: Vec3,
        up: Vec3,
        taper: f32,
        bottom: [f32; 4],
        top: [f32; 4],
        normal: [f32; 3],
    ) {
        self.vertex(centre - right - up, normal, [0.0, 1.0], bottom);
        self.vertex(centre + right - up, normal, [1.0, 1.0], bottom);
        self.vertex(centre + right * taper + up, normal, [1.0, 0.0], top);
        self.vertex(centre - right * taper + up, normal, [0.0, 0.0], top);
        // joined by `BILLBOARD`'s indices
    }

    /// A billboard with a colour per corner (bottom left, bottom right, top
    /// right, top left), for the shaded puffs.
    fn billboard_corners(&mut self, centre: Vec3, right: Vec3, up: Vec3, colors: [[f32; 4]; 4], normal: [f32; 3]) {
        self.vertex(centre - right - up, normal, [0.0, 1.0], colors[0]);
        self.vertex(centre + right - up, normal, [1.0, 1.0], colors[1]);
        self.vertex(centre + right + up, normal, [1.0, 0.0], colors[2]);
        self.vertex(centre - right + up, normal, [0.0, 0.0], colors[3]);
    }

    /// A flame tongue: a sheet of `TONGUE_SEGMENTS` rows from foot to tip,
    /// on which the shader plays a filmed flame (`flame.wesl`, the
    /// flipbook); its top sways a little on two travelling waves. The
    /// tongue's number, from `phase`, rides in uv.x and picks its film.
    /// `foot` is the centre of the base; `h` the height.
    #[allow(clippy::too_many_arguments)]
    fn tongue(
        &mut self,
        foot: Vec3,
        right: Vec3,
        h: f32,
        half_w: f32,
        t: f32,
        phase: f32,
        bottom: [f32; 4],
        top: [f32; 4],
    ) {
        // Each wave's phase at the foot; a row's phase is that minus (or plus)
        // a fixed offset, so its sine is an angle sum with the row's constants:
        // three sin_cos per tongue instead of a sine per row and wave and a
        // power per row (libm in wasm: a fifth of the browser's frame).
        let (a1, b1) = (t * 7.0 + phase).sin_cos();
        let (a2, b2) = (t * 11.3 + phase * 1.7).sin_cos();
        let (a3, b3) = (t * 9.0 + phase).sin_cos();
        let id = 2.0 * (hash01((phase * 1000.0) as i64 as u64 ^ 0x5EED) * 256.0).floor().min(255.0);
        for row in tongue_rows() {
            let (s, s2, s3) = (row.s, row.s2, row.s3);
            // Two waves running upward at different speeds: the second makes
            // the motion non-periodic to the eye.
            let w1 = a1 * row.w1.0 - b1 * row.w1.1; // sin(t·7 + phase − 4.5 s)
            let w2 = a2 * row.w2.0 - b2 * row.w2.1; // sin(t·11.3 + 1.7 phase − 7 s)
            let w3 = a3 * row.w3.0 + b3 * row.w3.1; // sin(t·9 + phase + 3 s)
            // The film moves the flame (`flame.wesl`, a flipbook): the sheet
            // only sways at the top, and keeps its width, or the frames
            // would be squeezed into a point (2026-11-05).
            let lateral = h * (0.08 * s2 * w1 + 0.04 * s3 * w2);
            let width = half_w * (1.0 + 0.04 * w3) * (1.0 - 0.15 * s);
            let centre = foot + Vec3::Y * (h * s) + right * lateral;
            let color = [
                bottom[0] + (top[0] - bottom[0]) * s,
                bottom[1] + (top[1] - bottom[1]) * s,
                bottom[2] + (top[2] - bottom[2]) * s,
                // the foot's fade (`flame.wesl`): 1 on the ground's tongues
                bottom[3] + (top[3] - bottom[3]) * s,
            ];
            // uv.y: 1 at the foot, 0 at the tip (what the shader expects);
            // uv.x: 2 id + 0.5 across the sheet, the tongue's own number
            // choosing its film (`flame.wesl`)
            let side = right * width;
            self.vertex(centre - side, [0.0, 1.0, 0.0], [id + 0.5, 1.0 - s], color);
            self.vertex(centre + side, [0.0, 1.0, 0.0], [id + 1.5, 1.0 - s], color);
        }
        // joined by `TONGUE`'s indices
    }

    /// A flame volume (`shaders/fire_volume.wesl`): the box's corners, each
    /// carrying the box itself (centre in the normal slot, half extents x
    /// and z in the uv, flame height, heat and half height in the colour).
    /// `quad`: the four corners of a quad across the view instead, for a box
    /// around the eye (it stands on the `+z` face; the rest degenerates).
    fn volume(&mut self, b: &VolumeBox, quad: Option<[Vec3; 4]>) {
        let normal = b.centre.into();
        let uv = [b.half.x, b.half.z];
        let color = [b.flame_m, b.heat, b.half.y, 0.0];
        for i in 0..8 {
            let p = match quad {
                Some(q) => q[if i < 4 { 0 } else { [0, 1, 3, 2][i - 4] }],
                None => b.centre + b.half * Vec3::new(if i & 1 == 1 { 1.0 } else { -1.0 }, if i & 2 == 2 { 1.0 } else { -1.0 }, if i & 4 == 4 { 1.0 } else { -1.0 }),
            };
            self.vertex(p, normal, uv, color);
        }
    }

    /// A smoke volume (`shaders/smoke_volume.wesl`): the box's corners,
    /// each carrying its block's footprint (centre in the normal slot, half
    /// side in the uv) and the box's centre y and half height (colour).
    fn smoke_volume(&mut self, b: &VolumeBox, blk: &SmokeBlock, half: f32, quad: Option<[Vec3; 4]>) {
        let normal = [blk.x, (blk.g_lo + blk.g_hi) * 0.5, blk.z];
        let uv = [half, half];
        let color = [b.centre.y, b.half.y, 0.0, 0.0];
        for i in 0..8 {
            let p = match quad {
                Some(q) => q[if i < 4 { 0 } else { [0, 1, 3, 2][i - 4] }],
                None => b.centre + b.half * Vec3::new(if i & 1 == 1 { 1.0 } else { -1.0 }, if i & 2 == 2 { 1.0 } else { -1.0 }, if i & 4 == 4 { 1.0 } else { -1.0 }),
            };
            self.vertex(p, normal, uv, color);
        }
    }

    /// The position of vertex `i` (tests).
    #[cfg(test)]
    fn position(&self, i: usize) -> Vec3 {
        let at = i * crate::streamed_mesh::stride(self.normals);
        let f = |k: usize| f32::from_le_bytes(self.bytes[at + 4 * k..at + 4 * k + 4].try_into().unwrap());
        Vec3::new(f(0), f(1), f(2))
    }
}

// --- flames, smoke, embers -------------------------------------------------

/// Rebuild the particle geometry. Runs every frame: the fire has to move even
/// when the sim is paused between steps or running at 1x.
pub fn update_flames(
    sim: Res<Sim>,
    time: Res<Time>,
    camera: Query<(&Transform, &Projection), (With<Camera3d>, Without<crate::camera::AuxiliaryCamera>)>,
    buildings: Res<crate::buildings::Buildings>,
    vegetation: Option<Res<crate::vegetation::Vegetation>>,
    fog: Query<(&DistanceFog, Option<&Exposure>), With<Camera3d>>,
    suns: Query<(&GlobalTransform, &DirectionalLight)>,
    mut view: ResMut<FireView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut frame: ResMut<StreamedFrame>,
    mut smoke_volume_materials: Option<ResMut<Assets<SmokeVolumeMaterial>>>,
) {
    let Ok((cam, projection)) = camera.single() else {
        return;
    };
    let scn = &sim.scn;
    let t = time.elapsed_secs();
    let dt = time.delta_secs().min(0.1);
    let now = sim.time_s() as f32;

    // Billboard axes: horizontal-facing, so flames stay upright rather than
    // tipping over when the camera looks down at the ground.
    let mut right = cam.rotation * Vec3::X;
    right.y = 0.0;
    let right = right.normalize_or_zero();
    let up = Vec3::Y;

    let field = FireField {
        arrival: sim.fire.arrival_times(),
        intensity: sim.fire.intensity(),
        cleared: sim.fire.cleared(),
        world: scn.world,
    };

    // The tongue list depends on the fire state only; the sim moves in whole
    // steps, so rebuild it on a new generation (or sim time), not every frame.
    let key = (sim.generation, now.to_bits());
    if view.tongues_key != key {
        view.tongues_key = key;
        let mut tongues = std::mem::take(&mut view.tongues);
        tongues.clear();
        let active = sim.fire.active_cells();
        let stride = (active.len() / MAX_FLAME_CELLS).max(1);
        for cell in active.iter().step_by(stride) {
            let fli = sim.fire.cell_intensity(*cell);
            let arrival = sim.fire.arrival_time(*cell).unwrap_or(now as i32) as f32;
            // The flaming band: only cells the front has just crossed carry flame.
            let flaming = (1.0 - (now - arrival) / FLAMING_S).clamp(0.0, 1.0);
            // the band's tail carries no lone candles: char and embers
            if flaming < 0.2 {
                continue;
            }

            // Byram flame length, with a floor: a metre-high creeping flame is
            // physically right and visually nothing, and the player still has to
            // be able to see where the fire is.
            // Toy scale, like the houses (`buildings::TOY_SCALE`): from the play
            // camera a true-height flame is a few pixels and the fire read as a
            // dark smudge; the front has to be the brightest thing on the table.
            let flame_m = (flame_length_m(fli) * 3.4).clamp(22.0, 110.0) * (0.45 + 0.55 * flaming);
            // up close: the true length, with a floor so the band still flames
            // (a quarter of the toy scale kept: the front must still read as
            // a line of fire over tall grass a hundred metres off)
            let true_m = ((flame_length_m(fli) * 1.2).clamp(1.2, 30.0) * (0.45 + 0.55 * flaming)).max(flame_m * 0.25).min(flame_m);
            // A hot cell carries several tongues, a creeping one carries a single
            // flicker. Sub-cell placement is what stops them lining up on a grid.
            // (2026-11-05, the flipbook: each film is one narrow flame, so
            // the front is drawn by many side by side: up to 12 a cell, of
            // which only the first stays past `FAR_TONGUES_M`)
            let n = (2.0 + (fli / 300.0).min(10.0) * flaming).round() as usize;
            let centre = scn.world.centre_of(*cell);
            let half = scn.world.cellsize * 0.5;

            for k in 0..n {
                let h = hash01(cell.row as u64 * 7919 + cell.col as u64 * 104_729 + k as u64 * 31);
                let h2 = hash01(cell.row as u64 * 104_729 + cell.col as u64 * 7919 + k as u64 * 97);
                let p = Pos {
                    x: centre.x + (h - 0.5) * 2.0 * half,
                    y: centre.y + (h2 - 0.5) * 2.0 * half,
                };
                let ground = scn.terrain.height_at(p);
                let fuel = scn.fuel_at(*cell);
                let phase = h * 30.0 + k as f32 * 2.3;
                // Local intensity, not the cell's: the interpolated field varies
                // across the cell, so tongues differ within one cell too.
                let local = field.intensity(p).max(fli * 0.4);
                // Additive light competing with a daylit hillside: the multiplier
                // has to be well above 1 or the flame is a pale smudge.
                let heat = (local / 2000.0).clamp(0.55, 2.2) * (0.6 + 0.4 * flaming);
                // tongues of a fire are not alike: a few tall, most low
                let h3 = hash01(cell.row as u64 * 31 + cell.col as u64 * 7 + k as u64 * 104_729);
                let vary = 0.45 + 1.1 * h3 * h3;
                tongues.push(CachedTongue {
                    foot: Vec3::new(p.x, ground, -p.y),
                    flame_m: flame_m * vary,
                    true_m: true_m * vary,
                    ratio: 0.6 + 0.4 * (local / fli.max(1.0)).min(1.5),
                    phase,
                    heat,
                    flaming,
                    // Conifers (tall pines) flare higher than broadleaves.
                    crown_h: match fuel {
                        10..=12 => 2.6,
                        4..=6 => 1.9,
                        _ => 1.0,
                    },
                    crown_cyc: h * 13.0 + h2 * 5.0,
                    crown_lift: if fuel >= 10 { 12.0 } else { 7.0 },
                    rank: k.min(255) as u8,
                });
            }
        }
        view.tongues = tongues;
    }
    // the boxes follow the sim, and the eye by 50 m steps: their height is
    // the flames' at that distance (`grow`), so the near ones stay tight
    let eye_cell = (cam.translation / 50.0).floor().as_ivec3();
    if view.volumes.is_some() && view.boxes_key != Some((key, eye_cell)) {
        view.boxes_key = Some((key, eye_cell));
        let mut boxes = std::mem::take(&mut view.boxes);
        build_boxes(&sim, now, cam.translation, &mut boxes);
        view.boxes = boxes;
        view.boxes_written = None;
    }

    let mut flames = QuadBuilder::new(false, view.last_flame_verts);
    let max_half_w = scn.world.cellsize * 1.3;
    let eye = cam.translation;
    for c in &view.tongues {
        let flicker = 0.62 + 0.38 * (t * 5.5 + c.phase).sin() * (t * 2.3 + c.phase).cos();
        // toy scale from afar, the true flame length up close
        let d_eye = eye.distance(c.foot);
        let toy = smoothstep(TRUE_SCALE_NEAR_M, TOY_SCALE_FAR_M, d_eye);
        // near the eye the volumes burn instead (`update_volumes`): the
        // tongues shrink away as they come
        // with the screen's volumes on (`fire_volumetric`), they draw every
        // flame, near and far (user, 2026-11-05: «voglio tutte le fiamme
        // volumetriche»)
        let mut keep = if crate::fire_volumetric::on() {
            0.0
        } else if view.volumes.is_some() {
            smoothstep(VOL_FULL_M, VOL_END_M, d_eye)
        } else {
            1.0
        };
        // From afar one tongue a cell (2026-11-05): seven overlapping
        // sheets at toy scale drew a field of saturated cones; one, wider,
        // draws the front as a ragged line of fire.
        if c.rank > 0 {
            keep *= 1.0 - smoothstep(FAR_TONGUES_M, FAR_TONGUES_M * 1.4, d_eye);
        }
        if keep <= 0.001 {
            continue;
        }
        let mut h_m = c.flame_m * (c.true_m / c.flame_m + (1.0 - c.true_m / c.flame_m) * toy) * flicker * c.ratio * keep;
        let mut lift = 0.0;
        // Crowning, drawn only: now and then a tongue over tall trees
        // climbs into the canopy and roars well above the surface fire.
        // Each tongue has its own slow cycle, so flare-ups come and go
        // at random.
        if c.crown_h > 1.0 && c.flaming > 0.3 {
            let cyc = (t * 0.11 + c.crown_cyc).fract();
            // up close the plants' own flames do the crowning (`plant_torches`)
            let env = if cyc < 0.22 { (std::f32::consts::PI * cyc / 0.22).sin() } else { 0.0 } * toy;
            h_m *= 1.0 + (c.crown_h - 1.0) * env * c.flaming;
            // The tongue stands on the crown, not on the trunk's foot:
            // the shader already burns the foliage there (`burn_plant`).
            lift = env * c.flaming * c.crown_lift;
        }
        // wide enough that neighbours overlap into a wall (the licks are cut
        // in the shader, `flame.wesl`); wider still where it stands alone
        let half_w = (h_m * (0.32 + 0.1 * smoothstep(FAR_TONGUES_M, FAR_TONGUES_M * 1.4, d_eye))).min(max_half_w);
        let heat = c.heat;
        flames.tongue(
            c.foot + Vec3::Y * lift,
            right,
            h_m,
            half_w,
            t,
            c.phase,
            [3.6 * heat, 1.55 * heat, 0.28 * heat, 1.0],
            [2.0 * heat, 0.50 * heat, 0.07 * heat, 1.0],
        );
    }

    if let Some(veg) = vegetation.as_deref().filter(|_| !crate::fire_volumetric::on()) {
        let mut torches = std::mem::take(&mut view.torches);
        gather_torches(veg, eye, &mut torches);
        plant_torches(&mut flames, &torches, eye, right, t);
        view.torches = torches;
    }

    // The plume is there when the fire is (2026-11-05): after a restart or
    // a jump of the sim the puffs start from nothing and the column took a
    // minute to build; from an empty state they are rolled forward first.
    if view.smoke.is_empty() && view.smoke_volumes.is_none() && !crate::fire_volumetric::on() && !sim.fire.active_cells().is_empty() {
        for _ in 0..PREROLL_STEPS {
            step_particles(&mut view, &sim, PREROLL_DT_S, now);
        }
    }
    step_particles(&mut view, &sim, dt, now);

    // Drama where the fire meets people: a home the books count as lost burns
    // as a column of flame with its own smoke and sparks (people react in `people::bob_people`). Read from the books, so it cannot
    // disagree with the end card (finding 43).
    let burning = buildings.burning(now);
    for (i, (pos, age)) in burning.iter().enumerate() {
        let ground = scn.terrain.height_at(*pos);
        // Roars up in the first minute, then settles to a long burn-down.
        let intensity = (1.0 - (*age - 60.0).max(0.0) / 400.0).clamp(0.25, 1.0);
        for k in 0..4u64 {
            let h = hash01(i as u64 * 977 + k * 131);
            let phase = h * 30.0;
            let flicker = 0.6 + 0.4 * (t * 7.0 + phase).sin() * (t * 3.1 + phase * 1.7).cos();
            let h_m = (38.0 + 30.0 * h) * flicker * intensity;
            let half_w = h_m * 0.38;
            let off = Vec3::new((h - 0.5) * 16.0, 0.0, (hash01(i as u64 * 31 + k) - 0.5) * 16.0);
            let sway = (t * 3.2 + phase).sin() * h_m * 0.18;
            let _ = sway;
            flames.tongue(
                Vec3::new(pos.x, ground, -pos.y) + off,
                right,
                h_m,
                half_w,
                t,
                phase,
                [5.0 * intensity, 2.0 * intensity, 0.35 * intensity, 1.0],
                [2.6 * intensity, 0.6 * intensity, 0.08 * intensity, 1.0],
            );
        }
        // Dark smoke column and a steady rain of sparks.
        if view.smoke.len() < MAX_SMOKE && hash01(view.seed.wrapping_add(i as u64) ^ (t * 60.0) as u64) < dt * 4.5 {
            view.seed = view.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let phase = hash01(view.seed);
            view.smoke.push(Particle {
                pos: Vec3::new(pos.x, ground + 30.0, -pos.y),
                vel: Vec3::Y * 12.0,
                age: 0.0,
                life: 18.0 + phase * 10.0,
                size: 14.0 + 10.0 * intensity,
                phase,
                flare: false,
            });
        }
        if view.embers.len() < MAX_EMBERS && hash01(view.seed ^ (i as u64 * 7) ^ (t * 90.0) as u64) < dt * 3.75 {
            view.seed = view.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let phase = hash01(view.seed);
            let a = phase * std::f32::consts::TAU;
            view.embers.push(Particle {
                pos: Vec3::new(pos.x, ground + 25.0, -pos.y),
                vel: Vec3::new(a.cos() * 6.0, 14.0 + 10.0 * phase, a.sin() * 6.0),
                age: 0.0,
                life: 3.0 + 2.0 * phase,
                size: 0.8 + phase * 0.8,
                phase,
                flare: false,
            });
        }
    }

    let fogger = PuffFog::new(&fog, &suns, cam.translation);
    // the sun, for the puffs' shading: toward it
    let sun = fogger.lights.first().map_or(Vec3::Y, |l| l.0);
    let shadow = PlumeShadow::new(&view.smoke, sun);
    // Back to front (2026-11-05): blended in spawn order, the dark puffs
    // inside the plume painted over its lit shell.
    let mut order: Vec<(f32, usize)> = view.smoke.iter().enumerate().map(|(i, p)| (p.pos.distance_squared(eye), i)).collect();
    order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let mut smoke = QuadBuilder::new(true, 4 * view.smoke.len());
    for &(_, i) in &order {
        let puff = &view.smoke[i];
        let k = (puff.age / puff.life).clamp(0.0, 1.0);
        // Grows as it disperses, darkest and densest near the fire.
        let size = puff_size(puff, k);
        // A wildfire's smoke (2026-11-05): brown-grey where it leaves the
        // flames, grey, then pale as it rises and spreads. Albedo, not
        // brightness: the light is added below. (The 2026-11-04 shade of
        // 0.22 drew the plume so dark that over the scar it vanished.)
        let albedo = Vec3::new(0.52, 0.44, 0.37).lerp(Vec3::new(0.86, 0.86, 0.88), k.powf(0.6));
        // Fade in fast, out slowly: a puff should never pop into existence.
        // Hundreds of puffs overlap: thin each, denser while young.
        // Dense enough to be a plume from the play camera; thin next to
        // the camera, so a view dropped into the smoke is not a wall.
        let d_eye = eye.distance(puff.pos);
        let alpha = puff_alpha(k) * smoothstep(20.0, 90.0, d_eye);
        // From afar the puffs are a few pixels and the plume a veil: they
        // grow with the distance (up to twice), as the far trees do.
        let size = size * (1.0 + 0.6 * smoothstep(300.0, 1500.0, d_eye));
        let spin = puff.phase * std::f32::consts::TAU + k * 0.6;
        let (s, c) = spin.sin_cos();
        let r = (right * c + up * s) * size;
        let u = (up * c - right * s) * size;
        let (gain, offset) = fogger.at(puff.pos);
        // lit orange from below by the flames, for its first seconds
        let lit = (1.0 - k / 0.18).max(0.0);
        let offset = offset + Vec3::new(0.55, 0.20, 0.03) * lit * lit * alpha;
        // Lit by the sun through the plume itself (2026-11-05): the smoke
        // between this puff and the sun shades it (`PlumeShadow`), so the
        // top and the sunward face of the column are bright, the underside
        // and the lee dark; the sky lights the rest. Then each puff is a
        // ball: a pseudo-normal per corner, leaning toward the eye, shades
        // its own side away from the sun. The flames are drawn after the
        // smoke (`FireMaterial::depth_bias`), so it does not bury them.
        let shade = shadow.at(puff.pos);
        let to_eye = (eye - puff.pos).normalize_or_zero();
        let rn = r.normalize_or_zero();
        let un = u.normalize_or_zero();
        let corner = |sx: f32, sy: f32| {
            let n = (rn * sx + un * sy + to_eye * 0.9).normalize_or_zero();
            let ndl = n.dot(sun).max(0.0);
            let lit = albedo * gain * (0.50 + 0.50 * shade * (0.35 + 0.65 * ndl));
            [lit.x, lit.y, lit.z, alpha]
        };
        // The normal slot carries the fog's additive part (`smoke.wgsl`).
        smoke.billboard_corners(puff.pos, r, u, [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)], offset.into());
    }

    // Embers are plain round particles (`flame.wgsl`, mode 2). Colour
    // channels carry no colour: r = age share,
    // g = per-ember phase, b = gain (negative for the landing flash).
    let mut sparks = QuadBuilder::new(false, 4 * view.embers.len());
    for ember in &view.embers {
        let k = (ember.age / ember.life).clamp(0.0, 1.0);
        // never smaller than ~2 px, or a far ember shimmers in and out
        let min_size = eye.distance(ember.pos) * 0.0017;
        if ember.flare {
            // A quick, bright pop where a firebrand has just landed on
            // unburnt fuel — sharp attack, quick decay, not a fade like an
            // ordinary ember: this is a spot-ignition risk lighting up, not
            // embers cooling.
            let pulse = (1.0 - k).powf(0.5) * (1.0 - (k * 3.0 - 1.0).max(0.0));
            let size = (ember.size * (1.0 + 1.5 * (1.0 - k))).max(min_size);
            let c = [k, ember.phase, -pulse, 1.0];
            sparks.billboard(ember.pos, right * size, up * size, 1.0, c, c, [0.0, 1.0, 0.0]);
            continue;
        }
        // a real ember is a few centimetres: drawn a metre wide it reads from
        // the play camera, but next to the camera it was a pale disc
        let d_eye = eye.distance(ember.pos);
        let near_cap = d_eye * 0.003;
        let size = (ember.size * (1.0 - 0.35 * k)).min(near_cap).max(min_size);
        // dimmer from afar (2026-11-05): from the play camera five hundred
        // embers at full gain were a swarm of white fireflies
        let gain = (0.9 + 0.5 * ember.phase) * (1.0 - 0.85 * smoothstep(300.0, 1000.0, d_eye));
        let c = [k, ember.phase, gain, 1.0];
        // An oval: a little longer along the direction of travel, seen from
        // the camera. The shader draws the round-edged shape inside it.
        let to_eye = (eye - ember.pos).normalize_or_zero();
        let speed = ember.vel.length();
        let dir = if speed > 0.5 { ember.vel / speed } else { Vec3::Y };
        let mut across = dir.cross(to_eye).normalize_or_zero();
        let mut along = across.cross(to_eye).normalize_or_zero();
        if across == Vec3::ZERO || along == Vec3::ZERO {
            across = right;
            along = up;
        }
        let stretch = 1.0 + (0.04 * speed).min(0.7);
        sparks.billboard(ember.pos, across * size, along * size * stretch, 1.0, c, c, [0.0, 1.0, 0.0]);
    }

    view.last_flame_verts = flames.verts;
    let view = &mut *view;
    for (target, built, shape) in [(&mut view.flames, flames, TONGUE), (&mut view.smoke_mesh, smoke, BILLBOARD), (&mut view.sparks, sparks, BILLBOARD)] {
        let items = built.verts as u32 / shape.verts;
        target.write(&mut meshes, &mut frame, built.bytes, items);
    }
    update_volumes(view, cam, projection, &mut meshes, &mut frame);
    update_smoke_volumes(view, &sim, now, key, cam, projection, &mut meshes, &mut frame, &mut smoke_volume_materials);
}

/// The wind's drift for the smoke (`SmokeParams::wind`): Bevy x and z in
/// m/s, the speed, and the plume's lateral margin, a share of its drift
/// (`sway` in `smoke_volume.wesl`), never under 60 m.
pub(crate) fn smoke_wind(sim: &Sim) -> Vec4 {
    let weather = sim.fire.weather();
    // `wind_dir_deg` is the bearing the wind blows *from* (Bevy -Z is north)
    let from = (weather.wind_dir_deg as f32).to_radians();
    let wind_ms = weather.wind_speed_kmh as f32 / 3.6;
    let margin = (wind_ms * 6.0).max(60.0);
    Vec4::new(-from.sin() * wind_ms, from.cos() * wind_ms, wind_ms, margin)
}

/// The box of a smoke block (`smoke_volume.wesl` computes the same): its
/// footprint sheared downwind to the plume's top, with the margin.
fn smoke_box(b: &SmokeBlock, half: f32, wind: Vec4) -> VolumeBox {
    let shift = Vec2::new(wind.x, wind.y) * (SMOKE_TOP_M / SMOKE_RISE_MS);
    let foot = Vec2::new(b.x, b.z);
    let lo = foot.min(foot + shift) - half - wind.w;
    let hi = foot.max(foot + shift) + half + wind.w;
    let (bottom, top) = (b.g_lo - 5.0, b.g_hi + SMOKE_TOP_M);
    VolumeBox {
        centre: Vec3::new((lo.x + hi.x) * 0.5, (bottom + top) * 0.5, (lo.y + hi.y) * 0.5),
        half: Vec3::new((hi.x - lo.x) * 0.5, (top - bottom) * 0.5, (hi.y - lo.y) * 0.5),
        flame_m: 0.0,
        heat: 0.0,
    }
}

/// The smoke volumes: the blocks of smoking ground follow the sim; their
/// boxes are written every frame, the farthest first, since they overlap
/// and blend in the order they are drawn. A box around the eye goes as a
/// quad across the view (`update_volumes`).
#[allow(clippy::too_many_arguments)]
fn update_smoke_volumes(
    view: &mut FireView,
    sim: &Sim,
    now: f32,
    key: (u64, u32),
    cam: &Transform,
    projection: &Projection,
    meshes: &mut Assets<Mesh>,
    frame: &mut StreamedFrame,
    materials: &mut Option<ResMut<Assets<SmokeVolumeMaterial>>>,
) {
    let Some((mesh, mat)) = view.smoke_volumes.as_mut() else { return };
    let scn = &sim.scn;
    let w = scn.world;
    let cs = w.cellsize;
    let side = SMOKE_BLOCK as f32 * cs;
    let wind = smoke_wind(sim);
    // the material is touched only when the wind changes (a touch sends
    // the uniform again)
    if let Some(m) = materials.as_mut() {
        if m.get(&*mat).is_some_and(|m| m.params.wind != wind) {
            if let Some(mut m) = m.get_mut(&*mat) {
                m.params.wind = wind;
            }
        }
    }
    if view.smoke_key != Some(key) {
        view.smoke_key = Some(key);
        let k = SMOKE_BLOCK as i32;
        let mut blocks = std::collections::HashSet::new();
        for cell in sim.fire.active_cells() {
            let arrival = sim.fire.arrival_time(*cell).unwrap_or(now as i32) as f32;
            if now - arrival > SMOKE_S {
                continue;
            }
            blocks.insert(((cell.row as i32).div_euclid(k), (cell.col as i32).div_euclid(k)));
        }
        let mut keys: Vec<_> = blocks.into_iter().collect();
        keys.sort();
        view.smoke_blocks.clear();
        for (br, bc) in keys {
            let x0 = bc as f32 * side;
            let y1 = w.height_m - br as f32 * side;
            let (mut g_lo, mut g_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for i in 0..=k {
                for j in 0..=k {
                    let h = scn.terrain.height_at(Pos { x: x0 + i as f32 * cs, y: y1 - j as f32 * cs });
                    g_lo = g_lo.min(h);
                    g_hi = g_hi.max(h);
                }
            }
            view.smoke_blocks.push(SmokeBlock { x: x0 + side * 0.5, z: -(y1 - side * 0.5), g_lo, g_hi });
        }
    }
    let eye = cam.translation;
    let half = side * 0.5;
    let mut order: Vec<(VolumeBox, &SmokeBlock)> = view.smoke_blocks.iter().map(|b| (smoke_box(b, half, wind), b)).collect();
    order.sort_by(|a, b| b.0.centre.distance_squared(eye).total_cmp(&a.0.centre.distance_squared(eye)));
    let mut built = QuadBuilder::new(true, 8 * order.len());
    for (b, blk) in &order {
        let quad = (eye - b.centre).abs().cmplt(b.half + 1.0).all().then(|| near_quad(cam, projection));
        built.smoke_volume(b, blk, half, quad);
    }
    let items = built.verts as u32 / BOX.verts;
    mesh.write(meshes, frame, built.bytes, items);
}

/// A quad just inside the near plane, across the whole view, for a volume
/// the eye is in (`update_volumes`).
fn near_quad(cam: &Transform, projection: &Projection) -> [Vec3; 4] {
    let (fov, aspect, near) = match projection {
        Projection::Perspective(p) => (p.fov, p.aspect_ratio, p.near),
        _ => (1.0, 1.6, 0.5),
    };
    let d = near * 3.0;
    let half_h = d * (fov * 0.5).tan() * 1.3;
    let half_w = half_h * aspect * 1.3;
    let right = cam.rotation * Vec3::X * half_w;
    let up = cam.rotation * Vec3::Y * half_h;
    let centre = cam.translation + cam.rotation * Vec3::NEG_Z * d;
    [centre - right - up, centre + right - up, centre + right + up, centre - right + up]
}

/// The flame volumes' boxes for the fire's state now (`VolumeBox`): the
/// flaming band tiled by blocks of [`VOL_BLOCK`] cells a side, a box per
/// block that holds a flaming cell or touches one, as tall as the ground
/// under it varies plus the tallest flame it carries. Boxes abut and do
/// not overlap: the shader samples the fire map and the noise in world
/// space, so the flame runs across their faces without a seam, and no
/// band is counted twice. Cells are read as `update_flames` reads them for
/// the tongues, so the volumes stand where the tongues would.
fn build_boxes(sim: &Sim, now: f32, eye: Vec3, out: &mut Vec<VolumeBox>) {
    out.clear();
    let scn = &sim.scn;
    let w = scn.world;
    let cs = w.cellsize;
    let k = VOL_BLOCK as i32;
    let mut blocks: std::collections::HashMap<(i32, i32), (f32, f32)> = std::collections::HashMap::new();
    for cell in sim.fire.active_cells() {
        let arrival = sim.fire.arrival_time(*cell).unwrap_or(now as i32) as f32;
        let flaming = (1.0 - (now - arrival) / FLAMING_S).clamp(0.0, 1.0);
        if flaming < 0.2 {
            continue;
        }
        let fli = sim.fire.cell_intensity(*cell);
        let flame_m = vol_flame_m(fli);
        let heat = (fli / 2500.0).clamp(0.6, 1.1);
        let (r, c) = (cell.row as i32, cell.col as i32);
        for dr in -1..=1 {
            for dc in -1..=1 {
                let key = ((r + dr).div_euclid(k), (c + dc).div_euclid(k));
                let e = blocks.entry(key).or_insert((0.0, 0.0));
                e.0 = e.0.max(flame_m);
                e.1 = e.1.max(heat);
            }
        }
    }
    let mut keys: Vec<_> = blocks.into_iter().collect();
    keys.sort_by_key(|(key, _)| *key);
    for ((br, bc), (flame_m, heat)) in keys {
        if br < 0 || bc < 0 || (br * k) as usize >= w.fire_rows || (bc * k) as usize >= w.fire_cols {
            continue;
        }
        let side = k as f32 * cs;
        let x0 = bc as f32 * side;
        let y1 = w.height_m - br as f32 * side;
        let y0 = y1 - side;
        // the ground's range in the block, sampled every half cell
        let (mut g_lo, mut g_hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for i in 0..=2 * k {
            for j in 0..=2 * k {
                let h = scn.terrain.height_at(Pos { x: x0 + i as f32 * cs * 0.5, y: y0 + j as f32 * cs * 0.5 });
                g_lo = g_lo.min(h);
                g_hi = g_hi.max(h);
            }
        }
        let bottom = g_lo - 1.0;
        // room for the flames as the shader grows them with distance toward
        // the toy scale (`grow` in `fire_volume.wesl`), and no more: a box
        // of empty air above the flames costs fragments
        let foot = Vec3::new(x0 + side * 0.5, (g_lo + g_hi) * 0.5, -(y0 + y1) * 0.5);
        let grow = 1.0 + 1.5 * smoothstep(150.0, VOL_END_M, eye.distance(foot));
        let top = g_hi + flame_m * grow * 1.08 + 1.0;
        out.push(VolumeBox {
            centre: Vec3::new(x0 + side * 0.5, (bottom + top) * 0.5, -(y0 + y1) * 0.5),
            half: Vec3::new(side * 0.5, (top - bottom) * 0.5, side * 0.5),
            flame_m,
            heat,
        });
    }
}

/// Write the flame volumes' mesh when the boxes changed (a new sim state,
/// the eye moved) or the eye entered or left a box. A box around the eye has no front
/// face to draw: it goes as a quad just inside the near plane that spans
/// the view, and the shader marches from the eye.
fn update_volumes(view: &mut FireView, cam: &Transform, projection: &Projection, meshes: &mut Assets<Mesh>, frame: &mut StreamedFrame) {
    let Some(volumes) = view.volumes.as_mut() else { return };
    let eye = cam.translation;
    let inside = view.boxes.iter().position(|b| (eye - b.centre).abs().cmplt(b.half + 1.0).all());
    if view.boxes_written == Some(inside) {
        return;
    }
    view.boxes_written = Some(inside);
    let mut built = QuadBuilder::new(true, 8 * view.boxes.len());
    for (i, b) in view.boxes.iter().enumerate() {
        let quad = (Some(i) == inside).then(|| near_quad(cam, projection));
        built.volume(b, quad);
    }
    let items = built.verts as u32 / BOX.verts;
    volumes.write(meshes, frame, built.bytes, items);
}


fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// The burning plants near `eye`, at most [`MAX_TORCHES`], the nearest.
fn gather_torches(veg: &crate::vegetation::Vegetation, eye: Vec3, out: &mut Vec<crate::vegetation::torch::Torch>) {
    out.clear();
    veg.torches(eye, TORCH_RADIUS_M, out);
    if out.len() > MAX_TORCHES {
        out.select_nth_unstable_by(MAX_TORCHES, |a, b| a.foot.distance_squared(eye).total_cmp(&b.foot.distance_squared(eye)));
        out.truncate(MAX_TORCHES);
    }
}

/// Flames on the burning plants (2026-11-04): the fire climbs a plant from
/// its foot at [`CLIMB_S_PER_M`] and each point of it burns for
/// [`CROWN_SHARE`] of the plant's flaming time, as the shader chars it
/// (`plant_common.wesl`, `place`). So at any instant a band of the plant
/// is alight, from the trunk up into the crown and then out at the top:
/// the flames stand on that band, and where it holds the crown's foliage
/// they torch well above the plant, as a crown fire does.
fn plant_torches(flames: &mut QuadBuilder, torches: &[crate::vegetation::torch::Torch], eye: Vec3, right: Vec3, t: f32) {
    let burn_s = crate::vegetation::PLANT_FLAMING_S * CROWN_SHARE;
    let back = right.cross(Vec3::Y);
    for p in torches {
        let h = p.height.max(0.5);
        // the alight band [y0, y1], metres above the foot
        let y1 = (p.age / CLIMB_S_PER_M).min(h);
        let y0 = ((p.age - burn_s * FLAME_SHARE) / CLIMB_S_PER_M).max(0.0);
        if y1 <= y0 {
            continue;
        }
        // the crown: where the model's foliage starts
        let crown_lo = p.crown_lo.min(0.9 * h);
        let crown = h - crown_lo;
        let leafy = (y1.min(h) - y0.max(crown_lo)).max(0.0) / crown;
        // lit over a few seconds
        let onset = smoothstep(0.0, 6.0, p.age);
        let band = y1 - y0;
        let d = eye.distance(p.foot);
        // a shrub's flames from afar are the front's own: alone, its single
        // tongue drew a candle dotted over the burnt ground (2026-11-04)
        if h < 3.0 && d > 250.0 {
            continue;
        }
        // one tongue from afar, a crown of them up close
        let n = if d > 600.0 {
            1
        } else if d > 300.0 {
            2
        } else {
            (p.width / 2.2).round().clamp(2.0, 6.0) as usize
        };
        let heat = (0.45 + 0.85 * leafy) * onset;
        let key = (p.seed * 1.0e6) as u64;
        for k in 0..n {
            let hk = hash01(key * 31 + k as u64 * 7919);
            let hk2 = hash01(key * 97 + k as u64 * 104_729);
            let hk3 = hash01(key * 13 + k as u64 * 3_571);
            let phase = p.seed * 40.0 + k as f32 * 2.7;
            let main = k == 0;
            // where on the band the tongue stands, and how wide the plant is there
            let fy = y0 + band * if main { 0.08 } else { 0.05 + 0.6 * hk2 };
            let spread = if fy > crown_lo { p.width * 0.42 } else { p.width * 0.08 };
            let side = right * (hk - 0.5) * 2.0 * spread * if main { 0.3 } else { 1.0 } + back * (hk3 - 0.5) * spread;
            // the band itself, and above it a torch as long as the crown alight
            let flame = if main {
                band * 0.9 + 1.0 * leafy * crown
            } else {
                (y1 - fy) * (0.6 + 0.4 * hk3) + 0.45 * leafy * crown
            };
            let flicker = 0.85 + 0.15 * (t * 6.1 + phase).sin() * (t * 2.9 + phase * 1.3).cos();
            let h_m = (flame * flicker * onset).max(0.4);
            // narrow tongues: a burning crown is many of them, not one sheet
            let half_w = (h_m * 0.22).min(p.width * if main { 0.2 } else { 0.13 }).max(0.25);
            // the foot fades in (alpha 0 at the foot), the tongue stands in the air
            flames.tongue(
                p.foot + Vec3::Y * fy + side,
                right,
                h_m,
                half_w,
                t,
                phase,
                [3.6 * heat, 1.55 * heat, 0.28 * heat, 0.0],
                [2.0 * heat, 0.50 * heat, 0.07 * heat, 1.0],
            );
        }
    }
}

/// The plants' flames in the plant lab (`veg_lab`, `KIOSK_PIANTE`): the same
/// [`plant_torches`] the island draws, with no fire map and no front.
#[cfg(not(target_arch = "wasm32"))]
pub struct LabTorchesPlugin;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct LabTorches {
    flames: Streamed,
    torches: Vec<crate::vegetation::torch::Torch>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Plugin for LabTorchesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((crate::fire_shader::FireShaderPlugin, crate::streamed_mesh::StreamedMeshPlugin))
            .add_systems(Startup, lab_torches_setup)
            .add_systems(PostUpdate, lab_torches.before(bevy::transform::TransformSystems::Propagate));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn lab_torches_setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut fire_materials: ResMut<Assets<FireMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let flames = Streamed::new(&mut meshes, TONGUE, false);
    let mat = fire_materials.add(FireMaterial { texture: images.add(textures::flame_flipbook(0)), mode: Vec4::X });
    commands.spawn((
        (Mesh3d(flames.handle.clone()), MeshMaterial3d(mat)),
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
    ));
    commands.insert_resource(LabTorches { flames, torches: Vec::new() });
}

#[cfg(not(target_arch = "wasm32"))]
fn lab_torches(
    time: Res<Time>,
    veg: Res<crate::vegetation::Vegetation>,
    camera: Query<&Transform, With<Camera3d>>,
    mut lab: ResMut<LabTorches>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut frame: ResMut<StreamedFrame>,
) {
    let Ok(cam) = camera.single() else { return };
    let mut right = cam.rotation * Vec3::X;
    right.y = 0.0;
    let right = right.normalize_or_zero();
    let lab = &mut *lab;
    gather_torches(&veg, cam.translation, &mut lab.torches);
    let mut flames = QuadBuilder::new(false, 16 * 4 * lab.torches.len());
    plant_torches(&mut flames, &lab.torches, cam.translation, right, time.elapsed_secs());
    let items = flames.verts as u32 / TONGUE.verts;
    lab.flames.write(&mut meshes, &mut frame, flames.bytes, items);
}

/// The scene's distance fog as `colour * gain + offset`, evaluated on the CPU
/// for one puff. `bevy_pbr::fog::atmospheric_fog` is affine in the colour it
/// fogs, and across a puff the distance and view direction barely change, so
/// one evaluation per puff replaces a per-pixel one (a light loop and a pow
/// per fragment, about a fifth of the smoke's GPU time). Only the atmospheric
/// falloff is mirrored, the one `sky.rs` and `tactical.rs` use; with any other
/// the puffs go unfogged.
struct PuffFog {
    eye: Vec3,
    /// (extinction, inscattering, base colour, light colour x alpha) when fog is on.
    atmo: Option<(Vec3, Vec3, Vec4, Vec4, f32)>,
    /// Direction to each directional light, and its colour x exposure.
    lights: Vec<(Vec3, Vec3)>,
}

impl PuffFog {
    fn new(
        fog: &Query<(&DistanceFog, Option<&Exposure>), With<Camera3d>>,
        suns: &Query<(&GlobalTransform, &DirectionalLight)>,
        eye: Vec3,
    ) -> Self {
        let mut out = PuffFog { eye, atmo: None, lights: Vec::new() };
        let Ok((f, exposure)) = fog.single() else {
            return out;
        };
        let FogFalloff::Atmospheric { extinction, inscattering } = f.falloff else {
            return out;
        };
        let exposure = exposure.copied().unwrap_or_default().exposure();
        let base = LinearRgba::from(f.color);
        let dl = LinearRgba::from(f.directional_light_color);
        out.atmo = Some((
            extinction,
            inscattering,
            Vec4::new(base.red, base.green, base.blue, base.alpha),
            Vec4::new(dl.red, dl.green, dl.blue, dl.alpha),
            f.directional_light_exponent,
        ));
        for (t, l) in suns.iter() {
            let c = LinearRgba::from(l.color) * l.illuminance * exposure;
            out.lights.push((t.back().into(), Vec3::new(c.red, c.green, c.blue)));
        }
        out
    }

    /// (gain, offset) for a puff at `p`.
    fn at(&self, p: Vec3) -> (Vec3, Vec3) {
        let Some((be, bi, base, dl, exponent)) = self.atmo else {
            return (Vec3::ONE, Vec3::ZERO);
        };
        let v = p - self.eye;
        let d = v.length();
        let dir = v / d.max(1e-3);
        let mut fog = Vec3::new(base.x, base.y, base.z);
        if dl.w > 0.0 {
            let mut scatter = Vec3::ZERO;
            for (to_light, colour) in &self.lights {
                scatter += dir.dot(*to_light).max(0.0).powf(exponent) * *colour;
            }
            fog += scatter * Vec3::new(dl.x, dl.y, dl.z) * dl.w;
        }
        let ext = Vec3::ONE - Vec3::new((-d * be.x).exp(), (-d * be.y).exp(), (-d * be.z).exp());
        let ins = Vec3::ONE - Vec3::new((-d * bi.x).exp(), (-d * bi.y).exp(), (-d * bi.z).exp());
        (Vec3::ONE - ext * base.w, fog * ins * base.w)
    }
}

/// A puff's cover at `k` of its life: in fast, out slowly (a puff should
/// never pop into existence); hundreds overlap, so each is thin, denser
/// while young.
fn puff_alpha(k: f32) -> f32 {
    (k * 4.0).min(1.0) * (1.0 - k).powf(1.4) * (0.55 - 0.25 * k)
}

/// A puff's size at `k` of its life: grows as it disperses.
fn puff_size(p: &Particle, k: f32) -> f32 {
    p.size * (0.7 + 2.3 * k)
}

/// The plume's shadow on itself (2026-11-05): the puffs' optical depth
/// binned on a coarse grid, read along the sun from each puff. A column of
/// smoke is lit on top and on the side toward the sun and dark under and
/// behind: without this the puffs drew a flat beige mass.
struct PlumeShadow {
    cells: std::collections::HashMap<IVec3, f32>,
    sun: Vec3,
}

impl PlumeShadow {
    /// Side of a bin (m), taps toward the sun, and their spacing (m).
    const CELL_M: f32 = 40.0;
    const TAPS: usize = 4;
    const TAP_M: f32 = 45.0;

    fn new(puffs: &[Particle], sun: Vec3) -> Self {
        let mut cells = std::collections::HashMap::with_capacity(puffs.len() / 2);
        for p in puffs {
            let k = (p.age / p.life).clamp(0.0, 1.0);
            // what a puff puts in the way of the light: its cover (as drawn,
            // the texture's mean alpha is ~0.3) over its area, against the
            // bin's
            let size = puff_size(p, k);
            let w = puff_alpha(k) * 0.3 * (size * size) / (Self::CELL_M * Self::CELL_M);
            *cells.entry((p.pos / Self::CELL_M).floor().as_ivec3()).or_insert(0.0) += w;
        }
        PlumeShadow { cells, sun }
    }

    /// The sun's light reaching `p` through the plume, 0..1.
    fn at(&self, p: Vec3) -> f32 {
        let mut depth = 0.0;
        for i in 1..=Self::TAPS {
            let q = p + self.sun * (i as f32 * Self::TAP_M);
            if let Some(w) = self.cells.get(&(q / Self::CELL_M).floor().as_ivec3()) {
                depth += w;
            }
        }
        // soft: smoke scatters light into its own shadow
        (-depth * 0.7).exp()
    }
}

/// Advance smoke and embers, and top both up from the active front.
fn step_particles(view: &mut FireView, sim: &Sim, dt: f32, now: f32) {
    let scn = &sim.scn;
    let weather = sim.fire.weather();
    // `wind_dir_deg` is the bearing the wind blows *from*, so drift is the
    // opposite bearing. (Bevy -Z is north.)
    let from = (weather.wind_dir_deg as f32).to_radians();
    let wind_ms = weather.wind_speed_kmh as f32 / 3.6;
    let drift = Vec3::new(-from.sin(), 0.0, from.cos()) * wind_ms;

    for puff in &mut view.smoke {
        puff.age += dt;
        // Buoyancy dies off as the plume cools and entrains air, after which
        // the puff simply goes where the wind goes.
        let k = (puff.age / puff.life).clamp(0.0, 1.0);
        // The column of a hot front rises hundreds of metres before it
        // bends over (2026-11-05, from 7 (1 - k)^2, a dome 100 m high the
        // play camera looked down through): the puff's size carries the
        // plume's strength (`size` at the spawn site).
        let plume = ((puff.size - 20.0) / 22.0).clamp(0.3, 3.0);
        let buoyancy = Vec3::Y * ((7.0 + 5.0 * plume) * (1.0 - k).powf(1.5));
        let turbulence = Vec3::new(
            noise(puff.pos.x * 0.01, puff.age * 0.4 + puff.phase, 3) - 0.5,
            0.0,
            noise(puff.pos.z * 0.01, puff.age * 0.4 + puff.phase, 5) - 0.5,
        ) * wind_ms
            * 0.6;
        puff.vel = puff.vel.lerp(drift + buoyancy + turbulence, dt * 1.2);
        puff.pos += puff.vel * dt;
    }
    view.smoke.retain(|p| p.age < p.life);

    for ember in &mut view.embers {
        ember.age += dt;
        if ember.flare {
            continue;
        }
        // A real firebrand's horizontal speed barely changes in flight — the
        // wind carries it — so only gravity acts on the vertical component;
        // the horizontal speed was chosen at launch to cover the modelled
        // spotting distance in the modelled travel time (see the spawn
        // site), and damping it away early would strand it short.
        ember.vel.y -= 9.0 * dt;
        // Tumbling in the plume's eddies: a zero-mean flutter on top of the
        // ballistic path, so the landing point is unchanged but no ember
        // flies on rails.
        let flutter = Vec3::new(
            noise(ember.pos.x * 0.07, ember.age * 1.7 + ember.phase * 9.0, 11) - 0.5,
            noise(ember.pos.y * 0.07, ember.age * 2.3 + ember.phase * 5.0, 12) - 0.5,
            noise(ember.pos.z * 0.07, ember.age * 1.7 + ember.phase * 7.0, 13) - 0.5,
        ) * 7.0;
        ember.pos += (ember.vel + flutter) * dt;
    }
    // A firebrand that reaches ground level on unburnt, burnable fuel is
    // exactly what the core's own spotting model is throwing ahead of the
    // front — flash it there instead of silently despawning it like a spent,
    // harmless spark.
    let mut flares = Vec::new();
    for e in &view.embers {
        if e.flare {
            continue;
        }
        let ground = scn.terrain.height_at(Pos {
            x: e.pos.x,
            y: -e.pos.z,
        });
        let dying = e.age >= e.life || e.pos.y <= ground + 0.4;
        if !dying {
            continue;
        }
        let landing = Pos {
            x: e.pos.x,
            y: -e.pos.z,
        };
        if !scn.world.contains(landing) {
            continue;
        }
        let cell = scn.world.cell_of(landing);
        if scn.is_burnable(cell) && sim.fire.cell_state(cell) == CellFire::Unburnt {
            flares.push(Particle {
                pos: Vec3::new(e.pos.x, ground + 0.6, e.pos.z),
                vel: Vec3::ZERO,
                age: 0.0,
                life: 1.1,
                size: 2.6,
                phase: e.phase,
                flare: true,
            });
        }
    }
    view.embers.retain(|e| {
        e.age < e.life
            && (e.flare
                || e.pos.y
                    > scn.terrain.height_at(Pos {
                        x: e.pos.x,
                        y: -e.pos.z,
                    }) + 0.4)
    });
    view.embers.extend(flares);

    let active = sim.fire.active_cells();
    if active.is_empty() {
        return;
    }

    // Spawn budgets scale with the front but are capped: a 20 000-cell fire
    // does not get 20 000 plumes, it gets a full sky.
    // Truncating `n * dt` per frame used to lose the fraction every frame,
    // so the faster the renderer the thinner the smoke; it is carried over.
    // the smoke volumes draw the front's smoke (`update_smoke_volumes`)
    // and so does the screen-space march (`fire_volumetric`)
    let puffs = if view.smoke_volumes.is_some() || crate::fire_volumetric::on() { 0.0 } else { 1.2 };
    let (smoke_owed, ember_owed) = (view.owed.0 + active.len() as f32 * dt * puffs, view.owed.1 + active.len() as f32 * dt * 0.375);
    view.owed = (smoke_owed.fract(), ember_owed.fract());
    let smoke_budget = (smoke_owed as usize).min(MAX_SMOKE.saturating_sub(view.smoke.len()));
    let ember_budget = (ember_owed as usize).min(MAX_EMBERS.saturating_sub(view.embers.len()));

    for i in 0..smoke_budget + ember_budget {
        view.seed = view.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let r = (view.seed >> 33) as usize;
        let cell = active[r % active.len()];
        let fli = sim.fire.cell_intensity(cell);
        let arrival = sim.fire.arrival_time(cell).unwrap_or(now as i32) as f32;
        let flaming = (1.0 - (now - arrival) / (FLAMING_S * 3.0)).clamp(0.0, 1.0);

        let centre = scn.world.centre_of(cell);
        let half = scn.world.cellsize * 0.5;
        let jx = hash01(view.seed) - 0.5;
        let jy = hash01(view.seed ^ 0xABCD) - 0.5;
        let p = Pos {
            x: centre.x + jx * 2.0 * half,
            y: centre.y + jy * 2.0 * half,
        };
        let ground = scn.terrain.height_at(p);
        let phase = hash01(view.seed ^ 0x1234);

        if i < smoke_budget {
            // Smouldering ground smokes too, but the column stands on the
            // front (2026-11-05): a cell long past it smokes a quarter as
            // often, so the plume rises from the flames and the scar
            // behind it carries a thin haze, not the same smoke.
            let plume = (fli / 1200.0).clamp(0.3, 3.0);
            if hash01(view.seed ^ 0x5A5A) > (0.25 + 0.75 * flaming) * (plume / 1.5).clamp(0.2, 1.0) {
                continue;
            }
            view.smoke.push(Particle {
                // just over the flames, so the smoke is theirs
                pos: Vec3::new(p.x, ground + 8.0 + 10.0 * flaming, -p.y),
                vel: Vec3::Y * (6.0 + 10.0 * plume * flaming) + drift * 0.3,
                age: 0.0,
                life: 30.0 + phase * 30.0,
                size: 20.0 + 22.0 * plume,
                phase,
                flare: false,
            });
        } else {
            // Only a cell with a real plume lofts embers.
            if fli < 400.0 {
                continue;
            }
            // Launch azimuth measured *from* the downwind direction, so
            // `alignment = cos(offset)` is 1 directly downwind — the same
            // quantity `compute_spotting` calls `(w_dir - angle).cos()`.
            let offset = phase * std::f32::consts::TAU;
            let wind_hat = if wind_ms > 0.05 {
                drift / wind_ms
            } else {
                Vec3::Z
            };
            let perp = Vec3::new(-wind_hat.z, 0.0, wind_hat.x);
            let (s, c) = offset.sin_cos();
            let launch_dir = wind_hat * c + perp * s;
            let alignment = c;

            // Median landing distance: linear in wind, ~intensity^(1/3) via
            // plume lofting, concentrated downwind. Jittered log-normally
            // about that median, same as the core.
            let d_median = SPOT_DISTANCE_REF_M
                * (weather.wind_speed_kmh as f32 / SPOT_WIND_REF_KMH).max(0.0)
                * (fli / SPOT_FLI_REF).max(0.001).powf(SPOT_FLI_EXPONENT);
            let directional = (SPOT_ANISOTROPY * (alignment - 1.0)).exp();
            let jitter = 0.55 + hash01(view.seed ^ 0x9E37) * 0.9;
            // Capped at the same 400 m the threat field uses for ember reach
            // (`ThreatField`, `fire::threat`) — the distance a real crown
            // fire throws brands can run past a kilometre, but this is a
            // particle the player watches fly, not a hazard sample.
            let distance = (d_median * directional * jitter).max(2.0).min(400.0);

            let transport_speed = (wind_ms * alignment.max(0.15)).max(0.5);
            let travel_time = (distance / transport_speed).clamp(2.0, 12.0);

            // Ballistic vertical launch: apex at travel_time / 2, back to
            // spawn height at travel_time, so a long throw arcs high and a
            // short one barely lifts off — the loft the core's model
            // attributes to plume convection (`H ~ I^(2/3)`) made visible.
            const GRAVITY: f32 = 9.0;
            let v_y0 = 0.5 * GRAVITY * travel_time;
            view.embers.push(Particle {
                pos: Vec3::new(p.x, ground + 3.0, -p.y),
                vel: launch_dir * (distance / travel_time) + Vec3::Y * v_y0,
                age: 0.0,
                life: travel_time,
                size: 0.7 + phase * 0.9,
                phase,
                flare: false,
            });
        }
    }
}

/// Hash to `[0, 1)`, for per-instance jitter that must be stable frame to
/// frame (a tongue that teleports every frame reads as noise, not fire).
fn hash01(x: u64) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tongue's rows from the angle sums are the ones of the old formula
    /// (a sine per row and wave, a power per row), to float rounding.
    #[test]
    fn tongue_rows_match_the_direct_formula() {
        let (foot, right, h, half_w) = (Vec3::new(10.0, 50.0, -20.0), Vec3::X, 60.0, 15.0);
        for (t, phase) in [(0.0f32, 0.0f32), (3.7, 12.9), (812.25, 29.4), (3600.5, 7.1)] {
            let mut q = QuadBuilder::new(false, 32);
            q.tongue(foot, right, h, half_w, t, phase, [1.0; 4], [0.5; 4]);
            for j in 0..=TONGUE_SEGMENTS {
                let s = j as f32 / TONGUE_SEGMENTS as f32;
                let s2 = s * s;
                let w1 = (t * 7.0 - s * 4.5 + phase).sin();
                let w2 = (t * 11.3 - s * 7.0 + phase * 1.7).sin();
                let lateral = h * (0.08 * s2 * w1 + 0.04 * s2 * s * w2);
                let width = half_w * (1.0 + 0.04 * (t * 9.0 + phase + s * 3.0).sin()) * (1.0 - 0.15 * s);
                let centre = foot + Vec3::Y * (h * s) + right * lateral;
                for (k, side) in [-1.0f32, 1.0].into_iter().enumerate() {
                    let want = centre + right * width * side;
                    let got = q.position(2 * j + k);
                    // the phases of t = 3600 s carry ~2e-3 rad of f32 rounding either way
                    assert!(want.distance(got) < 0.05 + 2e-3 * h, "t {t} row {j}: {want} vs {got}");
                }
            }
        }
    }
}
