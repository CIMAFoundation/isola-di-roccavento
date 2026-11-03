//! The kiosk: the flow of a visit, around the one simulation and gameplay
//! authority, [`rocca::Game`]. No rules live here: this module holds what the
//! player is choosing (a team, a hex, a district), asks the game for its
//! previews, gives the orders the player confirms, and runs the clock at the
//! game's speed (docs/01-GIOCO.md).
//!
//! Flow: **Intro** (name, «Inizia») → **Tutorial** (day 1: a scripted small
//! fire, one prompted step at a time; the world stands still while a step
//! waits for the player) → **Scenario** (which fire) →
//! **Gioco** (×120, opens in pause; the pause stops the world, orders only
//! in pause) → **Fine** (stars
//! against the same fire with no orders, the day's board). A new visit starts
//! from the end screen or the operator menu (F2), never by itself.

pub mod board;
pub mod chat;
pub mod characters;
pub mod flags;
pub mod icons;
pub mod lang;
pub mod marks;
pub mod pins;
pub mod screens;
pub mod snap;
pub mod ui;
pub mod view;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::window::WindowMode;
use rocca::tutorial::{Expect, Script};
use rocca::{Axial, EvacPlan, MovePlan, Outcome, Territory};
use scenario::Pos;

use crate::sim::{Sim, SimRestarted};
use crate::{AppState, DataPath};

pub const TITLE: &str = "Isola di Roccavento";

/// The commit this build comes from (`build.rs`), written on the title screen
/// and sent with the feedback.
pub const BUILD: &str = env!("GAME_COMMIT");
/// Real seconds a notice (a refused order, a closed road) stays on screen.
pub const NOTICE_S: f32 = 4.0;

pub fn window_mode() -> WindowMode {
    if cfg!(target_arch = "wasm32") || std::env::var("KIOSK_WINDOWED").is_ok() {
        WindowMode::Windowed
    } else {
        WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The player's name and «Inizia»; the tutorial's fire is on the map, still.
    Intro,
    /// The scripted tutorial.
    Tutorial,
    /// After «Inizia»: which fire to fight, and how hard.
    Scenario,
    /// The played game.
    Gioco,
    /// The end: stars and board.
    Fine,
}

/// What the player has selected on the map or in the side bar.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Selection {
    #[default]
    None,
    Team(usize),
    District(usize),
}

/// Where the camera is flying to: a ground point (`None`: the home view) and
/// the distance as a fraction of the home view's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    pub at: Option<Pos>,
    pub zoom: f32,
}

