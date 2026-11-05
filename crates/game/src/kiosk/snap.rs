//! `KIOSK_SNAP`: screenshots of arbitrary game states, without playing.
//!
//! `KIOSK_SNAP=<dir>` names the output folder; `KIOSK_SNAP_SPEC` lists the
//! states, separated by `;`. Each state is `key=value` pairs separated by `,`:
//!
//! | key      | meaning                                                              |
//! |----------|----------------------------------------------------------------------|
//! | `name`   | file name without `.png` (default `snap<N>`)                         |
//! | `phase`  | `title`, `intro`, `tutorial`, `gioco` (default), `fine`             |
//! | `step`   | the tutorial's step on show (with `phase=tutorial`)                   |
//! | `t`      | simulated minutes since the start, run headless (default 0)          |
//! | `pause`  | `1`: in pause (the expert's forecast on the map)                     |
//! | `move`   | `<team>:<x>:<y>`, orders given in pause at `t`, `+` between            |
//! | `evac`   | a district evacuated in pause at `t`                                 |
//! | `team`   | the team selected; with `hex=<x>:<y>` the move previewed             |
//! | `select` | the district selected (its evacuation previewed)                     |
//! | `hover`  | the district under the pointer (its route and time shown)            |
//! | `at`     | camera focus: `home`, `fire`, `district:<name>`, `team:<n>`, `<x>:<y>` |
//! | `dist`, `yaw`, `pitch` | the view, metres and degrees                           |
//! | `menu`   | `1`: the settings menu (F2) open                                      |
//! | `esc`    | `1`: the game menu (Esc) open, in the tutorial or the game             |
//! | `iso`    | `0`: no isochrones (lines and labels)                                |
//! | `marks`  | `0`: no team, order, hex, road or flag marks on the map              |
//! | `ui`     | `0`: no interface at all (panels, labels, buttons)                   |
//! | `mario`  | `1`: Mario beside the fire, turned to the camera (needs `pause=1`); `<x>:<y>`: there instead |
//! | `walk`   | `1`: with `mario`, he runs straight ahead instead of posing (for `rec`) |
//! | `live`   | `1`: the world keeps running (×300) instead of freezing at the shot  |
//! | `opening`| real seconds into the opening sequence, held (with `phase=title`)     |
//! | `rec`    | seconds of video with sound instead of a picture: `<name>.mp4` (`record`) |
//! | `spin`   | the camera turns, degrees of yaw per second (with `rec` or the bench) |
//! | `zoom`   | the camera backs off, metres per second (negative: closer)            |
//! | `pan`    | `<x>:<y>`: the focus slides, metres per second east and north         |
//!
//! The camera moves (`spin`, `zoom`, `pan`) from the end of the settling, on
//! the game's clock: a recording at a fixed step films the same path on any
//! machine, frame after frame, as a player's drag would show it.
//!
//! `KIOSK_SNAP_BENCH=<s>` also measures each state before its picture: the
//! frame times over `<s>` real seconds (vsync off), printed as `bench:` lines.
//!
//! The game is that of the phase (the tutorial's fire or the played one).
//! Time only goes forward: a state with an earlier `t` or another phase's
//! fire restarts it. The world is frozen at the shot. Example:
//!
//! ```text
//! KIOSK_SNAP=/tmp/snaps KIOSK_SNAP_SPEC='name=a,phase=intro;name=b,t=20,pause=1,team=0,hex=5300:4600' cargo run -p game --release
//! ```

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::PrimaryWindow;
use scenario::Pos;

use super::{Kiosk, Phase, Selection};
use crate::camera::OrbitCamera;
use crate::sim::{Sim, SimRestarted};

/// The view the state on show asks for, in radians and Bevy space.
#[derive(Resource, Default, Clone, Copy)]
pub struct SnapView {
    pub focus: Option<Vec3>,
    pub dist: Option<f32>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
}

/// Real seconds a state is shown before its picture is taken.
const SETTLE_S: f32 = 2.0;

