//! «Dove andrà il fuoco»: the forecast's isochrones on the map
//! (`rocca::forecast`, user 2026-10-13).
//!
//! When the game says a forecast is due ([`rocca::Game::forecast_epoch`]: the
//! change of wind announced, then the wind turned), the ensemble runs off the
//! frame — on a thread natively, one member per frame in the browser — and
//! the 50 % contours at +1 h (solid) and +3 h (dashed) are draped on the
//! terrain above the canopy, with a small label each. Nothing here decides
//! anything: what is due, the members and the probabilities are the game's.
//! Cleared when no forecast is due any more, on a restart, and on the title
//! screen.

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rocca::forecast::{Epoch, Forecast, Job, HORIZONS};
use scenario::{Pos, World};

use crate::sim::{Sim, SimRestarted};

/// Contours shorter than this (a lone spot fire's cell) are not drawn.
const MIN_LOOP_M: f32 = 160.0;
/// Gaussian smoothing of the isochrones along their length, and the spacing
/// of the resampled points, metres.
const SMOOTH_SIGMA_M: f32 = 90.0;
const SMOOTH_STEP_M: f32 = 12.0;
/// Label lift above the ground, metres.
const LABEL_LIFT_M: f32 = 30.0;
/// Shades of red, +1 h the deepest and +3 h the palest (user, 2026-10-16).
const LINES: [[f32; 3]; HORIZONS] = [[1.0, 0.04, 0.02], [1.0, 0.85, 0.0]];
/// The same, for the legend in the interface.
pub const LINE_COLOURS: [bevy_egui::egui::Color32; HORIZONS] = [
    bevy_egui::egui::Color32::from_rgb(255, 10, 5),
    bevy_egui::egui::Color32::from_rgb(255, 217, 0),
];
pub fn label(h: usize) -> &'static str {
    [rocca::tr!("tra 1 ora"), rocca::tr!("tra 3 ore")][h]
}
/// A label keeps this far from the others on the screen, pixels.
const LABEL_APART_PX: egui::Vec2 = egui::vec2(96.0, 26.0);

#[derive(Component)]
pub struct ForecastMark;

#[cfg(not(target_arch = "wasm32"))]
type Running = std::sync::Arc<std::sync::Mutex<Option<Result<Forecast, String>>>>;
#[cfg(target_arch = "wasm32")]
type Running = Job;

#[derive(Resource, Default)]
pub struct ForecastView {
    /// Bumped on each restart: a forecast computed for an older game is dropped.
    generation: u64,
    /// The forecast last asked for: what the game said was due.
    asked: Option<Epoch>,
    running: Option<(u64, Running)>,
    /// The forecast on the map, and per horizon the points a label may
    /// stand on, leading edge (down the wind) first.
    shown: Option<(Forecast, [Vec<Pos>; HORIZONS])>,
}

pub struct ForecastPlugin;

impl Plugin for ForecastPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ForecastView>()
            .add_systems(Update, update.after(crate::kiosk::step).run_if(in_state(crate::AppState::Playing)))
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                labels.after(crate::kiosk::ui::draw).run_if(in_state(crate::AppState::Playing)),
            );
    }
}

fn start(job: Job) -> Running {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let slot: Running = Default::default();
        let out = slot.clone();
        std::thread::spawn(move || {
            let r = job.run().map_err(|e| format!("{e:#}"));
            if let Ok(mut s) = out.lock() {
                *s = Some(r);
            }
        });
        slot
    }
    #[cfg(target_arch = "wasm32")]
    job
}

enum Poll {
    Pending(Running),
    Done(Result<Forecast, String>),
}