#[derive(Resource)]
pub struct Kiosk {
    pub phase: Phase,
    /// Real seconds since the phase began.
    pub phase_t: f32,
    pub territory: Territory,
    pub data: PathBuf,
    pub seed: u64,
    /// Operator choice for the next visit/day 2; never changes a running game.
    pub selected_case: String,
    pub player: String,
    /// The tutorial's script and the step on show.
    pub script: Option<Script>,
    pub step: usize,
    /// The camera's flight for the tutorial step on show, and the step it
    /// was given for, and the real seconds since (the highlight is brief).
    pub shot: Option<Shot>,
    pub shot_for: Option<usize>,
    pub step_t: f32,
    pub selection: Selection,
    /// The move being previewed: the team, the hex and the game's answer.
    pub move_plan: Option<Result<MovePlan, String>>,
    pub move_hex: Option<Axial>,
    /// The evacuation being previewed.
    pub evac_plan: Option<Result<EvacPlan, String>>,
    /// The district under the pointer, its evacuation and `Sim::generation`
    /// it was computed at: route and time are seen before choosing.
    pub evac_hover: Option<(usize, u64, Result<EvacPlan, String>)>,
    /// A team's flag being dragged, and the ground under the pointer.
    pub dragging: Option<(usize, Pos)>,
    /// Camera pan direction while a dragged flag is held at a map edge.
    pub drag_pan: Vec2,
    /// The pad moved last, not the mouse: the aim is the centre of the screen.
    pub pad_aim: bool,
    /// `Sim::generation` the previews were computed at.
    plans_at: u64,
    /// A short message (refused order, event) and its age in real seconds.
    pub notice: Option<(String, f32)>,
    /// Log entries already turned into notices.
    pub log_seen: usize,
    /// The same fire with no orders, run beside the game: (case, seed) and
    /// the result when ready.
    pub baseline: Arc<Mutex<Option<(String, u64, Result<Outcome, String>)>>>,
    /// The day's board, and the stamp (day, time) of this game once saved.
    pub board: rocca::score::Board,
    pub saved: Option<(String, u64)>,
    /// Width of the side bar, logical px: the camera frames the map left of it.
    pub side_w: f32,
    /// A ground point clicked on the map this frame (set by [`view::camera`]).
    pub click: Option<Pos>,
    pub follow: bool,
    pub reset_view: bool,
    pub zoom_steps: f32,
    /// Real seconds the opening screen's fire has been on show: the camera
    /// director's clock ([`demo_shot`]).
    pub demo_t: f32,
    /// The title screen («Gioca») is showing over the opening's fire.
    pub title: bool,
    /// The speed the director asks for, eased.
    demo_speed: f32,
    /// The shot being held (its index) and what it looks at.
    pub demo_pick: (usize, Subject),
    /// When ([`Kiosk::demo_t`]) the scripted orders under the opening are next given.
    demo_order_at: f32,
    /// A cut in progress between two distant shots: seconds since it began
    /// and whether the camera has already jumped. Fade to black, jump, fade in.
    pub(super) cut: Option<(f32, bool)>,
    /// The subject the camera is set on (a change of it may start a cut).
    pub(super) cut_subject: Subject,
    /// Black over the picture, 0..1 (drawn by `ui`).
    pub fade: f32,
    /// Real seconds into the opening sequence at launch (the CIMA logo, then
    /// the island; `screens::opening`), `None` once it is over or skipped.
    pub opening: Option<f32>,
    /// Real seconds since the opening ended (`screens::handoff`).
    pub after_opening: f32,
    /// The operator menu (F2).
    pub operator: bool,
    /// The game menu (Esc, or «Menu» bottom left) over the tutorial or the
    /// game: the world stands still while it is open.
    pub menu: bool,
    /// Operator's free camera over the opening's fire: no director, the
    /// fire keeps running, the view is the visitor's own.
    pub freecam: bool,
    /// «Ferma il mondo» (operator).
    pub frozen: bool,
    pub error: Option<String>,
}

impl Kiosk {
    /// How many of the opening's two jingles are due: the first as the logo
    /// fades in, the second as it fades out on the island (or once it is over).

    fn new(territory: Territory, data: PathBuf) -> Kiosk {
        Kiosk {
            phase: Phase::Intro,
            title: true,
            phase_t: 0.0,
            demo_t: 0.0,
            demo_speed: 0.0,
            demo_pick: (usize::MAX, Subject::Wide),
            demo_order_at: 2.0,
            cut: None,
            cut_subject: Subject::Wide,
            fade: 0.0,
            // screenshots start where they are asked to
            opening: (!snap::active()).then_some(0.0),
            after_opening: f32::MAX,
            selected_case: territory.game.clone(),
            territory,
            data,
            seed: 2,
            player: String::new(),
            script: None,
            step: 0,
            shot: None,
            shot_for: None,
            step_t: 0.0,
            selection: Selection::None,
            move_plan: None,
            move_hex: None,
            evac_plan: None,
            evac_hover: None,
            dragging: None,
            drag_pan: Vec2::ZERO,
            pad_aim: false,
            plans_at: 0,
            notice: None,
            log_seen: 0,
            baseline: Default::default(),
            board: board::load(),
            saved: None,
            side_w: 0.0,
            click: None,
            follow: true,
            reset_view: false,
            zoom_steps: 0.0,
            operator: false,
            freecam: false,
            frozen: false,
            menu: false,
            error: None,
        }
    }

