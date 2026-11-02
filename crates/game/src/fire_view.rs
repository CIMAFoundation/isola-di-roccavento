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
use crate::fire_shader::{FireMaterial, SmokeMaterial};
use crate::sim::Sim;
use crate::textures;

/// Fire map rewrite throttle (`update_fire_map`). At high time acceleration
/// the generation advances several times per frame.
const FIRE_MAP_MIN_INTERVAL_S: f32 = 0.15;

/// How long a cell actually *flames* after the front arrives. The model keeps
/// a cell alight for 20 minutes (`fire::BURNOUT_S`), which is residence time
/// including smouldering; the flaming zone of a real fire is a band of tens of
/// metres, not the whole burnt area.
const FLAMING_S: f32 = 900.0;

/// Caps, all of them per-frame geometry budgets rather than physical limits.
const MAX_FLAME_CELLS: usize = if crate::quality::WEBGL2 { 600 } else { 3_500 };
const MAX_SMOKE: usize = if crate::quality::WEBGL2 { 100 } else { 800 };
const MAX_EMBERS: usize = if crate::quality::WEBGL2 { 60 } else { 525 };

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
}

/// Everything about a front tongue that does not depend on the frame's clock
/// or camera: terrain height, hashes, intensities.
struct CachedTongue {
    foot: Vec3,
    flame_m: f32,
    ratio: f32,
    phase: f32,
    heat: f32,
    flaming: f32,
    /// Crown flare-up: height factor (1 = none), cycle offset, lift at peak.
    crown_h: f32,
    crown_cyc: f32,
    crown_lift: f32,
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut smoke_materials: ResMut<Assets<SmokeMaterial>>,
    mut fire_materials: ResMut<Assets<FireMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let flames = Streamed::new(&mut meshes, TONGUE, false);
    let smoke_mesh = Streamed::new(&mut meshes, BILLBOARD, true);
    let sparks = Streamed::new(&mut meshes, BILLBOARD, false);

    let flame_tex = images.add(textures::flame_tongue());
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

/// What a row of every tongue shares: its height share `s` (0 foot, 1 tip)
/// and powers, the taper `(1 − s)^0.85`, and the waves' phase offsets along
/// the tongue as (cos, sin): 4.5 s, 7 s and 3 s.
struct TongueRow {
    s: f32,
    s2: f32,
    s3: f32,
    taper: f32,
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
            TongueRow { s, s2: s * s, s3: s * s * s, taper: (1.0 - s).powf(0.85), w1: cs(4.5), w2: cs(7.0), w3: cs(3.0) }
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