#[derive(Default, Clone)]
struct State {
    name: String,
    phase: String,
    step: usize,
    t_min: f32,
    pause: bool,
    moves: Vec<(usize, Pos)>,
    evac: Option<String>,
    team: Option<usize>,
    hex: Option<Pos>,
    select: Option<String>,
    hover: Option<String>,
    at: String,
    dist: Option<f32>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    hide: SnapHide,
    menu: bool,
    esc: bool,
    mario: bool,
    /// `mario=<x>:<y>`: where Mario starts instead of beside the fire.
    mario_at: Option<Vec2>,
    /// `walk=1`: Mario runs ahead instead of posing (to film the ground going by).
    walk: bool,
    live: bool,
    opening: Option<f32>,
    rec: Option<f32>,
    /// The camera's motion: yaw (degrees/s), distance (m/s), focus (m/s
    /// east, north).
    spin: f32,
    zoom: f32,
    pan: Vec2,
}

/// What the state on show leaves out of the picture.
#[derive(Resource, Default, Clone, Copy)]
pub struct SnapHide {
    pub iso: bool,
    pub marks: bool,
    pub ui: bool,
}

fn xy(v: &str) -> Option<Pos> {
    let (x, y) = v.split_once(':')?;
    Some(Pos { x: x.parse().ok()?, y: y.parse().ok()? })
}

fn parse(spec: &str) -> Vec<State> {
    spec.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(n, s)| {
            let mut st = State { name: format!("snap{n}"), phase: "gioco".into(), at: "home".into(), ..Default::default() };
            for kv in s.split(',') {
                let Some((k, v)) = kv.split_once('=') else { continue };
                let v = v.trim();
                match k.trim() {
                    "name" => st.name = v.into(),
                    "phase" => st.phase = v.into(),
                    "step" => st.step = v.parse().unwrap_or(0),
                    "t" => st.t_min = v.parse().unwrap_or(0.0),
                    "pause" => st.pause = v == "1",
                    "move" => {
                        st.moves = v.split('+').filter_map(|m| m.split_once(':').and_then(|(t, p)| Some((t.parse().ok()?, xy(p)?)))).collect();
                    }
                    "evac" => st.evac = Some(v.into()),
                    "team" => st.team = v.parse().ok(),
                    "hex" => st.hex = xy(v),
                    "select" => st.select = Some(v.into()),
                    "hover" => st.hover = Some(v.into()),
                    "at" => st.at = v.into(),
                    "dist" => st.dist = v.parse().ok(),
                    "yaw" => st.yaw = v.parse().ok(),
                    "pitch" => st.pitch = v.parse().ok(),
                    "menu" => st.menu = v == "1",
                    "esc" => st.esc = v == "1",
                    "iso" => st.hide.iso = v == "0",
                    "marks" => st.hide.marks = v == "0",
                    "ui" => st.hide.ui = v == "0",
                    "mario" => {
                        st.mario_at = v.split_once(':').and_then(|(x, y)| Some(Vec2::new(x.parse().ok()?, y.parse().ok()?)));
                        st.mario = v == "1" || st.mario_at.is_some();
                    }
                    "walk" => st.walk = v == "1",
                    "live" => st.live = v == "1",
                    "opening" => st.opening = v.parse().ok(),
                    "rec" => st.rec = v.parse().ok(),
                    "spin" => st.spin = v.parse().unwrap_or(0.0),
                    "zoom" => st.zoom = v.parse().unwrap_or(0.0),
                    "pan" => st.pan = xy(v).map_or(Vec2::ZERO, |p| Vec2::new(p.x, p.y)),
                    other => eprintln!("snap: unknown key `{other}`"),
                }
            }
            st
        })
        .collect()
}

/// Since Bevy 0.20 the screenshot of a hidden window comes out black (seen on
/// macOS): with `KIOSK_SNAP` the game camera draws into an image of the
/// window's size instead, and that image is what gets saved.
#[derive(Resource)]
pub struct SnapTarget(pub Handle<Image>);

pub fn redirect_camera(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<Entity, Added<OrbitCamera>>,
) {
    if !active() {
        return;
    }
    let Ok(w) = windows.single() else { return };
    for e in &cams {
        let image = Image::new_target_texture(
            w.physical_width(),
            w.physical_height(),
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let handle = images.add(image);
        commands.entity(e).insert(bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: w.scale_factor(),
        }));
        commands.insert_resource(SnapTarget(handle));
    }
}

pub fn active() -> bool {
    std::env::var_os("KIOSK_SNAP").is_some()
}

/// Frames the CPU may queue ahead of the GPU, as with a swap chain (Bevy's
/// default `desired_maximum_frame_latency`).
#[cfg(not(target_arch = "wasm32"))]
const FRAMES_IN_FLIGHT: usize = 2;