    pub fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.phase_t = 0.0;
        self.notice = None;
        self.menu = false;
        self.clear_selection();
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
        self.move_plan = None;
        self.move_hex = None;
        self.evac_plan = None;
        self.dragging = None;
        self.drag_pan = Vec2::ZERO;
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), 0.0));
    }

    pub fn tutorial_case(&self) -> String {
        self.territory.tutorial.as_ref().map_or_else(|| self.territory.game.clone(), |t| t.case.clone())
    }

    /// The tutorial step on show.
    pub fn expect(&self) -> Option<Expect> {
        (self.phase == Phase::Tutorial).then(|| self.script.as_ref().and_then(|s| s.steps.get(self.step)).map(|s| s.expect)).flatten()
    }

    /// Whether the player may do this now: in the tutorial only what the
    /// step asks; in the game anything, the game itself refusing what its
    /// rules do not allow.
    pub fn allows(&self, what: Expect) -> bool {
        match self.phase {
            Phase::Gioco => true,
            Phase::Tutorial => match (self.expect(), what) {
                (Some(Expect::Move(t, _)), Expect::Select(u)) => t == u,
                (Some(Expect::Move(t, at)), Expect::Move(u, bt)) => t == u && at == bt,
                (Some(e), w) => e == w,
                (None, _) => false,
            },
            _ => false,
        }
    }

    /// The tutorial goes on when the player did what the step asked.
    pub fn done(&mut self, what: Expect) {
        if self.phase == Phase::Tutorial && self.expect() == Some(what) {
            self.step += 1;
        }
    }

    /// The no-orders game of `case`, when it is done.
    pub fn baseline(&self) -> Option<Result<Outcome, String>> {
        self.baseline.lock().ok().and_then(|b| b.as_ref().map(|(_, _, r)| r.clone()))
    }

    fn start_baseline(&mut self, case: &str) {
        // A finished worker from a previous game must not replace this result.
        self.baseline = Default::default();
        let slot = self.baseline.clone();
        let (data, case, seed) = (self.data.clone(), case.to_string(), self.seed);
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let r = rocca::Game::without_orders(&data, &case, seed).map_err(|e| format!("{e:#}"));
            if let Ok(mut s) = slot.lock() {
                *s = Some((case, seed, r));
            }
        });
        #[cfg(target_arch = "wasm32")]
        {
            // no threads in the browser: computed at the end ([`step`])
            let _ = (data, case, seed, slot);
        }
    }
}

pub fn launch(data: Res<DataPath>, mut commands: Commands, mut next: ResMut<NextState<AppState>>, mut exit: MessageWriter<AppExit>) {
    lang::init(&data.0);
    let built = Territory::load(&data.0).and_then(|t| {
        let k = Kiosk::new(t, data.0.clone());
        let sim = Sim::new(&data.0, &k.territory.game, k.seed)?;
        Ok((k, sim))
    });
    match built {
        Ok((k, sim)) => {
            commands.insert_resource(k);
            commands.insert_resource(sim);
            next.set(AppState::Playing);
        }
        Err(e) => {
            eprintln!("kiosk: {e:#}");
            exit.write(AppExit::error());
        }
    }
}

/// Real seconds a shot of the opening screen's camera lasts: long enough to
/// see something happen, not a montage.
pub const SHOT_S: f32 = 26.0;

/// What the opening screen's camera looks at; chosen once per shot, by what
/// the fire is doing (`view::pick_shot`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Subject {
    /// The whole burning front and the towns near it.
    Wide,
    /// The front, close.
    Front,
    /// The town the fire is nearest to.
    District(usize),
    /// A ground crew working near the fire.
    Crew(usize),
    /// A sight of the island, now and then between the fire's shots
    /// (`view::scenic_sites`).
    Scenic(usize),
}

impl Subject {
    /// How fast the fire runs while this is on show, against `RUN_X`: fast
    /// over the whole front, slower on a close subject.
    fn pace(self) -> f32 {
        match self {
            Subject::Wide => 1.5,
            Subject::Scenic(_) => 1.0,
            Subject::Front | Subject::District(_) => 0.8,
            Subject::Crew(_) => 0.4,
        }
    }
}

/// The index of the shot at real time `t` of the opening screen.
pub fn demo_shot(t: f32) -> usize {
    (t / SHOT_S) as usize
}

/// The fire behind the name screen: the played case, no orders, from its
/// start; when it has run its course it starts again.
fn demo_restart(k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, case: &str, seed: u64) {
    if let Err(e) = sim.restart(case, seed) {
        k.error = Some(format!("{e:#}"));
        return;
    }
    restarted.write(SimRestarted);
    k.demo_pick = (usize::MAX, Subject::Wide);
    k.cut = Some((0.0, false));
    k.demo_order_at = k.demo_t + 2.0;
}