    /// A flame tongue: a ribbon of `TONGUE_SEGMENTS` rows from foot to tip.
    /// It tapers to a point and its spine travels on two travelling waves
    /// whose amplitude grows toward the tip, so the tongue snakes and licks
    /// instead of bobbing. `foot` is the centre of the base; `h` the height.
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
        for row in tongue_rows() {
            let (s, s2, s3) = (row.s, row.s2, row.s3);
            // Two waves running upward at different speeds: the second makes
            // the motion non-periodic to the eye.
            let w1 = a1 * row.w1.0 - b1 * row.w1.1; // sin(t·7 + phase − 4.5 s)
            let w2 = a2 * row.w2.0 - b2 * row.w2.1; // sin(t·11.3 + 1.7 phase − 7 s)
            let w3 = a3 * row.w3.0 + b3 * row.w3.1; // sin(t·9 + phase + 3 s)
            let lateral = h * (0.30 * s2 * w1 + 0.12 * s3 * w2);
            // The tip droops a little as it whips.
            let drop = h * 0.06 * s3 * (w1 * w2).abs();
            let width = half_w * row.taper * (1.0 + 0.18 * w3);
            let centre = foot + Vec3::Y * (h * s - drop) + right * lateral;
            let color = [
                bottom[0] + (top[0] - bottom[0]) * s,
                bottom[1] + (top[1] - bottom[1]) * s,
                bottom[2] + (top[2] - bottom[2]) * s,
                1.0,
            ];
            // uv.y: 1 at the foot, 0 at the tip (what the shader expects).
            let side = right * width;
            self.vertex(centre - side, [0.0, 1.0, 0.0], [0.0, 1.0 - s], color);
            self.vertex(centre + side, [0.0, 1.0, 0.0], [1.0, 1.0 - s], color);
        }
        // joined by `TONGUE`'s indices
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
    camera: Query<&Transform, (With<Camera3d>, Without<crate::camera::AuxiliaryCamera>)>,
    buildings: Res<crate::buildings::Buildings>,
    fog: Query<(&DistanceFog, Option<&Exposure>), With<Camera3d>>,
    suns: Query<(&GlobalTransform, &DirectionalLight)>,
    mut view: ResMut<FireView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut frame: ResMut<StreamedFrame>,
) {
    let Ok(cam) = camera.single() else {
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
            if flaming < 0.05 {
                continue;
            }

            // Byram flame length, with a floor: a metre-high creeping flame is
            // physically right and visually nothing, and the player still has to
            // be able to see where the fire is.
            // Toy scale, like the houses (`buildings::TOY_SCALE`): from the play
            // camera a true-height flame is a few pixels and the fire read as a
            // dark smudge; the front has to be the brightest thing on the table.
            let flame_m = (flame_length_m(fli) * 3.4).clamp(22.0, 110.0) * (0.45 + 0.55 * flaming);
            // A hot cell carries several tongues, a creeping one carries a single
            // flicker. Sub-cell placement is what stops them lining up on a grid.
            let n = (1.0 + (fli / 450.0).min(6.0) * flaming).round() as usize;
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
                tongues.push(CachedTongue {
                    foot: Vec3::new(p.x, ground, -p.y),
                    flame_m,
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
                });
            }
        }
        view.tongues = tongues;
    }

    let mut flames = QuadBuilder::new(false, view.last_flame_verts);
    let max_half_w = scn.world.cellsize * 1.3;
    for c in &view.tongues {
        let flicker = 0.62 + 0.38 * (t * 5.5 + c.phase).sin() * (t * 2.3 + c.phase).cos();
        let mut h_m = c.flame_m * flicker * c.ratio;
        let mut lift = 0.0;
        // Crowning, drawn only: now and then a tongue over tall trees
        // climbs into the canopy and roars well above the surface fire.
        // Each tongue has its own slow cycle, so flare-ups come and go
        // at random.
        if c.crown_h > 1.0 && c.flaming > 0.3 {
            let cyc = (t * 0.11 + c.crown_cyc).fract();
            let env = if cyc < 0.22 { (std::f32::consts::PI * cyc / 0.22).sin() } else { 0.0 };
            h_m *= 1.0 + (c.crown_h - 1.0) * env * c.flaming;
            // The tongue stands on the crown, not on the trunk's foot:
            // the shader already burns the foliage there (`burn_plant`).
            lift = env * c.flaming * c.crown_lift;
        }
        let half_w = (h_m * 0.38).min(max_half_w);
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
        if view.smoke.len() < MAX_SMOKE && hash01(view.seed.wrapping_add(i as u64) ^ (t * 60.0) as u64) < dt * 3.0 {
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
    let mut smoke = QuadBuilder::new(true, 4 * view.smoke.len());
    for puff in &view.smoke {
        let k = (puff.age / puff.life).clamp(0.0, 1.0);
        // Grows as it disperses, darkest and densest near the fire.
        let size = puff.size * (0.6 + 1.9 * k);
        // Pale, toy smoke: grey near the fire, near-white as it disperses, so
        // the plume shows the wind without burying the flames under it.
        let shade = 0.58 + 0.34 * k;
        // Fade in fast, out slowly: a puff should never pop into existence.
        // Kept thin — hundreds of puffs overlap, and at 0.55 each the plume
        // turned into a white wall as soon as the camera dropped into it.
        let alpha = (k * 4.0).min(1.0) * (1.0 - k).powf(1.4) * 0.22;
        let spin = puff.phase * std::f32::consts::TAU + k * 0.6;
        let (s, c) = spin.sin_cos();
        let r = (right * c + up * s) * size;
        let u = (up * c - right * s) * size;
        let (gain, offset) = fogger.at(puff.pos);
        let c = [
            shade * 1.05 * gain.x,
            shade * 0.92 * gain.y,
            shade * 0.84 * gain.z,
            alpha,
        ];
        // The normal slot carries the fog's additive part (`smoke.wgsl`).
        smoke.billboard(puff.pos, r, u, 1.0, c, c, offset.into());
    }

    // Embers are plain round particles (`flame.wgsl`, mode 2). Colour
    // channels carry no colour: r = age share,
    // g = per-ember phase, b = gain (negative for the landing flash).
    let eye = cam.translation;
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
        let size = (ember.size * (1.0 - 0.35 * k)).max(min_size);
        let gain = 0.9 + 0.5 * ember.phase;
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
        let buoyancy = Vec3::Y * (7.0 * (1.0 - k) * (1.0 - k));
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
    let (smoke_owed, ember_owed) = (view.owed.0 + active.len() as f32 * dt * 1.2, view.owed.1 + active.len() as f32 * dt * 0.375);
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
            // Smouldering ground smokes too — often more visibly than flame.
            let plume = (fli / 1200.0).clamp(0.3, 3.0);
            view.smoke.push(Particle {
                pos: Vec3::new(p.x, ground + 25.0 + 20.0 * flaming, -p.y),
                vel: Vec3::Y * (4.0 + 8.0 * plume * flaming) + drift * 0.3,
                age: 0.0,
                life: 22.0 + phase * 20.0,
                size: 12.0 + 16.0 * plume,
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
                let lateral = h * (0.30 * s2 * w1 + 0.12 * s2 * s * w2);
                let drop = h * 0.06 * s2 * s * (w1 * w2).abs();
                let width = half_w * (1.0 - s).powf(0.85) * (1.0 + 0.18 * (t * 9.0 + phase + s * 3.0).sin());
                let centre = foot + Vec3::Y * (h * s - drop) + right * lateral;
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