/// The frame pacing a window would give. Drawing into an image (see
/// `redirect_camera`) there is no swap-chain texture to wait for, so the CPU
/// runs ahead of the GPU until Metal's command-buffer budget (4096) is full:
/// thousands of command buffers queued, seconds of latency, the GPU idle
/// between them, and frame times 20–30 % worse than the game's in a window
/// (measured 2026-10-21). Here each frame waits, before its views are
/// prepared, for the GPU to finish the frame `FRAMES_IN_FLIGHT` back.
#[cfg(not(target_arch = "wasm32"))]
pub struct SnapPacingPlugin;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Default)]
struct InFlight(std::collections::VecDeque<wgpu::SubmissionIndex>);

#[cfg(not(target_arch = "wasm32"))]
impl Plugin for SnapPacingPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        use bevy::render::renderer::{render_system, RenderDevice, RenderQueue};
        use bevy::render::{Render, RenderApp, RenderSystems};
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render
            .init_resource::<InFlight>()
            // the frame's work is all submitted by `render_system`: mark its end
            .add_systems(Render, (|mut f: ResMut<InFlight>, q: Res<RenderQueue>| {
                f.0.push_back(q.submit(std::iter::empty()));
            }).in_set(RenderSystems::Render).after(render_system))
            .add_systems(Render, (|mut f: ResMut<InFlight>, d: Res<RenderDevice>| {
                while f.0.len() > FRAMES_IN_FLIGHT {
                    let i = f.0.pop_front().expect("not empty");
                    let _ = d.poll(wgpu::PollType::Wait { submission_index: Some(i), timeout: None });
                }
            }).in_set(RenderSystems::CreateViews));
    }
}

#[derive(Default)]
pub struct Progress {
    states: Option<Vec<State>>,
    at: usize,
    applied: bool,
    waited: f32,
    goal: Option<(Vec3, Option<f32>, Option<f32>, Option<f32>)>,
    done_s: f32,
    /// The state on show is being filmed (`rec`).
    filming: bool,
    /// frame times (s) of the state on show, while benchmarking
    frames: Vec<f32>,
    /// Where a moving camera (`spin`, `zoom`, `pan`) set off from: focus,
    /// distance, yaw (radians).
    base: Option<(Vec3, f32, f32)>,
}

/// `KIOSK_SNAP_BENCH`: the seconds each state is measured for.
pub fn bench_s() -> Option<f32> {
    std::env::var("KIOSK_SNAP_BENCH").ok()?.parse().ok()
}