/// Seconds between the scripted orders under the opening.
const DEMO_ORDERS_S: f32 = 30.0;

/// The game under the opening plays itself: the teams go to the fire, the
/// town nearest to it is evacuated. The camera follows what was ordered (a
/// cut to the team on its way, to the town that is being emptied). The game
/// is still in its opening pause, so orders are allowed; the kiosk's clock
/// runs it anyway.
fn demo_orders(k: &mut Kiosk, sim: &mut Sim) {
    if k.demo_t < k.demo_order_at {
        return;
    }
    k.demo_order_at = k.demo_t + DEMO_ORDERS_S;
    // orders are given in a pause: a free one, over at once
    sim.opening_pause();
    demo_give_orders(k, sim);
    sim.resume();
}

fn demo_give_orders(k: &mut Kiosk, sim: &mut Sim) {
    let cells = sim.fire.active_cells();
    if cells.is_empty() {
        k.demo_order_at = k.demo_t + 3.0;
        return;
    }
    let fire: Vec<scenario::Pos> = cells.iter().step_by((cells.len() / 200).max(1)).map(|&c| sim.scn.world.centre_of(c)).collect();
    let n = sim.teams.len();
    let mut first = None;
    for t in 0..n {
        let p = fire[(t + 1) * fire.len() / (n + 1)];
        if let Some(h) = sim.hexes.nearest(p).map(|h| h.at) {
            match sim.move_team(t, h) {
                Ok(_) if first.is_none() => first = sim.teams[t].crew_units.first().copied(),
                Err(e) => eprintln!("DEMOTMP move {t}: {e}"),
                _ => {}
            }
        }
    }
    let gap = |p: scenario::Pos| fire.iter().map(|f| (f.x - p.x).hypot(f.y - p.y)).fold(f32::MAX, f32::min);
    let near = (0..sim.districts.len()).map(|d| (d, gap(sim.districts[d].centre))).filter(|t| t.1 < 2500.0).min_by(|a, b| a.1.total_cmp(&b.1));
    let mut cut = first.map(Subject::Crew);
    if let Some((d, _)) = near {
        if sim.evacuate(d).is_ok() {
            cut = Some(Subject::District(d));
        }
    }
    eprintln!("DEMOTMP t={} teams={} cut={:?} log={:?}", k.demo_t, n, cut, sim.log.last().map(|l| l.text.clone()));
    if let Some(s) = cut {
        k.demo_pick = (demo_shot(k.demo_t), s);
    }
}

/// A new visit: the played fire runs by itself under the name screen.
pub fn new_visit(k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) {
    let case = k.selected_case.clone();
    demo_restart(k, sim, restarted, &case, k.seed);
    k.demo_t = 0.0;
    k.demo_order_at = 2.0;
    k.demo_speed = 0.0;
    k.player.clear();
    k.saved = None;
    k.log_seen = 0;
    k.script = None;
    k.step = 0;
    k.title = true;
    k.enter(Phase::Intro);
}

/// «Inizia»: the tutorial, or straight to the game if there is none.
pub fn start_tutorial(k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) {
    if k.territory.tutorial.is_none() {
        start_game(k, sim, restarted);
        return;
    }
    let case = k.tutorial_case();
    if sim.case.name != case || sim.time_s() > rocca::STEP_S {
        if let Err(e) = sim.restart(&case, k.seed) {
            k.error = Some(format!("{e:#}"));
        }
        restarted.write(SimRestarted);
    }
    k.script = Script::of(sim);
    // like the game: it opens still
    sim.opening_pause();
    k.step = 0;
    k.shot_for = None;
    k.shot = None;
    k.log_seen = sim.log.len();
    k.enter(Phase::Tutorial);
}

/// «Inizia» and the end of the tutorial lead here: the player picks the fire and the level.
pub fn choose_scenario(k: &mut Kiosk) {
    k.enter(Phase::Scenario);
}

/// The played fire, with its no-orders twin started beside it.
pub fn start_game(k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) {
    let case = k.selected_case.clone();
    if let Err(e) = sim.restart(&case, k.seed) {
        k.error = Some(format!("{e:#}"));
        return;
    }
    restarted.write(SimRestarted);
    k.start_baseline(&case);
    k.saved = None;
    k.log_seen = 0;
    k.script = None;
    k.follow = true;
    // the game opens still: a look at the scene and the first orders
    sim.opening_pause();
    k.enter(Phase::Gioco);
}