/// The finished forecast, if it is: one member more per call in the browser.
fn poll(r: Running) -> Poll {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let done = r.lock().ok().and_then(|mut s| s.take());
        match done {
            Some(f) => Poll::Done(f),
            None => Poll::Pending(r),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut r = r;
        match r.step() {
            Ok(true) => Poll::Done(Ok(r.finish())),
            Ok(false) => Poll::Pending(r),
            Err(e) => Poll::Done(Err(format!("{e:#}"))),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    mut view: ResMut<ForecastView>,
    sim: Res<Sim>,
    kiosk: Res<crate::kiosk::Kiosk>,
    mut restarted: MessageReader<SimRestarted>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::forecast_zone::ZoneMaterial>>,
    mut marks: Query<(Entity, &mut Visibility), With<ForecastMark>>,
) {
    let clear = |view: &mut ForecastView, commands: &mut Commands, marks: &Query<(Entity, &mut Visibility), With<ForecastMark>>| {
        for (e, _) in marks.iter() {
            commands.entity(e).despawn();
        }
        view.shown = None;
    };
    if !restarted.is_empty() {
        restarted.clear();
        view.generation += 1;
        view.asked = None;
        view.running = None;
        clear(&mut view, &mut commands, &marks);
    }
    let due = sim.forecast_epoch();
    if due.is_none() {
        if view.shown.is_some() || view.running.is_some() || !marks.is_empty() {
            clear(&mut view, &mut commands, &marks);
        }
        view.asked = None;
        view.running = None;
        return;
    }
    if due != view.asked {
        view.asked = due;
        match sim.forecast_job() {
            Ok(job) => view.running = Some((view.generation, start(job))),
            Err(e) => warn!("previsione: {e:#}"),
        }
    }
    if let Some((generation, r)) = view.running.take() {
        match poll(r) {
            Poll::Pending(r) => view.running = Some((generation, r)),
            Poll::Done(Err(e)) => warn!("previsione: {e}"),
            Poll::Done(Ok(f)) if generation == view.generation && f.issued_s <= sim.time_s() => {
                clear(&mut view, &mut commands, &marks);
                let world = sim.scn.world;
                // the labels go on the leading edge: the furthest point down
                // the wind the forecast is about (the announced one, if any)
                let w = sim.territory.shifted(&sim.case).map_or(sim.fire.weather(), |(_, w)| w);
                let to = (w.wind_dir_deg as f32 + 180.0).to_radians();
                let mut at: [Vec<Pos>; HORIZONS] = Default::default();
                let masks = masks(&f);
                let mut all: [Vec<Vec<Pos>>; HORIZONS] = Default::default();
                for h in 0..HORIZONS {
                    let lines = isochrone(&masks[h], f.rows, f.cols, &world);
                    let ahead = |p: &Pos| p.x * to.sin() + p.y * to.cos();
                    at[h] = lines.iter().flatten().step_by(4).copied().collect();
                    at[h].sort_by(|a, b| ahead(b).total_cmp(&ahead(a)));
                    all[h] = lines;
                }
                commands.spawn((
                    (
                        Mesh3d(meshes.add(crate::forecast_zone::grid_mesh(&sim.scn, &all, &f.eta_s))),
                        MeshMaterial3d(materials.add(crate::forecast_zone::ZoneMaterial::new(LINES))),
                    ),
                    bevy::camera::visibility::NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                    ForecastMark,
                ));
                view.shown = Some((f, at));
            }
            Poll::Done(Ok(_)) => {}
        }
    }
    // the title screen's game shows no signs, nor does the debrief
    let hide = !matches!(kiosk.phase, crate::kiosk::Phase::Tutorial | crate::kiosk::Phase::Gioco);
    let want = if hide { Visibility::Hidden } else { Visibility::Inherited };
    for (_, mut v) in &mut marks {
        if *v != want {
            *v = want;
        }
    }
}

/// «tra 1 ora», «tra 3 ore» beside each isochrone: on its leading edge, or
/// the point nearest it that is on the map and not under the sidebar.
pub fn labels(
    mut contexts: EguiContexts,
    view: Res<ForecastView>,
    sim: Res<Sim>,
    kiosk: Res<crate::kiosk::Kiosk>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::camera::OrbitCamera>>,
    snap_hide: Res<crate::kiosk::snap::SnapHide>,
    mario: Res<crate::mario::Playground>,
) {
    if snap_hide.iso || mario.active {
        return;
    }
    let Some((_, at)) = &view.shown else { return };
    if !matches!(kiosk.phase, crate::kiosk::Phase::Tutorial | crate::kiosk::Phase::Gioco) {
        return;
    }
    let Ok((cam, gt)) = cameras.single() else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    // after the sidebar: what is left is the map
    let map = ctx.viewport_rect().shrink(40.0);
    let mut placed: Vec<egui::Pos2> = vec![];
    for (h, pts) in at.iter().enumerate() {
        // the first point on the map that is not on top of an earlier label
        let on_map = pts.iter().find_map(|&p| {
            let v = crate::frame::to_bevy(p, sim.scn.terrain.height_at(p) + LABEL_LIFT_M);
            let s = cam.world_to_viewport(gt, v).ok().map(crate::kiosk::display::to_points).filter(|&s| map.contains(s))?;
            placed.iter().all(|q| (s.x - q.x).abs() >= LABEL_APART_PX.x || (s.y - q.y).abs() >= LABEL_APART_PX.y).then_some(s)
        });
        let Some(s) = on_map else { continue };
        placed.push(s);
        let c = LINES[h].map(|x| (x * 255.0) as u8);
        egui::Area::new(egui::Id::new(("previsione", h)))
            .fixed_pos(egui::pos2(s.x + 8.0, s.y - 14.0))
            .order(egui::Order::Background)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(14, 16, 18, 215))
                    .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(c[0], c[1], c[2])))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(label(h)).size(14.0).strong().color(egui::Color32::from_rgb(c[0], c[1], c[2]))).extend());
                    });
            });
    }
}