/// Bring the game to the state.
fn apply(st: &State, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) -> Option<(Vec3, Option<f32>, Option<f32>, Option<f32>)> {
    let phase = match st.phase.as_str() {
        "intro" | "title" => Phase::Intro,
        "tutorial" => Phase::Tutorial,
        "scenario" => Phase::Scenario,
        "fine" => Phase::Fine,
        _ => Phase::Gioco,
    };
    let case = if matches!(phase, Phase::Tutorial) { k.tutorial_case() } else { k.selected_case.clone() };
    let t_s = (st.t_min * 60.0) as i64;
    if k.player.is_empty() {
        k.player = "Giulia".into();
    }
    if case != sim.case.name || t_s < sim.time_s() {
        if let Err(e) = sim.restart(&case, k.seed) {
            k.error = Some(format!("{e:#}"));
        }
        restarted.write(SimRestarted);
        if phase == Phase::Fine {
            k.start_baseline(&case);
        }
    }
    if let Err(e) = sim.run_until(t_s) {
        k.error = Some(format!("{e:#}"));
    }
    let wants_pause = st.pause || !st.moves.is_empty() || st.evac.is_some();
    if wants_pause && !sim.paused() {
        if let Err(e) = sim.pause() {
            eprintln!("snap: {}: {e}", st.name);
        }
    }
    for &(t, p) in &st.moves {
        if let Some(h) = sim.hexes.nearest(p).map(|h| h.at) {
            if let Err(e) = sim.move_team(t, h) {
                eprintln!("snap: {}: {e}", st.name);
            }
        }
    }
    if let Some(d) = st.evac.as_deref().and_then(|n| sim.district_index(n)) {
        if let Err(e) = sim.evacuate(d) {
            eprintln!("snap: {}: {e}", st.name);
        }
    }
    if !st.pause && sim.paused() {
        sim.resume();
    }
    sim.generation += 1;
    k.enter(phase);
    k.title = st.phase == "title";
    k.opening = st.opening;
    if phase == Phase::Tutorial {
        k.script = rocca::tutorial::Script::of(sim);
        k.step = st.step;
    }
    k.operator = st.menu;
    k.menu = st.esc;
    k.frozen = !st.live;
    k.follow = false;
    if let Some(t) = st.team {
        k.selection = Selection::Team(t);
        if let Some(p) = st.hex {
            // the usable hex nearest the point, as `move` does
            let at = sim.hexes.nearest(p).map_or(rocca::Axial::of(p), |h| h.at);
            k.move_hex = Some(at);
            k.move_plan = Some(sim.plan_move(t, at));
            // as if the team's flag were being dragged there
            k.dragging = Some((t, p));
            if let Some(Err(e)) = &k.move_plan {
                eprintln!("snap: {}: {e}", st.name);
            }
        }
    }
    k.evac_hover = None;
    if let Some(d) = st.hover.as_deref().and_then(|n| sim.district_index(n)) {
        k.evac_hover = Some((d, sim.generation, sim.plan_evacuation(d)));
    }
    if let Some(d) = st.select.as_deref().and_then(|n| sim.district_index(n)) {
        k.selection = Selection::District(d);
        k.evac_plan = Some(sim.plan_evacuation(d));
    }

    let on = |p: Pos| crate::frame::to_bevy(p, sim.scn.terrain.height_at(p));
    let (focus, span) = match st.at.as_str() {
        "home" => return Some((Vec3::NAN, st.dist, st.yaw, st.pitch)),
        // a cell burning now, the middle one of the front's list
        "front" => match sim.fire.active_cells() {
            [] => return Some((Vec3::NAN, st.dist, st.yaw, st.pitch)),
            cells => (on(sim.scn.world.centre_of(cells[cells.len() / 2])), None),
        },
        "fire" => match super::view::fire_extent(sim) {
            Some((mid, span)) => (Vec3::new(mid.x, 0.0, mid.y), Some(span * 2.8)),
            None => return Some((Vec3::NAN, st.dist, st.yaw, st.pitch)),
        },
        other => {
            let p = if let Some(name) = other.strip_prefix("district:") {
                sim.district_index(name).map(|d| sim.districts[d].centre)
            } else if let Some(n) = other.strip_prefix("team:") {
                n.parse::<usize>().ok().and_then(|n| sim.teams.get(n)).map(|t| sim.crews.units[t.unit].pos)
            } else {
                xy(other)
            };
            match p {
                Some(p) => (on(p), None),
                None => {
                    eprintln!("snap: cannot place the camera at `{other}`");
                    return Some((Vec3::NAN, st.dist, st.yaw, st.pitch));
                }
            }
        }
    };
    Some((focus, st.dist.or(span).or(Some(1200.0)), st.yaw, st.pitch.or(Some(-46.0))))
}