/// «Riprova»: the fire just played, same seed, from the start.
pub fn retry(k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) {
    k.selected_case = sim.case.name.clone();
    start_game(k, sim, restarted);
}

/// The clock and the previews. Runs before the view systems.
pub fn step(time: Res<Time>, mut kiosk: ResMut<Kiosk>, mut sim: ResMut<Sim>, mut restarted: MessageWriter<SimRestarted>, mut baseline_job: Local<Option<rocca::Game>>, mario: Res<crate::mario::Playground>) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = &mut baseline_job;
    #[cfg(target_arch = "wasm32")]
    {
        if kiosk.phase != Phase::Fine || kiosk.baseline().is_some() {
            *baseline_job = None;
        } else {
            // Keep the final screen and its buttons responsive. Previously the
            // whole five-hour comparison ran synchronously in this one frame.
            let start = js_sys::Date::now();
            let result = (|| -> anyhow::Result<Option<Outcome>> {
                if baseline_job.is_none() {
                    *baseline_job = Some(rocca::Game::new(&kiosk.data, &sim.case.name, sim.seed)?);
                    return Ok(None);
                }
                let game = baseline_job.as_mut().unwrap();
                while !game.is_over() {
                    game.step()?;
                    if js_sys::Date::now() - start >= 8.0 { return Ok(None); }
                }
                Ok(Some(game.outcome()))
            })();
            let done = match result { Ok(Some(o)) => Some(Ok(o)), Err(e) => Some(Err(format!("{e:#}"))), _ => None };
            if let Some(result) = done {
                if let Ok(mut slot) = kiosk.baseline.lock() { *slot = Some((sim.case.name.clone(), sim.seed, result)); }
                *baseline_job = None;
            }
        }
    }
    let dt = crate::playtest::delta(time.delta_secs());
    let k = &mut *kiosk;
    if !k.frozen && !k.menu {
        k.phase_t += dt;
        k.step_t += dt;
    }
    if let Some((_, age)) = &mut k.notice {
        *age += dt;
        if *age > NOTICE_S {
            k.notice = None;
        }
    }
    // the opening screen: the fire plays, at the pace of the shot on show
    if k.phase == Phase::Intro && !k.frozen && !mario.active {
        k.demo_t += dt;
        let want = rocca::RUN_X * k.demo_pick.1.pace();
        k.demo_speed += (want - k.demo_speed) * (1.0 - (-1.5 * dt.min(0.1)).exp());
        demo_orders(k, &mut sim);
        if sim.is_over() && k.demo_t > 3.0 {
            let (case, seed) = (sim.case.name.clone(), sim.seed);
            demo_restart(k, &mut sim, &mut restarted, &case, seed);
        }
    }
    sim.speed = match k.phase {
        _ if k.frozen || k.menu => 0.0,
        // Mario chooses how fast the world moves (0 = still)
        _ if mario.active => mario.simulation_speed,
        Phase::Intro => k.demo_speed,
        // the tutorial waits for the player, then runs by itself
        Phase::Tutorial if matches!(k.expect(), Some(Expect::Over | Expect::Run(_))) => sim.game.speed(),
        Phase::Gioco => sim.game.speed(),
        _ => 0.0,
    };
    if sim.speed > 0.0 {
        if let Err(e) = crate::playtest::tick(&mut sim, dt) {
            k.error = Some(format!("{e:#}"));
        }
    }
    // the tutorial's run is over: the game stops by itself
    if let (Phase::Tutorial, Some(Expect::Run(until))) = (k.phase, k.expect()) {
        if sim.time_s() >= until {
            sim.opening_pause();
            k.done(Expect::Run(until));
        }
    }
    // what happened goes to the group chat (`chat.rs`); the log only counts
    if sim.log.len() > k.log_seen {
        k.log_seen = sim.log.len();
    }
    match k.phase {
        Phase::Tutorial if k.expect() == Some(Expect::Over) && (sim.is_over() || sim.game.evacuation_finished()) => k.enter(Phase::Scenario),
        Phase::Gioco if sim.is_over() => {
            k.enter(Phase::Fine);
        }
        _ => {}
    }
    // the pause is over: nothing stays half-chosen
    if k.phase == Phase::Gioco && !sim.paused() && k.selection != Selection::None {
        k.clear_selection();
    }
    // previews follow the world (roads close, teams move)
    let stale = sim.generation >= k.plans_at.saturating_add(10);
    if stale {
        k.plans_at = sim.generation;
        if let (Selection::Team(t), Some(at), Some(_)) = (k.selection, k.move_hex, &k.move_plan) {
            k.move_plan = Some(sim.plan_move(t, at));
        }
        if let (Selection::District(d), Some(_)) = (k.selection, &k.evac_plan) {
            k.evac_plan = Some(sim.plan_evacuation(d));
        }
    }
}