/// The likely cells of each horizon, as the forecast gives them.
fn masks(f: &Forecast) -> [Vec<bool>; HORIZONS] {
    std::array::from_fn(|h| (0..f.rows * f.cols).map(|i| f.likely(h, i)).collect())
}

/// The 50 % contour of horizon `h`, as closed or open polylines in world
/// metres: marching squares on the fire grid's cell centres, padded so every
/// contour closes, joined into lines and smoothed.
pub fn isochrone(mask: &[bool], fire_rows: usize, fire_cols: usize, world: &World) -> Vec<Vec<Pos>> {
    let (rows, cols) = (fire_rows as i64 + 2, fire_cols as i64 + 2);
    let inside = |r: i64, c: i64| -> bool {
        let (r, c) = (r - 1, c - 1);
        r >= 0 && c >= 0 && r < fire_rows as i64 && c < fire_cols as i64 && mask[(r * fire_cols as i64 + c) as usize]
    };
    // edges by id: horizontal (r, c)-(r, c+1) is 2*(r*cols+c), vertical (r, c)-(r+1, c) is that + 1
    let hz = |r: i64, c: i64| 2 * (r * cols + c);
    let vt = |r: i64, c: i64| 2 * (r * cols + c) + 1;
    let mid = |e: i64| -> (f32, f32) {
        let (k, v) = (e / 2, e % 2 == 1);
        let (r, c) = ((k / cols) as f32, (k % cols) as f32);
        if v { (r + 0.5, c) } else { (r, c + 0.5) }
    };
    let mut segs: Vec<(i64, i64)> = vec![];
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let (a, b, d, e) = (inside(r, c), inside(r, c + 1), inside(r + 1, c + 1), inside(r + 1, c));
            let (top, right, bottom, left) = (hz(r, c), vt(r, c + 1), hz(r + 1, c), vt(r, c));
            let case = (a as u8) | (b as u8) << 1 | (d as u8) << 2 | (e as u8) << 3;
            match case {
                0 | 15 => {}
                1 | 14 => segs.push((left, top)),
                2 | 13 => segs.push((top, right)),
                3 | 12 => segs.push((left, right)),
                4 | 11 => segs.push((right, bottom)),
                6 | 9 => segs.push((top, bottom)),
                7 | 8 => segs.push((left, bottom)),
                // saddles: the diagonal pair stays apart
                5 => {
                    segs.push((left, top));
                    segs.push((right, bottom));
                }
                10 => {
                    segs.push((top, right));
                    segs.push((left, bottom));
                }
                _ => unreachable!(),
            }
        }
    }
    // join the segments through their shared edges
    let mut at: std::collections::HashMap<i64, Vec<usize>> = Default::default();
    for (i, &(p, q)) in segs.iter().enumerate() {
        at.entry(p).or_default().push(i);
        at.entry(q).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut out = vec![];
    for s0 in 0..segs.len() {
        if used[s0] {
            continue;
        }
        used[s0] = true;
        let mut line = vec![segs[s0].0, segs[s0].1];
        // forward from the end, then backward from the start
        for dir in 0..2 {
            loop {
                let end = if dir == 0 { *line.last().unwrap() } else { line[0] };
                let Some(&next) = at[&end].iter().find(|&&j| !used[j]) else { break };
                used[next] = true;
                let (p, q) = segs[next];
                let other = if p == end { q } else { p };
                if dir == 0 { line.push(other) } else { line.insert(0, other) }
            }
        }
        let closed = line.len() > 2 && line[0] == *line.last().unwrap();
        let mut pts: Vec<Pos> = line
            .iter()
            .map(|&e| {
                let (r, c) = mid(e);
                // padded grid -> fire grid cell centres -> world metres
                Pos { x: (c - 1.0 + 0.5) * world.cellsize, y: world.height_m - (r - 1.0 + 0.5) * world.cellsize }
            })
            .collect();
        let length: f32 = pts.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt()).sum();
        if length < MIN_LOOP_M {
            continue;
        }
        pts = chaikin(&pts, closed);
        out.push(smooth_line(&pts, closed, SMOOTH_SIGMA_M, SMOOTH_STEP_M));
    }
    out
}