/// Walk the states: apply one, hold it while the camera settles, take the
/// picture, go on; quit after the last.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut kiosk: ResMut<Kiosk>,
    mut sim: ResMut<Sim>,
    mut orbit: Query<&mut OrbitCamera>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    mut restarted: MessageWriter<SimRestarted>,
    mut view: ResMut<SnapView>,
    mut hide: ResMut<SnapHide>,
    mut mario: ResMut<crate::mario::Playground>,
    costs: Option<Res<crate::bench::Costs>>,
    target: Option<Res<SnapTarget>>,
    mut rec: ResMut<crate::record::Recorder>,
    mut p: Local<Progress>,
) {
    let Ok(dir) = std::env::var("KIOSK_SNAP") else { return };
    let Ok(window) = windows.single() else { return };
    if p.states.is_none() {
        let spec = std::env::var("KIOSK_SNAP_SPEC").unwrap_or_else(|_| "name=snap0".into());
        p.states = Some(parse(&spec));
        let _ = std::fs::create_dir_all(&dir);
        eprintln!("snap: cases {:?}", kiosk.territory.cases.iter().map(|c| c.name.as_str()).collect::<Vec<_>>());
        eprintln!("snap: districts {:?}", sim.districts.iter().map(|d| d.name.as_str()).collect::<Vec<_>>());
    }
    let dt = time.delta_secs();
    let Some(st) = p.states.as_ref().and_then(|s| s.get(p.at)).cloned() else {
        p.done_s += dt;
        if p.done_s > 1.0 {
            exit.write(AppExit::Success);
        }
        return;
    };
    if !p.applied {
        p.goal = apply(&st, &mut kiosk, &mut sim, &mut restarted);
        p.applied = true;
        p.waited = 0.0;
        p.base = None;
        if st.mario {
            mario.request_start_for_snapshot(false, st.mario_at, st.walk);
        }
    }
    p.waited += dt;
    *hide = st.hide;
    if let Some((focus, dist, yaw, pitch)) = p.goal {
        *view = SnapView { focus: (!focus.is_nan()).then_some(focus), dist, yaw: yaw.map(f32::to_radians), pitch: pitch.map(f32::to_radians) };
        // the camera on its way, from where the settling left it
        let moving = st.spin != 0.0 || st.zoom != 0.0 || st.pan != Vec2::ZERO;
        if let (true, Ok(o)) = (moving && p.waited > SETTLE_S, orbit.single()) {
            let (f, d, y) = *p.base.get_or_insert((o.focus, o.distance, o.yaw));
            let s = p.waited - SETTLE_S;
            view.focus = Some(f + Vec3::new(st.pan.x, 0.0, -st.pan.y) * s);
            view.dist = Some(d + st.zoom * s);
            view.yaw = Some(y + (st.spin * s).to_radians());
        }
        if let Ok(mut o) = orbit.single_mut() {
            if let Some(f) = view.focus {
                o.focus = f;
            }
            if let Some(d) = view.dist {
                o.distance = d;
            }
            if let Some(y) = view.yaw {
                o.yaw = y;
            }
            if let Some(pt) = view.pitch {
                o.pitch = pt;
            }
        }
    } else {
        *view = SnapView::default();
    }
    let ready = (st.phase != "fine" || kiosk.baseline().is_some())
        && (!st.mario || mario.snapshot_ready())
        && !hi_trees_pending();
    if let Some(b) = bench_s().filter(|_| p.waited > SETTLE_S && ready) {
        if p.frames.is_empty() {
            if let Some(c) = &costs {
                c.cpu.lock().unwrap().clear();
                c.gpu_pass.lock().unwrap().clear();
                c.encode.lock().unwrap().clear();
                c.render.lock().unwrap().clear();
            }
        }
        if p.waited < SETTLE_S + b {
            p.frames.push(real.delta_secs());
            return;
        }
        let stat = |v: &mut Vec<f32>| {
            v.sort_by(f32::total_cmp);
            if v.is_empty() { (0.0, 0.0, 0.0) } else { (v.iter().sum::<f32>() / v.len() as f32, v[(v.len() * 95 / 100).min(v.len() - 1)], v[v.len() - 1]) }
        };
        if let Some(c) = &costs {
            let (cm, cp, cx) = stat(&mut c.cpu.lock().unwrap());
            let (gm, gp, gx) = stat(&mut c.gpu_pass.lock().unwrap());
            let (em, _, _) = stat(&mut c.encode.lock().unwrap());
            let (rm, rp, rx) = stat(&mut c.render.lock().unwrap());
            eprintln!("bench: {:<14} gpu {gm:5.2} ms (p95 {gp:5.2}, max {gx:5.1})  render thread {rm:5.2} ms (p95 {rp:5.2}, max {rx:5.1}, graph {em:5.2})  main {cm:5.2} ms (p95 {cp:5.2}, max {cx:5.1})", st.name);
        }
        let mut f = std::mem::take(&mut p.frames);
        f.sort_by(f32::total_cmp);
        if !f.is_empty() {
            let mean = f.iter().sum::<f32>() / f.len() as f32;
            let p95 = f[(f.len() * 95 / 100).min(f.len() - 1)];
            let long = f.iter().filter(|&&d| d > 0.025).count();
            eprintln!("bench: {:<14} {:6.1} fps  mean {:5.2} ms  p95 {:5.2} ms  max {:5.1} ms  ({} frames, {long} over 25 ms)", st.name, 1.0 / mean, mean * 1e3, p95 * 1e3, f[f.len() - 1] * 1e3, f.len());
        }
    }
    // filming: one picture a frame until the recording is written
    if let (Some(seconds), Some(t)) = (st.rec, target.as_ref()) {
        if p.waited > SETTLE_S && ready {
            if !p.filming {
                p.filming = true;
                rec.start(&mut commands, &std::path::Path::new(&dir).join(&st.name), seconds);
            }
            if rec.frame(&mut commands, &t.0) {
                p.filming = false;
                p.at += 1;
                p.applied = false;
            }
        }
        return;
    }
    if p.waited > SETTLE_S && ready {
        let shot = target.as_ref().map_or(Screenshot::window(window), |t| Screenshot::image(t.0.clone()));
        commands.spawn(shot).observe(save_to_disk(format!("{dir}/{}.png", st.name)));
        eprintln!("snap: {} (t={} min, phase {})", st.name, st.t_min, st.phase);
        p.at += 1;
        p.applied = false;
    }
}