#[cfg(test)]
mod scenario_selection_tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    #[test]
    fn scenario_selection_applies_on_restart_and_keeps_the_default() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let territory = Territory::load(&data).unwrap();
        assert_eq!(territory.game, "medium");
        assert_eq!(territory.tutorial.as_ref().unwrap().case, "tutorial");
        assert_eq!(territory.levels.len(), 4);
        for name in ["medium", "impatti_valico_due_lati_30_6",
            "impatti_nuclei_valico_costante_lungo_28_7_0_60",
            "impatti_nuclei_est_giro_lungo_28_6_75_60"] {
            assert!(territory.case(name).is_some());
            assert_eq!(territory.levels.iter().filter(|l| l.case == name).count(), 1);
        }
        let mut sim = Sim::new(&data, &territory.game, 2).unwrap();
        let mut k = Kiosk::new(territory, data);
        assert_eq!(k.selected_case, "medium");
        let next = "impatti_valico_due_lati_30_6";
        k.selected_case = next.into();
        assert_eq!(sim.case.name, "medium", "selecting alone leaves the running fire intact");
        let previous = k.baseline.clone();
        let mut world = World::new();
        world.init_resource::<Messages<SimRestarted>>();
        let mut state = SystemState::<MessageWriter<SimRestarted>>::new(&mut world);
        let mut events = state.get_mut(&mut world).unwrap();
        start_game(&mut k, &mut sim, &mut events);
        assert_eq!(sim.case.name, next);
        assert_eq!(k.phase, Phase::Gioco);
        assert!(sim.paused());
        assert!(!Arc::ptr_eq(&previous, &k.baseline), "older baseline workers are isolated");
        k.selected_case = "medium".into();
        assert_eq!(sim.case.name, next, "a pending selection does not change score scope");
        let (active, seed) = (sim.case.name.clone(), sim.seed);
        demo_restart(&mut k, &mut sim, &mut events, &active, seed);
        assert_eq!(sim.case.name, next, "the automatic demo loop keeps the active case");
        new_visit(&mut k, &mut sim, &mut events);
        assert_eq!(sim.case.name, "medium");
        assert_eq!(k.phase, Phase::Intro);
        assert_eq!(k.territory.game, "medium");
        assert_eq!(k.tutorial_case(), "tutorial");
    }

    #[test]
    fn demo_orders_get_the_teams_moving() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let territory = Territory::load(&data).unwrap();
        let mut sim = Sim::new(&data, &territory.game, 2).unwrap();
        let mut k = Kiosk::new(territory, data);
        sim.opening_pause();
        k.demo_t = 3.0;
        k.demo_order_at = 2.0;
        for _ in 0..40 { sim.speed = 300.0; sim.tick(0.1).unwrap(); }
        demo_orders(&mut k, &mut sim);
        eprintln!("paused={} t={} log={:?}", sim.paused(), sim.time_s(), sim.log.iter().map(|l| l.text.clone()).collect::<Vec<_>>());
        let before: Vec<_> = sim.teams.iter().map(|t| sim.crews.units[t.unit].pos).collect();
        for _ in 0..200 { sim.speed = 300.0; sim.tick(0.1).unwrap(); }
        let after: Vec<_> = sim.teams.iter().map(|t| sim.crews.units[t.unit].pos).collect();
        eprintln!("{before:?}\n{after:?}");
        assert!(before != after);
    }
}