/// One round of corner cutting: the staircase of the grid becomes a curve.
fn chaikin(pts: &[Pos], closed: bool) -> Vec<Pos> {
    let lerp = |a: Pos, b: Pos, t: f32| Pos { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
    let n = pts.len();
    let mut out = Vec::with_capacity(n * 2);
    if !closed {
        out.push(pts[0]);
    }
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        out.push(lerp(a, b, 0.25));
        out.push(lerp(a, b, 0.75));
    }
    if closed {
        let first = out[0];
        out.push(first);
    } else {
        out.push(pts[n - 1]);
    }
    out
}

/// Gaussian filter of a polyline's coordinates along its arc length: resampled
/// every `step` metres, each point replaced by the weighted mean of its
/// neighbours within 3 `sigma`. A closed line wraps; an open one keeps its
/// ends where they are.
fn smooth_line(pts: &[Pos], closed: bool, sigma: f32, step: f32) -> Vec<Pos> {
    let dist = |a: Pos, b: Pos| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    let mut cum = vec![0.0f32];
    for w in pts.windows(2) {
        cum.push(cum.last().unwrap() + dist(w[0], w[1]));
    }
    let total = *cum.last().unwrap();
    let n = (total / step).round() as usize;
    if n < 4 {
        return pts.to_vec();
    }
    // uniform resampling (closed: the last point repeats the first, drop it)
    let mut seg = 0;
    let mut res: Vec<Pos> = (0..=n)
        .map(|i| {
            let d = total * i as f32 / n as f32;
            while seg + 2 < cum.len() && cum[seg + 1] < d {
                seg += 1;
            }
            let t = ((d - cum[seg]) / (cum[seg + 1] - cum[seg]).max(1e-6)).clamp(0.0, 1.0);
            Pos { x: pts[seg].x + (pts[seg + 1].x - pts[seg].x) * t, y: pts[seg].y + (pts[seg + 1].y - pts[seg].y) * t }
        })
        .collect();
    if closed {
        res.pop();
    }
    let m = res.len() as isize;
    let h = ((3.0 * sigma / (total / n as f32)).ceil() as isize).max(1);
    let k: Vec<f32> = (-h..=h).map(|j| (-((j as f32 * total / n as f32) / sigma).powi(2) / 2.0).exp()).collect();
    let mut out: Vec<Pos> = (0..m)
        .map(|i| {
            let (mut x, mut y, mut w) = (0.0, 0.0, 0.0);
            for (j, &kj) in k.iter().enumerate() {
                let q = i + j as isize - h;
                let q = if closed { q.rem_euclid(m) } else { q.clamp(0, m - 1) };
                x += kj * res[q as usize].x;
                y += kj * res[q as usize].y;
                w += kj;
            }
            Pos { x: x / w, y: y / w }
        })
        .collect();
    if closed {
        out.push(out[0]);
    } else {
        out[0] = res[0];
        *out.last_mut().unwrap() = *res.last().unwrap();
    }
    out
}