/// Runs after the kiosk camera: puts the camera exactly where the state asks.
pub fn hold(view: Res<SnapView>, sim: Res<Sim>, mut cam: Query<(&mut OrbitCamera, &mut Transform, Option<&mut bevy::post_process::dof::DepthOfField>)>) {
    if !active() || (view.focus.is_none() && view.dist.is_none() && view.yaw.is_none() && view.pitch.is_none()) {
        return;
    }
    let Ok((mut orbit, mut tf, mut dof)) = cam.single_mut() else { return };
    if let Some(f) = view.focus {
        orbit.focus = f;
    }
    if let Some(d) = view.dist {
        orbit.distance = d.max(20.0);
    }
    if let Some(y) = view.yaw {
        orbit.yaw = y;
    }
    if let Some(p) = view.pitch {
        // up to 34° upward: from low on a slope, the eye looks up at what
        // stands over it (the focus higher than the eye, against the sun)
        orbit.pitch = p.clamp(-1.553, 0.6);
    }
    orbit.focus.y = sim.scn.terrain.height_at(crate::frame::to_world(orbit.focus));
    let dir = Vec3::new(orbit.yaw.sin() * orbit.pitch.cos(), -orbit.pitch.sin(), orbit.yaw.cos() * orbit.pitch.cos());
    tf.translation = orbit.focus + dir * orbit.distance;
    let ground = sim.scn.terrain.height_at(crate::frame::to_world(tf.translation));
    tf.translation.y = tf.translation.y.max(ground + 4.0);
    tf.look_at(orbit.focus, Vec3::Y);
    if let Some(dof) = dof.as_mut() {
        dof.focal_distance = tf.translation.distance(orbit.focus);
    }
}

/// Runs after the marks are updated: hides what the state leaves out.
pub fn hide_marks(
    hide: Res<SnapHide>,
    mario: Res<crate::mario::Playground>,
    mut iso: Query<&mut Visibility, With<crate::forecast_view::ForecastMark>>,
    mut marks: Query<
        &mut Visibility,
        (
            Without<crate::forecast_view::ForecastMark>,
            Or<(
                With<super::marks::HexMark>,
                With<super::marks::DistrictZoneMark>,
                With<super::marks::RoadNetMark>,
                With<super::marks::OrderMark>,
                With<super::marks::ClosedMark>,
                With<super::marks::TeamMark>,
                With<super::pins::Pin>,
                // the teams' cut lines and order rings: yellow ribbons over the scene
                With<crate::units::WorkOverlay>,
                With<crate::units::OrderMarker>,
            )>,
        ),
    >,
) {
    // Mario has the whole map: no isochrones, hexes, cones or other marks
    let hide = if mario.active { SnapHide { iso: true, marks: true, ui: hide.ui } } else { *hide };
    if !active() && !mario.active {
        return;
    }
    if hide.iso {
        for mut v in &mut iso {
            *v = Visibility::Hidden;
        }
    }
    if hide.marks {
        for mut v in &mut marks {
            *v = Visibility::Hidden;
        }
    }
}

/// After egui has made its output: no interface in the picture.
pub fn hide_ui(hide: Res<SnapHide>, mut out: Query<&mut bevy_egui::EguiRenderOutput>) {
    if hide.ui {
        for mut o in &mut out {
            o.paint_jobs = Default::default();
        }
    }
}

/// Whether the full-resolution trees (`hi_trees`, desktop only) are still
/// being built: a snapshot waits for them.
fn hi_trees_pending() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::hi_trees::PENDING.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}
