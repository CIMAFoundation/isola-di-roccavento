//! The kiosk's interface (egui): few words, big buttons (docs/01-GIOCO.md §6).
//!
//! - bottom bar, centred: day, clock and the pause («Chiedi all'esperto»);
//!   wind and fuel moisture bottom right; the group chat top left;
//! - side bar: the teams and the districts, and what is selected: the move or
//!   the evacuation with its time, to confirm;
//! - on the map: a badge per team and per district (clickable), the notice
//!   of the last event;
//! - the tutorial's step, the opening, day 2 and the end screens.
//!
//! Every order goes through the game (`rocca::Game::move_team`,
//! `rocca::Game::evacuate`), which refuses what its rules do not allow; the
//! tutorial also refuses what its step does not ask (`Kiosk::allows`).

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, RichText};
use bevy_egui::EguiContexts;
use rocca::tutorial::Expect;
use scenario::Pos;

use super::characters::{Logo, Portraits, ScenarioIcons};
use super::{screens, Kiosk, Phase, Selection};
use crate::sim::{Sim, SimRestarted};

const PANEL: Color32 = Color32::from_rgba_premultiplied(18, 20, 24, 245);
pub(super) const AMBER: Color32 = Color32::from_rgb(250, 195, 70);
pub(super) const RED: Color32 = Color32::from_rgb(255, 105, 85);
const BLUE: Color32 = Color32::from_rgb(110, 185, 255);
const GREEN: Color32 = Color32::from_rgb(110, 230, 150);
/// The evacuation's colour, the same as its route on the map (`marks::EVAC`).
const EVAC: Color32 = Color32::from_rgb((super::marks::EVAC[0] * 255.0) as u8, (super::marks::EVAC[1] * 255.0) as u8, (super::marks::EVAC[2] * 255.0) as u8);
pub(super) const GREY: Color32 = Color32::from_rgb(205, 208, 212);

/// The colour of team `t`: the same on its badge, cone, zone and road.
pub(super) fn team_colour(t: usize) -> Color32 {
    let c = super::marks::team_rgb(t);
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

/// Use the same layout in either orientation, including short phone viewports.
pub(super) fn compact(ctx: &egui::Context) -> bool {
    let r = ctx.viewport_rect();
    r.width() < 1000.0 || r.height() < 600.0
}

fn clock(s: i64) -> String {
    rocca::words::clock(s)
}

pub(super) fn secondary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(17.0).color(Color32::WHITE))
        .fill(Color32::from_gray(62))
        .stroke(egui::Stroke::new(1.0, Color32::from_gray(110)))
        .corner_radius(8.0)
        .min_size(egui::vec2(44.0, 40.0))
}

pub(super) fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(20.0).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(10.0).min_size(egui::vec2(140.0, 46.0))
}

fn panel() -> egui::Frame {
    egui::Frame::NONE.fill(PANEL).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8))
}

pub(super) fn card(tint: Color32) -> egui::Frame {
    egui::Frame::NONE.fill(PANEL).stroke(egui::Stroke::new(1.5, tint.gamma_multiply(0.8))).corner_radius(14.0).inner_margin(18.0)
}

/// Expanding rings that repeat for as long as the step waits: on the team to
/// drag and on the place to drop it. The map marks alone were hard to find.
fn tutorial_pulse(ctx: &egui::Context, k: &Kiosk, sim: &Sim, cam: (&Camera, &GlobalTransform), placed: &Placed) {
    let Some(Expect::Move(t, at)) = k.expect() else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("tutorial_pulse")));
    let time = ctx.input(|i| i.time) as f32;
    let ring = |centre: egui::Pos2, base: f32| {
        // three rings a third of a period apart, each growing and fading out
        for i in 0..3 {
            let phase = (time * 0.8 + i as f32 / 3.0).fract();
            let radius = base * (0.5 + 2.5 * phase);
            let a = 0.3 + 0.7 * (1.0 - phase);
            painter.circle_stroke(centre, radius, egui::Stroke::new(10.0, Color32::from_black_alpha((190.0 * a) as u8)));
            painter.circle_stroke(centre, radius, egui::Stroke::new(6.0, AMBER.gamma_multiply(a)));
        }
        painter.circle_filled(centre, base * 0.35, AMBER.gamma_multiply(0.55 + 0.35 * (time * 5.0).sin().abs()));
    };
    let spot = sim.hexes.get(at).map_or(at.centre(), |h| h.target);
    if let Some(p) = screen(cam, sim, spot, 20.0) {
        ring(p, 26.0);
    }
    // the team's badge: where to start the drag
    if k.selection != Selection::Team(t) && k.dragging.is_none() {
        if let Some(Some(p)) = placed.team.get(t) {
            ring(*p, 26.0);
        }
    }
    ctx.request_repaint();
}

/// World point to screen point, `lift_m` above the ground.
pub(super) fn screen(cam: (&Camera, &GlobalTransform), sim: &Sim, p: Pos, lift_m: f32) -> Option<egui::Pos2> {
    let v = crate::frame::to_bevy(p, sim.scn.terrain.height_at(p) + lift_m);
    let s = cam.0.world_to_viewport(cam.1, v).ok()?;
    Some(egui::pos2(s.x, s.y))
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut contexts: EguiContexts,
    mut kiosk: ResMut<Kiosk>,
    mut sim: ResMut<Sim>,
    mut focus: ResMut<crate::ui::UiFocus>,
    mut restarted: MessageWriter<SimRestarted>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::camera::OrbitCamera>>,
    (mut gfx, mut audio, time): (ResMut<crate::quality::Graphics>, ResMut<crate::sound::AudioPrefs>, Res<Time>),
    faces: Option<Res<Portraits>>,
    logo: Option<Res<Logo>>,
    scenario_icons: Option<Res<ScenarioIcons>>,
    mario: Res<crate::mario::Playground>,
    pad: Res<crate::pad::PadView>,
    pads: Query<(), With<Gamepad>>,
    mut pad_mode: ResMut<crate::pad::PadMode>,
    auto: Option<Res<crate::adaptive::Adaptive>>,
    mut commands: Commands,
) {
    crate::playtest::begin();
    let Ok(ctx) = contexts.ctx_mut() else { return };
    // Mario has the screen: his own panel is all there is
    if mario.active {
        focus.pointer = ctx.is_pointer_over_egui();
        return;
    }
    let k = &mut *kiosk;
    k.drag_pan = Vec2::ZERO;
    let Ok(cam) = cameras.single() else { return };
    if ctx.global_style().visuals.interact_cursor.is_none() {
        ctx.global_style_mut(|s| {
            s.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
            s.animation_time = 0.0;
            s.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(3.0, Color32::WHITE);
        });
    }
    if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::F2)) {
        k.operator = !k.operator;
    }
    // Klingon, hidden: Ctrl+Shift+K
    if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::K)) {
        super::lang::toggle_klingon();
    }
    egui::Area::new(egui::Id::new("menu_btn")).anchor(Align2::LEFT_BOTTOM, [10.0, -10.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.horizontal(|ui| {
            let r = ui.add(secondary(if k.operator { rocca::tr!("Chiudi impostazioni") } else { rocca::tr!("Impostazioni") }));
            crate::playtest::control("Menu", &r, true);
            if r.clicked() {
                k.operator = !k.operator;
            }
            let r = mute_button(ui, &mut audio);
            crate::playtest::control("Audio", &r, true);
        });
    });
    // On a phone the menu is a page of its own: nothing is drawn under it.
    if k.operator && compact(ctx) {
        k.side_w = 0.0;
    } else {
    match k.phase {
        Phase::Intro if k.freecam => k.side_w = 0.0,
        Phase::Intro => {
            k.side_w = 0.0;
            if cfg!(target_arch = "wasm32") && !compact(ctx) { banner(ctx); }
            if k.title {
                // the opening sequence, once at launch; a click or a key skips
                // it (a phone goes straight to the title)
                if let Some(t) = k.opening {
                    let skip = compact(ctx) || ctx.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
                    // a snapshot holds the moment it asks for; a recording runs
                    // it on the game's clock, one frame's step at a time
                    let t = if super::snap::active() && !crate::record::running() { t } else { t + time.delta_secs().min(0.1) };
                    k.opening = (!skip && t < screens::OPENING_S).then_some(t);
                    screens::opening(ctx, t, logo.as_deref());
                }
                let alpha = screens::title_alpha(k.opening);
                if alpha > 0.0 && screens::title(ctx, k, &sim, logo.as_deref(), alpha) && k.opening.is_none() {
                    k.title = false;
                }
            } else {
                screens::intro(ctx, k, &mut sim, &mut restarted, logo.as_deref(), faces.as_deref());
            }
        }
        Phase::Scenario => {
            k.side_w = 0.0;
            screens::scenario(ctx, k, &mut sim, &mut restarted, scenario_icons.as_deref());
        }
        Phase::Fine => {
            k.side_w = 0.0;
            if cfg!(target_arch = "wasm32") && !compact(ctx) {
                banner(ctx);
            }
            screens::end(ctx, k, &mut sim, &mut restarted, faces.as_deref());
        }
        Phase::Tutorial | Phase::Gioco => {
            let r = ctx.viewport_rect();
            k.side_w = if compact(ctx) && r.width() > r.height() && k.phase == Phase::Tutorial { (r.width() * 0.37).clamp(220.0, 316.0) } else { 0.0 };
            top_bar(ctx, k, &mut sim, cam);
            // Esc takes back a choice first (and is then used up: in the tutorial it would also skip)
            if !ctx.egui_wants_keyboard_input() && k.selection != Selection::None && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                k.clear_selection();
            }
            map_badges(ctx, k, &mut sim, cam);
            if ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                k.pad_aim = false;
            }
            if pad.active() || pad.team != 0 || pad.district != 0 || pad.confirm || pad.evacuate {
                k.pad_aim = true;
            }
            let pad_aim = k.pad_aim && pads.iter().next().is_some();
            pad_orders(k, &mut sim, cam, &pad, pad_aim, ctx);
            if !pad_aim {
                hover_move(ctx, k, &sim, cam);
            }
            map_click(k, &mut sim, ctx.input(|i| i.has_touch_screen()));
            if k.phase == Phase::Gioco {
                offscreen_alerts(ctx, k, &sim, cam);
            }
            if k.phase == Phase::Tutorial {
                screens::tutorial(ctx, k, &mut sim, &mut restarted, faces.as_deref(), cam);
            }
            notice(ctx, k);
            if k.phase == Phase::Gioco {
                super::chat::show(ctx, &sim, faces.as_deref(), k.notice.is_some());
            }
            if compact(ctx) {
                egui::Area::new(egui::Id::new("map_tools")).anchor(Align2::RIGHT_BOTTOM, [-8.0 - k.side_w, -8.0]).order(egui::Order::Foreground).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (label, zoom) in [("−", -1.0), ("+", 1.0)] {
                            let r = ui.add(secondary(label).min_size(egui::vec2(44.0, 44.0)));
                            crate::playtest::control(label, &r, true);
                            if r.clicked() { k.zoom_steps += zoom; }
                        }
                        let r = ui.add(secondary(rocca::tr!("Isola")));
                        crate::playtest::control("Isola", &r, true);
                        if r.clicked() { k.reset_view = true; }
                    });
                });
            }
        }
    }
    }
    let mut pad_h = 0.0;
    // a pad left plugged in stays out of the snapshots (`KIOSK_SNAP`)
    if pads.iter().next().is_some() && std::env::var_os("KIOSK_SNAP").is_none() {
        pad_h = pad_help(ctx, k, &sim) + 8.0;
        if matches!(k.phase, Phase::Tutorial | Phase::Gioco) {
            pad_list(ctx, k, &sim);
        }
    }
    // the flags, bottom right, before the game only (2026-10-30): during play
    // the corner holds the weather, and the chat keeps the language it was
    // written in; the settings menu still has the list
    if k.phase == Phase::Intro && !k.freecam && !(k.operator && compact(ctx)) {
        super::lang::button(ctx, pad_h);
    }
    if k.operator {
        operator(ctx, k, &mut gfx, &mut audio, auto.as_deref(), &mut commands);
    }
    pad_mode.game = matches!(k.phase, Phase::Tutorial | Phase::Gioco);
    focus.pointer = ctx.is_pointer_over_egui();
    focus.keyboard = ctx.egui_wants_keyboard_input();
    if k.fade > 0.0 && k.phase == Phase::Intro {
        let a = (k.fade.clamp(0.0, 1.0) * 255.0) as u8;
        ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("cut_fade")))
            .rect_filled(ctx.viewport_rect(), 0.0, Color32::from_black_alpha(a));
    }
    k.click = None;
}

/// The Google Form for the feedback of those who play in the browser, with the
/// «Versione del gioco» answer filled in by the build's commit.
pub(super) fn feedback_url() -> String {
    format!("https://docs.google.com/forms/d/e/1FAIpQLSfHWmJHJwLLzqLmjomK-HA3YikUQfw4wz5L9cSjqpgS4i1ipw/viewform?usp=pp_url&entry.2126742218={}", super::BUILD)
}

/// The banner at the top right, in the browser: a button to the feedback form,
/// on the title, the opening and the end.
fn banner(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("provami")).anchor(Align2::RIGHT_TOP, [-14.0, 14.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        card(AMBER).inner_margin(10.0).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(rocca::tr!("Dicci cosa ne pensi")).size(20.0).strong().color(AMBER));
                ui.add_space(4.0);
                if ui.add(primary(rocca::tr!("Dai un feedback"))).clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(feedback_url()));
                }
            });
        });
    });
}

// --- the pause and the weather ------------------------------------------------

fn top_bar(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform)) {
    if compact(ctx) {
        compact_top(ctx, k, sim, cam);
        return;
    }
    egui::Area::new(egui::Id::new("barra")).anchor(Align2::CENTER_BOTTOM, [0.0, -10.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        panel().show(ui, |ui| {
            ui.horizontal(|ui| {
                pause_dial(ui, k, sim);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.set_min_width(300.0);
                                        ui.label(RichText::new(clock(sim.time_s())).size(34.0).strong().color(Color32::WHITE));
                    // the scenario's progress: elapsed time as a bar, with the end hour
                    let frac = (sim.time_s() as f32 / sim.case.duration_s().max(1) as f32).clamp(0.0, 1.0);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(clock(0)).size(16.0).color(GREY));
                        let w = (ui.available_width() - 70.0).max(40.0);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 7.0, Color32::from_gray(60));
                        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(r.width() * frac, r.height())), 7.0, AMBER);
                        ui.label(RichText::new(clock(sim.case.duration_s())).size(16.0).color(GREY));
                    });
                });
            });
        });
    });
    conditions(ctx, sim, cam);
}

/// Wind and fuel moisture, big, bottom right. The wind's arrow and the north
/// mark turn with the camera, so they match what the map shows.
fn conditions(ctx: &egui::Context, sim: &Sim, cam: (&Camera, &GlobalTransform)) {
    use super::icons::{self, Icon};
    let w = sim.weather();
    egui::Area::new(egui::Id::new("condizioni")).anchor(Align2::RIGHT_BOTTOM, [-12.0, -12.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        panel().show(ui, |ui| {
            ui.horizontal(|ui| {
                let size = 84.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                let c = r.center();
                ui.painter().circle(c, size * 0.5, Color32::from_black_alpha(90), egui::Stroke::new(2.0, Color32::from_gray(110)));
                // north on the screen, from the camera
                let right: Vec3 = cam.1.right().into();
                let fwd: Vec3 = cam.1.forward().into();
                let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
                let north = Vec3::new(0.0, 0.0, -1.0);
                let n = egui::vec2(north.dot(right), -north.dot(fwd)).normalized();
                ui.painter().text(c + n * size * 0.40, Align2::CENTER_CENTER, "N", egui::FontId::proportional(15.0), GREY);
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(size * 0.17)).layout(*ui.layout()));
                wind_arrow(&mut inner, cam, w.wind_dir_deg, size * 0.66, Color32::WHITE);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{:.0} km/h", w.wind_speed_kmh)).size(32.0).strong().color(Color32::WHITE));
                    ui.label(RichText::new(rocca::tr!("da {0}", rocca::words::compass(w.wind_dir_deg))).size(15.0).color(GREY));
                });
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        icons::show(ui, Icon::Drop, 30.0, BLUE);
                        ui.label(RichText::new(format!("{:.0} %", w.moisture_pct)).size(32.0).strong().color(Color32::WHITE));
                    });
                    ui.label(RichText::new(rocca::tr!("umidità")).size(15.0).color(GREY));
                });
            });
            // the forecast, while paused: the lines' legend and the wind to come
            if sim.paused() {
                ui.separator();
                ui.horizontal(|ui| {
                    for (col, text) in crate::forecast_view::LINE_COLOURS.into_iter().zip([rocca::tr!("1 h"), rocca::tr!("3 h")]) {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::hover());
                        ui.painter().line_segment([r.left_center(), r.right_center()], egui::Stroke::new(5.0, col));
                        ui.label(RichText::new(text).size(20.0).strong().color(col));
                        ui.add_space(4.0);
                    }
                    if let Some((at, w2)) = sim.weather_ahead() {
                        ui.add_space(10.0);
                        icons::show(ui, Icon::Clock, 24.0, AMBER);
                        ui.label(RichText::new(clock(at)).size(20.0).strong().color(AMBER));
                        wind_arrow(ui, cam, w2.wind_dir_deg, 30.0, AMBER);
                        ui.label(RichText::new(format!("{:.0} km/h", w2.wind_speed_kmh)).size(20.0).strong().color(AMBER));
                    }
                });
            }
        });
    });
}

/// An arrow the way the wind blows, as seen on the map from the camera.
fn wind_arrow(ui: &mut egui::Ui, cam: (&Camera, &GlobalTransform), from_deg: f64, size: f32, col: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let to = (from_deg as f32 + 180.0).to_radians();
    // world east/north to Bevy (x, -z), then onto the screen's right and up
    let d = Vec3::new(to.sin(), 0.0, -to.cos());
    let right: Vec3 = cam.1.right().into();
    let fwd: Vec3 = cam.1.forward().into();
    let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
    let v = egui::vec2(d.dot(right), -d.dot(fwd)).normalized();
    let c = r.center();
    let (tip, tail) = (c + v * size * 0.45, c - v * size * 0.45);
    let n = egui::vec2(-v.y, v.x);
    let p = ui.painter();
    p.line_segment([tail, tip - v * size * 0.2], egui::Stroke::new(size * 0.12, col));
    p.add(egui::Shape::convex_polygon(vec![tip, tip - v * size * 0.35 + n * size * 0.22, tip - v * size * 0.35 - n * size * 0.22], col, egui::Stroke::NONE));
}

/// An arc from the top, clockwise, over `frac` of the circle.
fn arc(p: &egui::Painter, c: egui::Pos2, radius: f32, from: f32, frac: f32, stroke: egui::Stroke) {
    let n = ((frac * 64.0).ceil() as usize).max(1);
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (from + frac.clamp(0.0, 1.0) * i as f32 / n as f32);
            c + egui::vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    p.add(egui::Shape::line(pts, stroke));
}

const DIAL: f32 = 112.0;

/// The pause, a big round button: ready (amber ring, pause symbol) or in
/// pause (no time limit: play symbol, green ring). No wait between pauses.
fn pause_dial(ui: &mut egui::Ui, k: &mut Kiosk, sim: &mut Sim) {
    let dial = if compact(ui.ctx()) { 56.0 } else { DIAL };
    use super::icons::{self, Icon};
    let paused = sim.paused();
    let enabled = if paused { k.allows(Expect::Resume) } else { sim.can_pause() && k.allows(Expect::Pause) };
    let (r, resp) = ui.allocate_exact_size(egui::vec2(dial, dial), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let p = ui.painter();
    let c = r.center();
    let radius = dial * 0.5 - 7.0;
    let hot = resp.hovered() && enabled;
    p.circle_filled(c, dial * 0.5, if hot { Color32::from_gray(60) } else { Color32::from_black_alpha(90) });
    p.circle_stroke(c, radius, egui::Stroke::new(10.0, Color32::from_gray(55)));
    let (frac, ring, icon, icon_col) = if paused {
        (1.0, GREEN, Icon::Play, GREEN)
    } else {
        (1.0, AMBER, Icon::Pause, AMBER)
    };
    arc(p, c, radius, 0.0, frac, egui::Stroke::new(10.0, ring));
    icons::draw(p, egui::Rect::from_center_size(c, egui::vec2(dial * 0.36, dial * 0.36)), icon, icon_col);
    if hot {
        p.circle_stroke(c, dial * 0.5, egui::Stroke::new(2.5, Color32::WHITE));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    crate::playtest::control(if paused { "Riprendi" } else { "Pausa" }, &resp, enabled);
    let space = enabled && !ui.ctx().egui_wants_keyboard_input() && ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
    if resp.clicked() || space {
        if paused {
            sim.resume();
            k.clear_selection();
            k.done(Expect::Resume);
        } else {
            match sim.pause() {
                Ok(()) => k.done(Expect::Pause),
                Err(e) => k.notify(e),
            }
        }
    }
}

/// A small icon and a short text, side by side.
fn chip(ui: &mut egui::Ui, icon: super::icons::Icon, text: &str, col: Color32) {
    super::icons::show(ui, icon, 22.0, col);
    ui.add(egui::Label::new(RichText::new(text).size(18.0).strong().color(col)).wrap());
}

/// Choosing a team or a district while the world runs stops it: orders are
/// given in the pause (user, 2026-10-16). The tutorial has its own pause step.
fn auto_pause(k: &mut Kiosk, sim: &mut Sim) {
    if k.phase == Phase::Gioco && !sim.paused() {
        if let Err(e) = sim.pause() {
            k.notify(e);
        }
    }
}

/// In the game the world runs again as soon as an order is given (user,
/// 2026-10-17). The tutorial keeps its own «Riprendi» step.
fn resume_after_order(k: &Kiosk, sim: &mut Sim) {
    if k.phase == Phase::Gioco && sim.paused() {
        sim.resume();
    }
}

fn select_team(k: &mut Kiosk, sim: &mut Sim, t: usize) {
    if !k.allows(Expect::Select(t)) {
        k.notify(tutorial_hint(k));
        return;
    }
    auto_pause(k, sim);
    k.clear_selection();
    k.selection = Selection::Team(t);
    k.follow = false;
    k.shot = None;
    k.done(Expect::Select(t));
}

fn select_district(k: &mut Kiosk, sim: &mut Sim, d: usize) {
    if k.phase == Phase::Tutorial && !matches!(k.expect(), Some(Expect::Evacuate(e)) if e == d) {
        k.notify(tutorial_hint(k));
        return;
    }
    auto_pause(k, sim);
    k.clear_selection();
    k.selection = Selection::District(d);
    k.evac_plan = Some(sim.plan_evacuation(d));
}

fn tutorial_hint(k: &Kiosk) -> String {
    match k.expect() {
        Some(Expect::Select(_)) | Some(Expect::Move(..)) => rocca::tr!("Segui l'istruzione: la squadra e la destinazione indicate.").into(),
        Some(Expect::Evacuate(_)) => rocca::tr!("Segui l'istruzione: il borgo indicato.").into(),
        _ => rocca::tr!("Segui l'istruzione in basso.").into(),
    }
}

/// Preview the move of team `t` to the hex `at` (under the dragged flag, or
/// clicked): the tutorial refuses a hex its step does not ask.
fn preview_move(k: &mut Kiosk, sim: &Sim, t: usize, at: rocca::Axial) {
    if k.move_hex == Some(at) {
        return;
    }
    k.move_hex = Some(at);
    k.move_plan = Some(match k.expect() {
        Some(Expect::Move(_, want)) if at != want => Err(rocca::tr!("usa la destinazione evidenziata").into()),
        _ => sim.plan_move(t, at),
    });
}

/// Send team `t` to hex `at` now: there is no confirmation (user,
/// 2026-10-14); an order the game refuses is told and forgotten.
fn give_move(k: &mut Kiosk, sim: &mut Sim, t: usize, at: rocca::Axial) {
    preview_move(k, sim, t, at);
    let result = match k.move_plan.clone() {
        Some(Ok(_)) if !k.allows(Expect::Move(t, at)) => Err(tutorial_hint(k)),
        Some(Ok(_)) => sim.move_team(t, at).map(|_| resume_after_order(k, sim)),
        Some(Err(e)) => Err(e),
        None => Ok(()),
    };
    match result {
        Ok(()) => { k.notice = None; k.done(Expect::Move(t, at)); }
        Err(e) => k.notify(e),
    }
    k.clear_selection();
}

/// The gamepad's controls in the game (2026-10-17), as the mouse's calls: the
/// d-pad left/right chooses a team and up/down a district, Y stops or starts
/// the world, RB shows the island. With a team
/// chosen the aim is the centre of the screen, moved with the camera, and A
/// sends the team there; with a district chosen A (or X) evacuates it.
fn pad_orders(k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform), pad: &crate::pad::PadView, connected: bool, ctx: &egui::Context) {
    if pad.pause && k.phase == Phase::Gioco {
        if sim.paused() {
            k.clear_selection();
            sim.resume();
        } else {
            auto_pause(k, sim);
        }
    }
    if pad.home {
        k.reset_view = true;
    }
    let (teams, districts) = (sim.teams.len() as i32, sim.districts.len() as i32);
    // the camera flies to what is chosen
    if pad.team != 0 && teams > 0 {
        let t = match k.selection {
            Selection::Team(t) => (t as i32 + pad.team).rem_euclid(teams),
            _ if pad.team > 0 => 0,
            _ => teams - 1,
        };
        select_team(k, sim, t as usize);
        if let Selection::Team(t) = k.selection {
            let team = &sim.teams[t];
            let at = team.hex.map_or(Pos::from(sim.territory.stations[team.station].pos), |h| h.centre());
            k.shot = Some(super::Shot { at: Some(at), zoom: 0.5 });
        }
    }
    if pad.district != 0 && districts > 0 {
        let d = match k.selection {
            Selection::District(d) => (d as i32 + pad.district).rem_euclid(districts),
            _ if pad.district > 0 => 0,
            _ => districts - 1,
        };
        select_district(k, sim, d as usize);
        if k.selection == Selection::District(d as usize) {
            k.shot = Some(super::Shot { at: Some(sim.districts[d as usize].centre), zoom: 0.5 });
        }
    }
    if !sim.paused() {
        return;
    }
    match k.selection {
        Selection::Team(t) if connected => {
            let r = ctx.viewport_rect();
            let centre = egui::pos2((r.width() - k.side_w) * 0.5, r.height() * 0.5);
            if let Some(g) = ground_at(sim, cam, centre) {
                preview_move(k, sim, t, rocca::Axial::of(snap_to_target(sim, t, g)));
            }
            aim(ctx, centre);
            if let (true, Some(at)) = (pad.confirm, k.move_hex) {
                give_move(k, sim, t, at);
            }
        }
        Selection::District(d) if pad.evacuate || pad.confirm => {
            if k.evac_plan.as_ref().is_some_and(|p| p.is_ok()) && k.allows(Expect::Evacuate(d)) {
                match sim.evacuate(d) {
                    Ok(_) => {
                        k.done(Expect::Evacuate(d));
                        resume_after_order(k, sim);
                        k.clear_selection();
                    }
                    Err(e) => k.notify(e),
                }
            }
        }
        _ => {}
    }
}

/// What the buttons do now, shown while a gamepad is connected (2026-10-17).
/// Returns the panel's height.
fn pad_help(ctx: &egui::Context, k: &Kiosk, sim: &Sim) -> f32 {
    let rows: &[(&str, Color32, &str)] = match k.selection {
        Selection::Team(_) if sim.paused() => &[
            ("Stick sx", Color32::WHITE, rocca::tr!("porta il mirino sul punto")),
            ("A", GREEN, rocca::tr!("manda la squadra")),
            ("B", RED, rocca::tr!("annulla")),
            ("◀ ▶", Color32::WHITE, rocca::tr!("altra squadra")),
            ("▲ ▼", Color32::WHITE, rocca::tr!("scegli il borgo")),
        ],
        Selection::District(_) => &[("A", BLUE, rocca::tr!("evacua")), ("B", RED, rocca::tr!("annulla")), ("▲ ▼", Color32::WHITE, rocca::tr!("altro borgo")), ("◀ ▶", Color32::WHITE, rocca::tr!("scegli la squadra"))],
        _ if matches!(k.phase, Phase::Tutorial | Phase::Gioco) => &[
            ("◀ ▶", Color32::WHITE, rocca::tr!("scegli la squadra")),
            ("▲ ▼", Color32::WHITE, rocca::tr!("scegli il borgo")),
            ("Y", AMBER, rocca::tr!("ferma / riprendi")),
            ("Stick sx", Color32::WHITE, rocca::tr!("muovi la mappa")),
            ("Stick dx", Color32::WHITE, rocca::tr!("ruota e inclina")),
            ("LT RT", Color32::WHITE, rocca::tr!("zoom")),
            ("RB", Color32::WHITE, rocca::tr!("tutta l'isola")),
            ("LB + stick", Color32::WHITE, rocca::tr!("cursore")),
        ],
        _ => &[("Stick", Color32::WHITE, rocca::tr!("cursore")), ("A", GREEN, rocca::tr!("clic"))],
    };
    egui::Area::new(egui::Id::new("pad_help")).anchor(Align2::RIGHT_BOTTOM, [-10.0, -10.0]).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
        panel().show(ui, |ui| {
            ui.label(RichText::new(rocca::tr!("Gamepad")).size(15.0).strong().color(Color32::from_gray(200)));
            for (key, col, what) in rows {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!(" {key} ")).size(16.0).strong().color(Color32::BLACK).background_color(*col));
                    ui.label(RichText::new(*what).size(16.0).color(Color32::WHITE));
                });
            }
        });
    }).response.rect.height()
}

/// The teams, then the districts, with the chosen one outlined.
fn pad_list(ctx: &egui::Context, k: &Kiosk, sim: &Sim) {
    let teams = sim.teams.len();
    let max_h = (ctx.viewport_rect().height() * 0.55).max(120.0);
    egui::Area::new(egui::Id::new("pad_list")).anchor(Align2::LEFT_CENTER, [10.0 + k.side_w, 0.0]).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
        panel().show(ui, |ui| {
            ui.set_min_width(170.0);
            egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, _i: usize, dot: Color32, name: &str, chosen: bool| {
                                        let r = egui::Frame::NONE
                        .fill(if chosen { Color32::from_gray(70) } else { Color32::TRANSPARENT })
                        .stroke(egui::Stroke::new(2.0, if chosen { AMBER } else { Color32::TRANSPARENT }))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(16.0).color(dot));
                                ui.label(RichText::new(name).size(17.0).color(Color32::WHITE));
                            });
                        })
                        .response;
                    if chosen {
                        r.scroll_to_me(None);
                    }
                };
                ui.label(RichText::new(rocca::tr!("Squadre")).size(14.0).strong().color(Color32::from_gray(170)));
                for (t, team) in sim.teams.iter().enumerate() {
                    row(ui, t, team_colour(t), &team.name, k.selection == Selection::Team(t));
                }
                ui.add_space(4.0);
                ui.label(RichText::new(rocca::tr!("Borghi")).size(14.0).strong().color(Color32::from_gray(170)));
                for (d, district) in sim.districts.iter().enumerate() {
                    row(ui, teams + d, EVAC, &district.name, k.selection == Selection::District(d));
                }
            });
        });
    });
}

/// The aim of the gamepad: a small ring at the centre of the map.
fn aim(ctx: &egui::Context, c: egui::Pos2) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("pad_aim")));
    p.circle_stroke(c, 14.0, egui::Stroke::new(4.0, Color32::BLACK));
    p.circle_stroke(c, 14.0, egui::Stroke::new(2.0, Color32::WHITE));
    p.circle_filled(c, 2.5, Color32::WHITE);
}

/// A click on the map with a team selected: the team goes to the hex under it.
/// On a touch screen there is no pointer to hover with, so the first tap only
/// previews (circle, road, minutes) and a second tap on the same place gives
/// the order; a tap elsewhere moves the preview (2026-10-17).
fn map_click(k: &mut Kiosk, sim: &mut Sim, touch: bool) {
    let Some(p) = k.click else { return };
    if let Selection::Team(t) = k.selection {
        if sim.paused() {
            let at = rocca::Axial::of(snap_to_target(sim, t, p));
            // a click where the team already is takes the choice back
            if sim.teams[t].hex == Some(at) {
                k.clear_selection();
            } else if touch && k.move_hex != Some(at) {
                preview_move(k, sim, t, at);
            } else {
                give_move(k, sim, t, at);
            }
        }
    }
}

/// A team chosen and the pointer over the map: the hex under it is previewed
/// (outline, road and minutes) before the click that sends the team there.
fn hover_move(ctx: &egui::Context, k: &mut Kiosk, sim: &Sim, cam: (&Camera, &GlobalTransform)) {
    let Selection::Team(t) = k.selection else { return };
    if !sim.paused() || k.dragging.is_some() || ctx.dragged_id().is_some() || ctx.is_pointer_over_egui() {
        return;
    }
    let Some(p) = ctx.pointer_hover_pos().filter(|&p| drop_rect(ctx, k).contains(p)) else { return };
    if let Some(g) = ground_at(sim, cam, p) {
        preview_move(k, sim, t, rocca::Axial::of(snap_to_target(sim, t, g)));
    }
}

/// How far from a usable spot a click is still pulled to it, metres.
const SNAP_M: f32 = 250.0;

/// A point not on a usable hex is pulled to the road point of the nearest
/// hex the team can be sent to, if that is within one hex (user, 2026-10-16).
fn snap_to_target(sim: &Sim, t: usize, g: Pos) -> Pos {
    let targets = sim.move_targets(t);
    if targets.contains(&rocca::Axial::of(g)) {
        return g;
    }
    let near = targets.iter().filter_map(|&a| sim.hexes.get(a)).map(|h| (rocca::hex::dist(h.at.centre(), g), h.target))
        .filter(|(d, _)| *d <= SNAP_M).min_by(|a, b| a.0.total_cmp(&b.0));
    near.map_or(g, |(_, p)| p)
}

/// The ground under a screen point (egui logical px).
fn ground_at(sim: &Sim, cam: (&Camera, &GlobalTransform), p: egui::Pos2) -> Option<Pos> {
    let vp = crate::pick::window_to_viewport(cam.0, Vec2::new(p.x, p.y))?;
    let ray = cam.0.viewport_to_world(cam.1, vp).ok()?;
    crate::pick::ray_ground(&sim.scn, ray.origin, *ray.direction)
}

/// A compact ring gauge: dark disc, an arc clockwise from the top for
/// `frac` (0-1) in `col`.
fn ring_gauge(p: &egui::Painter, c: egui::Pos2, r: f32, frac: f32, col: Color32) {
    p.circle_filled(c, r + 3.0, Color32::from_black_alpha(200));
    p.circle_stroke(c, r, egui::Stroke::new(4.0, Color32::from_white_alpha(40)));
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let n = (48.0 * f).ceil().max(2.0) as usize;
        let pts: Vec<_> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * f * i as f32 / n as f32;
                c + r * egui::vec2(a.cos(), a.sin())
            })
            .collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(4.0, col)));
    }
}

/// District badges (clicked; the pointer over one shows its evacuation's
/// route and time), the teams' flags (dragged onto a hex, the order given on
/// release), the evacuation's «Evacua», and the evacuations' timers.
fn map_badges(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform)) {
    let hide_names = cfg!(target_arch = "wasm32") && compact(ctx);
    let mut hovered = None;
    let mut rects = vec![None; sim.districts.len()];
    let placed = declutter(ctx, k, sim, cam);
    let mut order: Vec<_> = (0..sim.districts.len()).collect();
    order.sort_by_key(|&d| !(k.selection == Selection::District(d) || k.expect() == Some(Expect::Evacuate(d))));
    for d in order {
        let Some(s) = placed.district[d] else { continue };
        let c = sim.districts[d].centre;
        let name = sim.districts[d].name.clone();
        let on = k.selection == Selection::District(d);
        // life: the share of the homes the fire has not reached
        let (hit, homes) = sim.district_damage(d);
        let life = if homes == 0 { 1.0 } else { 1.0 - hit as f32 / homes as f32 };
        let life_col = if life > 0.66 { GREEN } else if life > 0.33 { AMBER } else { RED };
        // evacuated: every family left and the evacuation ran its course
        let evacuated = sim.evac.of(d).is_some_and(|e| !e.interrupted) && sim.evac.remaining_min(d, sim.time_s()).is_none();
        let interrupted = sim.evac.of(d).is_some_and(|e| e.interrupted);
        let evacuating = sim.evac.of(d).is_some() && !evacuated && !interrupted;
        // far away and quiet: only a ring with a house, the name on hover
        let far = cam.1.translation().distance(crate::frame::to_bevy(c, sim.scn.terrain.height_at(c))) > MINI_BADGE_M;
        let mini = far && !on && !evacuated && !evacuating && !interrupted && hit == 0 && k.expect() != Some(Expect::Evacuate(d));
        let hide = hide_names || mini;
        // the colour of the district's state: blue quiet, amber or red hit, green evacuated
        let tint = if interrupted { RED } else if hit > 0 { if life > 0.66 { AMBER } else { life_col } } else if evacuated { GREEN } else { BLUE };
        let inner = egui::Area::new(egui::Id::new(("borgo", d))).fixed_pos(s).constrain(false).pivot(Align2::CENTER_CENTER).order(egui::Order::Middle).show(ctx, |ui| {
            let b = egui::Button::new(RichText::new(if hide { "" } else { &name }).size(16.0).strong().color(Color32::WHITE))
                .frame(false)
                .min_size(egui::vec2(if mini { 26.0 } else { 44.0 }, if compact(ctx) { 44.0 } else if mini { 24.0 } else { 28.0 }));
            // one container: name, life of the homes, evacuation timer
            egui::Frame::NONE
                .fill(if on { BLUE.gamma_multiply(0.6) } else if evacuated { Color32::from_rgb(18, 70, 40) } else if hit > 0 { Color32::from_rgba_unmultiplied(tint.r() / 3, tint.g() / 3, tint.b() / 3, 235) } else { Color32::from_black_alpha(190) })
                .stroke(egui::Stroke::new(if on { 3.0 } else if tint != BLUE { 2.5 } else { 1.0 }, if on { Color32::WHITE } else { tint }))
                .corner_radius(8.0)
                .inner_margin(if mini { egui::Margin::symmetric(3, 1) } else { egui::Margin::symmetric(6, 3) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let r = ui.add(b);
                        if hide {
                            super::icons::draw(ui.painter(), egui::Rect::from_center_size(r.rect.center(), egui::vec2(if mini { 16.0 } else { 22.0 }, if mini { 16.0 } else { 22.0 })), super::icons::Icon::House, Color32::WHITE);
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
                        }
                        if mini {
                            let r = r.on_hover_text(&name);
                            let (g, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            ring_gauge(ui.painter(), egui::pos2(g.center().x, r.rect.center().y), 7.0, life, life_col);
                            return r;
                        }
                        if evacuated && !hide {
                            ui.label(RichText::new(rocca::tr!("evacuato")).size(14.0).strong().color(GREEN));
                        }
                        let (g, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        ring_gauge(ui.painter(), egui::pos2(g.center().x, r.rect.center().y), 9.0, life, life_col);
                        evac_timer(ui, sim, d);
                        r
                    })
                    .inner
                })
                .inner
        });
        let r = inner.inner;
        rects[d] = Some(inner.response.rect);
        ctx.data_mut(|m| m.insert_temp(egui::Id::new(("bsz", "d", d)), inner.response.rect.size()));
        crate::playtest::control(&name, &r, true);
        if r.clicked() {
            select_district(k, sim, d);
        }
        if r.hovered() && ctx.dragged_id().is_none() && sim.evac.of(d).is_none() {
            hovered = Some(d);
        }
    }
    // the evacuation under the pointer: its route (on the map) and its time
    match hovered {
        Some(d) => {
            if !matches!(&k.evac_hover, Some((h, g, _)) if *h == d && *g + 10 > sim.generation) {
                k.evac_hover = Some((d, sim.generation, sim.plan_evacuation(d)));
            }
        }
        // a still picture (`snap`) keeps the one it was given
        None if k.frozen => {}
        None => k.evac_hover = None,
    }
    if let Some(d) = k.evac_hover.as_ref().map(|h| h.0) {
        if let (Some(rect), true) = (rects[d], k.selection != Selection::District(d)) {
            use super::icons::Icon;
            egui::Area::new(egui::Id::new(("evacua_su", d))).fixed_pos(rect.left_bottom() + egui::vec2(0.0, 6.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
                egui::Frame::NONE.fill(Color32::from_black_alpha(215)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| match &k.evac_hover {
                        Some((_, _, Ok(p))) => {
                            chip(ui, Icon::Family, &p.people.to_string(), Color32::WHITE);
                            ui.add_space(6.0);
                            chip(ui, Icon::Clock, &rocca::tr!("{0} min", format!("{:.0}", p.total_min().ceil())), EVAC);
                        }
                        Some((_, _, Err(_))) => chip(ui, Icon::Alert, rocca::tr!("strade chiuse"), RED),
                        None => {}
                    });
                });
            });
        }
    }
    // a district of scattered hamlets (Le Ghiande): chosen or under the
    // pointer, every hamlet says it belongs to it, so one click is all homes
    let shown = match k.selection {
        Selection::District(d) => Some(d),
        _ => k.evac_hover.as_ref().map(|h| h.0),
    };
    if let (Some(d), Some(badge)) = (shown, shown.and_then(|d| rects[d])) {
        hamlet_tags(ctx, sim, cam, d, badge);
    }
    team_flags(ctx, k, sim, cam, &placed);
    if k.phase == Phase::Tutorial {
        tutorial_pulse(ctx, k, sim, cam, &placed);
    }
    confirm_bubble(ctx, k, sim, cam);
}

/// For a district with several separate hamlets: a tag on each, with the
/// district's name and its homes there, and a line from the district's badge.
/// The click (and the evacuation) covers all of them.
fn hamlet_tags(ctx: &egui::Context, sim: &Sim, cam: (&Camera, &GlobalTransform), d: usize, badge: egui::Rect) {
    let district = &sim.districts[d];
    if district.nuclei.len() < 2 {
        return;
    }
    let mut homes = vec![0usize; district.nuclei.len()];
    for &i in &district.households {
        let p = Pos::from(sim.scn.population.households[i].pos);
        let near = (0..homes.len()).min_by(|&a, &b| rocca::district::dist(p, district.nuclei[a]).total_cmp(&rocca::district::dist(p, district.nuclei[b])));
        if let Some(n) = near {
            homes[n] += 1;
        }
    }
    let line = egui::Stroke::new(2.0, BLUE);
    for (n, &at) in district.nuclei.iter().enumerate() {
        let Some(s) = screen(cam, sim, at, 40.0) else { continue };
        ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new(("nucleo_linea", d)))).line_segment([badge.center(), s], line);
        egui::Area::new(egui::Id::new(("nucleo", d, n))).fixed_pos(s).constrain(false).pivot(Align2::CENTER_CENTER).order(egui::Order::Middle).interactable(false).show(ctx, |ui| {
            egui::Frame::NONE.fill(Color32::from_black_alpha(215)).stroke(line).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.label(RichText::new(rocca::tr!("{0} · {1} case", district.name, homes[n])).size(14.0).strong().color(Color32::WHITE));
            });
        });
    }
}

/// The handles of the teams' 3D flags (`pins`): clicking one stops the world;
/// in the pause a flag is dragged onto a hex and the team goes there when it
/// is let go; a click on a flag and then on a hex does the same, with the hex
/// under the pointer previewed (`hover_move`). A team in trouble says so.
fn team_flags(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform), placed: &Placed) {
    use super::pins;
    let right: Vec3 = cam.1.right().into();
    let feet = pins::feet(k, sim, cam.1.translation(), right);
    for t in 0..sim.teams.len() {
        let Some((_, sc)) = feet[t] else { continue };
        // the handle: a badge over the team's spot, moved aside if it would cover another (`declutter`)
        let Some(centre) = placed.team[t] else { continue };
        let col = team_colour(t);
        // «Squadra A» → «A»
        let letter = sim.teams[t].name.split_whitespace().last().unwrap_or("").to_string();
        let on = k.selection == Selection::Team(t);
        let unit = &sim.crews.units[sim.teams[t].unit];
        let lost = unit.state == abm::suppression::UnitState::Lost;
        let withdrawing = unit.state == abm::suppression::UnitState::Withdrawing;
        let multi = ctx.input(|i| i.multi_touch().is_some_and(|m| m.num_touches > 1));
        let canceled = ctx.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. })));
        // in danger the badge blinks red; the team then withdraws on its own (`Game::team_in_danger`)
        let alarm = !lost && sim.team_in_danger(t);
        let blink = alarm && ctx.input(|i| (i.time * 3.0) as i64 % 2 == 0);
        if alarm {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        // far away and quiet: just the letter, to leave the map readable
        let small = sc > SMALL_FLAG_SCALE && !on && !alarm && !k.dragging.is_some_and(|(d, _)| d == t);
        let size = if small { egui::vec2(50.0, 44.0) } else { egui::vec2(96.0, 56.0) };
        let flag = egui::Rect::from_center_size(centre, size);
        ctx.data_mut(|m| m.insert_temp(egui::Id::new(("bsz", "t", t)), size));
        let can_order = !lost && !multi && !canceled;
        let mut may_drag = sim.paused() && can_order;
        if (multi || canceled) && k.dragging.is_some() { k.clear_selection(); }
        let inner = egui::Area::new(egui::Id::new(("bandiera", t))).fixed_pos(flag.min).constrain(false).order(egui::Order::Middle).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(flag.size(), egui::Sense::click_and_drag());
            let p = ui.painter();
            let badge = rect.shrink(4.0);
            let hot = on || resp.hovered();
            let (fill, edge) = if blink { (Color32::from_rgb(200, 30, 25), Color32::WHITE) } else { (Color32::from_black_alpha(215), if alarm { RED } else if hot { Color32::WHITE } else { col }) };
            p.rect_filled(badge, 16.0, fill);
            p.rect_stroke(badge, 16.0, egui::Stroke::new(if hot || alarm { 4.0 } else { 3.0 }, edge), egui::StrokeKind::Middle);
            let ink = if blink { Color32::WHITE } else { col };
            if small {
                let g = p.layout_no_wrap(letter.clone(), egui::FontId::proportional(26.0), ink);
                p.galley(badge.center() - g.size() / 2.0, g, ink);
            } else {
                super::icons::draw(p, egui::Rect::from_center_size(badge.left_center() + egui::vec2(24.0, 0.0), egui::vec2(34.0, 34.0)), super::icons::Icon::Engine, ink);
                let g = p.layout_no_wrap(letter.clone(), egui::FontId::proportional(28.0), ink);
                p.galley(egui::pos2(badge.right() - 14.0 - g.size().x, badge.center().y - g.size().y / 2.0), g, ink);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(if may_drag { egui::CursorIcon::Grab } else { egui::CursorIcon::PointingHand });
            }
            resp
        });
        let resp = inner.inner;
        crate::playtest::control(&sim.teams[t].name.clone(), &resp, true);
        // what is wrong, in two words under the badge
        let trouble = if lost { Some((rocca::tr!("fuori servizio"), GREY)) } else if withdrawing { Some((rocca::tr!("si ritira"), RED)) } else if alarm { Some((rocca::tr!("fuoco vicino!"), RED)) } else { None };
        if let Some((text, c)) = trouble {
            egui::Area::new(egui::Id::new(("stato_squadra", t))).fixed_pos(flag.center_bottom() + egui::vec2(0.0, 2.0)).pivot(Align2::CENTER_TOP).order(egui::Order::Middle).interactable(false).show(ctx, |ui| {
                egui::Frame::NONE.fill(Color32::from_black_alpha(215)).stroke(egui::Stroke::new(1.5, c)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(7, 2)).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(15.0).strong().color(c)).wrap_mode(egui::TextWrapMode::Extend));
                });
            });
        }
        if resp.clicked() && on {
            // a second click on the chosen flag takes the choice back
            k.clear_selection();
        } else if (resp.drag_started() || resp.clicked()) && can_order {
            select_team(k, sim, t);
            may_drag = sim.paused() && k.selection == Selection::Team(t);
        }
        if resp.dragged() && may_drag && k.selection == Selection::Team(t) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(p) = ctx.pointer_latest_pos() {
                k.drag_pan = edge_pan(drop_rect(ctx, k), p);
            }
            if let Some(g) = ctx.pointer_latest_pos().and_then(|p| ground_at(sim, cam, p)) {
                let g = snap_to_target(sim, t, g);
                preview_move(k, sim, t, rocca::Axial::of(g));
                k.dragging = Some((t, g));
            }
        }
        // where it would go: the time on the road, next to the flag or the pointer
        let dragged = k.dragging.is_some_and(|(d, _)| d == t);
        let hovering = on && !dragged && sim.paused() && k.move_hex.is_some() && !ctx.is_pointer_over_egui();
        if dragged || hovering {
            use super::icons::Icon;
            let at = match ctx.pointer_hover_pos() {
                _ if compact(ctx) => egui::pos2(12.0, 122.0),
                Some(p) if hovering => p + egui::vec2(18.0, 18.0),
                _ => flag.right_top() + egui::vec2(8.0, 4.0),
            };
            egui::Area::new(egui::Id::new("trascina")).fixed_pos(at).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
                egui::Frame::NONE.fill(Color32::from_black_alpha(215)).stroke(egui::Stroke::new(2.0, col)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    // one short line: an icon and a few words, never wrapped
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon, text, c) = match &k.move_plan {
                            Some(Ok(plan)) => (Icon::Clock, rocca::tr!("{0} min", format!("{:.0}", plan.minutes.ceil())), AMBER),
                            Some(Err(e)) => (Icon::Cross, e.clone(), RED),
                            None => return,
                        };
                        super::icons::show(ui, icon, 22.0, c);
                        ui.add(egui::Label::new(RichText::new(text).size(18.0).strong().color(c)).wrap_mode(egui::TextWrapMode::Extend));
                    });
                });
            });
        }
        if resp.drag_stopped() && k.selection == Selection::Team(t) {
            // Touch release also emits PointerGone; latest_pos is then None.
            // The release event still carries the actual drop location.
            let release = ctx.input(|i| i.raw.events.iter().rev().find_map(|e| match e {
                egui::Event::Touch { phase: egui::TouchPhase::End, pos, .. } => Some(*pos),
                egui::Event::PointerButton { pressed: false, pos, .. } => Some(*pos),
                _ => None,
            })).or_else(|| ctx.pointer_latest_pos());
            let on_map = release.is_some_and(|p| drop_rect(ctx, k).contains(p));
            match k.dragging {
                Some((_, g)) if on_map && may_drag => give_move(k, sim, t, rocca::Axial::of(g)),
                _ => { k.clear_selection(); k.notify(rocca::tr!("Spostamento annullato: rilascia la squadra sulla mappa.")); }
            }
        }
    }
}

/// Where the badges sit on the screen this frame: the centre of each
/// district's and the badge of each team's.
struct Placed {
    district: Vec<Option<egui::Pos2>>,
    team: Vec<Option<egui::Pos2>>,
}

/// Puts every badge where it covers no other badge and no other entity's
/// spot. Each one wants the place over its entity; in order of importance
/// (the team being moved, a team in danger, the other teams, the chosen
/// district, the rest) it takes the nearest free place: the last frame's, to
/// keep still, then rings of growing radius. A badge that had to leave is tied
/// to its entity with a line.
fn declutter(ctx: &egui::Context, k: &Kiosk, sim: &Sim, cam: (&Camera, &GlobalTransform)) -> Placed {
    use super::pins;
    struct Item {
        team: bool,
        idx: usize,
        anchor: egui::Pos2,
        size: egui::Vec2,
        /// the preferred centre, from the anchor
        pref: egui::Vec2,
        /// room reserved under the badge (a team's two words of trouble)
        below: f32,
        col: Color32,
        prio: usize,
        must: bool,
    }
    let compact = compact(ctx);
    let bounds = if compact { drop_rect(ctx, k).shrink(4.0) } else { ctx.viewport_rect().shrink(4.0) };
    let size_of = |kind: &str, i: usize, dflt: egui::Vec2| ctx.data(|m| m.get_temp::<egui::Vec2>(egui::Id::new(("bsz", kind, i)))).unwrap_or(dflt);
    let mut items: Vec<Item> = vec![];
    let right: Vec3 = cam.1.right().into();
    let feet = pins::feet(k, sim, cam.1.translation(), right);
    for t in 0..sim.teams.len() {
        let Some((foot, sc)) = feet[t] else { continue };
        let lift = crate::rings::RING_LIFT_M + pins::LIFT_M * sc;
        let Ok(a) = cam.0.world_to_viewport(cam.1, crate::frame::to_bevy(foot, sim.scn.terrain.height_at(foot) + lift)) else { continue };
        let unit = &sim.crews.units[sim.teams[t].unit];
        let lost = unit.state == abm::suppression::UnitState::Lost;
        let alarm = !lost && sim.team_in_danger(t);
        let trouble = lost || alarm || unit.state == abm::suppression::UnitState::Withdrawing;
        let dragged = k.dragging.is_some_and(|(d, _)| d == t);
        let prio = if dragged { 0 } else if k.selection == Selection::Team(t) { 1 } else if alarm { 2 } else { 3 };
        items.push(Item {
            team: true,
            idx: t,
            anchor: egui::pos2(a.x, a.y),
            size: size_of("t", t, egui::vec2(96.0, 56.0)),
            pref: egui::vec2(0.0, -8.0),
            below: if trouble { 28.0 } else { 0.0 },
            col: team_colour(t),
            prio,
            // an order can only be given through its badge: never dropped
            must: true,
        });
    }
    // a chosen team's destination takes priority over the labels of the towns
    if !(compact && matches!(k.selection, Selection::Team(_))) {
        for d in 0..sim.districts.len() {
            let c = sim.districts[d].centre;
            let Some(s) = screen(cam, sim, c, 40.0) else { continue };
            let picked = k.selection == Selection::District(d) || k.expect() == Some(Expect::Evacuate(d));
            items.push(Item {
                team: false,
                idx: d,
                anchor: s,
                size: size_of("d", d, egui::vec2(150.0, 40.0)),
                pref: egui::vec2(0.0, if compact { 26.0 } else { 0.0 }),
                below: 0.0,
                col: BLUE,
                prio: if picked { 4 } else { 5 },
                must: false,
            });
        }
    }
    items.retain(|i| {
        let dragged = i.team && k.dragging.is_some_and(|(d, _)| d == i.idx);
        dragged || bounds.expand(if compact { 0.0 } else { 40.0 }).contains(i.anchor)
    });
    items.sort_by_key(|i| (i.prio, i.team as usize ^ 1, i.idx));

    // candidate offsets, nearest first: wide rings, since badges are wide
    let mut ring: Vec<egui::Vec2> = vec![];
    for i in 1..=14 {
        let r = 26.0 * i as f32;
        let n = (6 + 3 * i).min(36);
        for j in 0..n {
            let m = ((j + 1) / 2) as f32;
            let a = -std::f32::consts::FRAC_PI_2 + if j % 2 == 1 { 1.0 } else { -1.0 } * std::f32::consts::TAU * m / n as f32;
            ring.push(egui::vec2(a.cos() * r * 1.5, a.sin() * r));
        }
    }
    let spots: Vec<egui::Pos2> = items.iter().map(|i| i.anchor).collect();
    let mut taken: Vec<egui::Rect> = vec![];
    let mut out = Placed { district: vec![None; sim.districts.len()], team: vec![None; sim.teams.len()] };
    let leaders = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("badge_leaders")));
    for (n, it) in items.iter().enumerate() {
        let key = egui::Id::new(("boff", it.team, it.idx));
        let last = ctx.data(|m| m.get_temp::<egui::Vec2>(key));
        let reserved = |off: egui::Vec2| {
            let centre = it.anchor + off;
            egui::Rect::from_min_size(centre - it.size * 0.5, it.size + egui::vec2(0.0, it.below))
        };
        let free = |r: egui::Rect| {
            bounds.contains_rect(r)
                && taken.iter().all(|o| !o.intersects(r.expand(4.0)))
                && spots.iter().enumerate().all(|(i, &p)| i == n || !r.expand(3.0).contains(p))
        };
        let mut off = std::iter::once(it.pref).chain(last).chain(ring.iter().map(|o| it.pref + *o)).find(|&o| free(reserved(o)));
        if off.is_none() && it.must {
            // no free place at all: keep it reachable, over the others
            off = Some(it.pref);
        }
        let Some(mut off) = off else { continue };
        let mut r = reserved(off);
        if it.must && !bounds.contains_rect(r) {
            let dx = (bounds.left() - r.left()).max(0.0) - (r.right() - bounds.right()).max(0.0);
            let dy = (bounds.top() - r.top()).max(0.0) - (r.bottom() - bounds.bottom()).max(0.0);
            off += egui::vec2(dx, dy);
            r = reserved(off);
        }
        taken.push(r);
        ctx.data_mut(|m| m.insert_temp(key, off));
        let centre = it.anchor + off;
        // away from its entity: a line to it
        if !r.expand(6.0).contains(it.anchor) {
            let end = egui::pos2(it.anchor.x.clamp(r.left(), r.right()), it.anchor.y.clamp(r.top(), r.bottom()));
            leaders.line_segment([it.anchor, end], egui::Stroke::new(4.5, Color32::from_black_alpha(160)));
            leaders.line_segment([it.anchor, end], egui::Stroke::new(2.0, it.col));
            leaders.circle_filled(it.anchor, 5.0, Color32::from_black_alpha(200));
            leaders.circle_filled(it.anchor, 3.5, it.col);
        }
        if it.team { out.team[it.idx] = Some(centre) } else { out.district[it.idx] = Some(centre) }
    }
    out
}

/// Seen from this far (`pins::scale`) a quiet team is only its letter.
const SMALL_FLAG_SCALE: f32 = 1.6;
/// Seen from this far (metres) a district that is quiet shrinks to a ring.
const MINI_BADGE_M: f32 = 3500.0;

/// What is going wrong outside the picture: a district that begins to lose its
/// homes, a team in danger. An arrow badge on the edge points that way; a
/// click flies the camera there.
fn offscreen_alerts(ctx: &egui::Context, k: &mut Kiosk, sim: &Sim, cam: (&Camera, &GlobalTransform)) {
    use super::icons::{self, Icon};
    let seen = drop_rect(ctx, k).shrink(6.0);
    let inset = seen.shrink2(egui::vec2(54.0, 64.0));
    // (what, where, colour, label, severity)
    let mut items: Vec<(Icon, Pos, Color32, String, f32)> = vec![];
    for d in 0..sim.districts.len() {
        let (hit, homes) = sim.district_damage(d);
        let interrupted = sim.evac.of(d).is_some_and(|e| e.interrupted);
        if hit == 0 && !interrupted {
            continue;
        }
        let life = if homes == 0 { 1.0 } else { 1.0 - hit as f32 / homes as f32 };
        let col = if life > 0.66 { AMBER } else { RED };
        items.push((Icon::House, sim.districts[d].centre, col, sim.districts[d].name.clone(), 1.0 - life));
    }
    for t in 0..sim.teams.len() {
        if sim.team_in_danger(t) {
            let u = &sim.crews.units[sim.teams[t].unit];
            items.push((Icon::Engine, u.pos, RED, sim.teams[t].name.split_whitespace().last().unwrap_or("").to_string(), 2.0));
        }
    }
    let centre = seen.center();
    let mut placed: Vec<egui::Pos2> = vec![];
    let mut fly = None;
    items.sort_by(|a, b| b.4.total_cmp(&a.4));
    for (i, (icon, at, col, label, _)) in items.into_iter().enumerate() {
        let v = crate::frame::to_bevy(at, sim.scn.terrain.height_at(at) + 20.0);
        let shown = cam.0.world_to_viewport(cam.1, v).ok();
        if shown.is_some_and(|s| seen.contains(egui::pos2(s.x, s.y))) {
            continue;
        }
        // the direction on the screen; behind the camera it is turned round
        let dir = match shown {
            Some(s) => egui::vec2(s.x, s.y) - centre.to_vec2(),
            None => {
                let l = cam.1.affine().inverse().transform_point3(v);
                let d = egui::vec2(l.x, -l.y);
                if l.z > 0.0 { -d } else { d }
            }
        };
        let dir = if dir.length() < 1.0 { egui::vec2(0.0, -1.0) } else { dir.normalized() };
        let reach = (inset.width() * 0.5 / dir.x.abs().max(1e-3)).min(inset.height() * 0.5 / dir.y.abs().max(1e-3));
        let mut pos = centre + dir * reach;
        // two badges close together: slide the later one along the edge
        let mut guard = 0;
        while placed.iter().any(|q| (*q - pos).length() < 58.0) && guard < 8 {
            let side = egui::vec2(-dir.y, dir.x);
            pos = inset.clamp(pos + side * 60.0);
            guard += 1;
        }
        placed.push(pos);
        let pulse = ctx.input(|i| (i.time * 2.5).sin() as f32 * 0.5 + 0.5);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let resp = egui::Area::new(egui::Id::new(("fuori", i))).fixed_pos(pos).pivot(Align2::CENTER_CENTER).constrain(false).order(egui::Order::Foreground).show(ctx, |ui| {
            let size = 52.0;
            let (r, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
            let c = r.center();
            let p = ui.painter();
            // the arrow, pointing out of the picture
            let n = egui::vec2(-dir.y, dir.x);
            let tip = c + dir * (size * 0.5 + 17.0);
            let base = c + dir * (size * 0.5 - 2.0);
            p.add(egui::Shape::convex_polygon(vec![tip, base + n * 13.0, base - n * 13.0], col, egui::Stroke::NONE));
            p.circle_filled(c, size * 0.5, Color32::from_black_alpha(225));
            p.circle_stroke(c, size * 0.5, egui::Stroke::new(3.0 + 2.0 * pulse, col));
            icons::draw(p, egui::Rect::from_center_size(c - egui::vec2(0.0, 4.0), egui::vec2(26.0, 26.0)), icon, col);
            let g = p.layout_no_wrap(label.clone(), egui::FontId::proportional(12.0), Color32::WHITE);
            p.galley(egui::pos2(c.x - g.size().x / 2.0, r.bottom() + 1.0), g, Color32::WHITE);
            resp
        }).inner;
        crate::playtest::control(&format!("Vai a {label}"), &resp, true);
        if resp.clicked() {
            fly = Some(at);
        }
    }
    if let Some(at) = fly {
        k.follow = false;
        k.shot = Some(super::Shot { at: Some(at), zoom: 0.5 });
    }
}

/// The evacuation being chosen, on its district: who, where to and how
/// long (a dial: preparation, departures, the road), «Evacua» / «Annulla».
/// In the same pause the choice can be changed: another district replaces
/// it, and the district itself offers to take it back.
fn confirm_bubble(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform)) {
    use super::icons::Icon;
    if ctx.dragged_id().is_some() {
        return;
    }
    let Selection::District(d) = k.selection else { return };
    let Some(plan) = k.evac_plan.clone() else { return };
    let s = if compact(ctx) {
        egui::pos2(if k.side_w > 0.0 { ctx.viewport_rect().right() - k.side_w * 0.5 } else { ctx.viewport_rect().center().x }, if k.side_w > 0.0 { 8.0 } else { 100.0 })
    } else {
        let Some(s) = screen(cam, sim, sim.districts[d].centre, 40.0) else { return };
        s
    };
    let this_pause = sim.can_cancel_evacuation();
    let ordered_here = this_pause && sim.evac.active().is_some_and(|e| e.district == d);
    let mut close = false;
    egui::Area::new(egui::Id::new("conferma")).fixed_pos(s + egui::vec2(0.0, 24.0)).pivot(Align2::CENTER_TOP).order(egui::Order::Foreground).show(ctx, |ui| {
        card(BLUE).inner_margin(if compact(ctx) { 10.0 } else { 18.0 }).show(ui, |ui| {
            if compact(ctx) { ui.set_width((if k.side_w > 0.0 { k.side_w - 40.0 } else { ctx.viewport_rect().width() - 48.0 }).min(340.0)); }
            ui.label(RichText::new(&sim.districts[d].name).size(19.0).strong().color(BLUE));
            if ordered_here {
                ui.horizontal(|ui| chip(ui, Icon::Exit, rocca::tr!("evacuazione ordinata"), GREEN));
                ui.horizontal(|ui| {
                    let r = ui.add(secondary(rocca::tr!("Annulla l'evacuazione")));
                    crate::playtest::control("Annulla l'evacuazione", &r, true);
                    if r.clicked() {
                        if let Err(e) = sim.cancel_evacuation() {
                            k.notify(e);
                        }
                        close = true;
                    }
                    let r = ui.add(secondary(rocca::tr!("Chiudi")));
                    close |= r.clicked();
                });
                return;
            }
            match &plan {
                Err(e) => {
                    ui.horizontal(|ui| chip(ui, Icon::Alert, e, RED));
                }
                Ok(p) => {
                    ui.horizontal(|ui| {
                        chip(ui, Icon::Family, &p.people.to_string(), Color32::WHITE);
                        ui.add_space(10.0);
                        chip(ui, Icon::Clock, &rocca::tr!("{0} min", format!("{:.0}", p.total_min().ceil())), EVAC);
                    });
                }
            }
            let other = sim.evac.active().map(|e| e.district).filter(|&e| e != d);
            if let (Some(e), true) = (other, this_pause) {
                ui.horizontal(|ui| chip(ui, Icon::Up, &rocca::tr!("al posto di {0}", sim.districts[e].name), AMBER));
            }
            let free = other.is_none() || this_pause;
            let ok = plan.is_ok() && sim.paused() && free && k.allows(Expect::Evacuate(d));
            ui.horizontal(|ui| {
                let r = ui.add_enabled(ok, primary(rocca::tr!("Evacua")));
                crate::playtest::control("Evacua", &r, ok);
                if r.clicked() {
                    match sim.evacuate(d) {
                        Ok(_) => {
                            k.done(Expect::Evacuate(d));
                            resume_after_order(k, sim);
                            close = true;
                        }
                        Err(e) => k.notify(e),
                    }
                }
                let r = ui.add(secondary(rocca::tr!("Annulla")));
                crate::playtest::control("Annulla", &r, true);
                close |= r.clicked();
            });
            if let (Some(e), false) = (other, this_pause) {
                ui.horizontal(|ui| chip(ui, Icon::Alert, &rocca::tr!("in corso: {0}", sim.districts[e].name), AMBER));
            }
        });
    });
    if close {
        k.clear_selection();
    }
}

/// The timer on a district being evacuated: a ring that empties with the time
/// left until the last family reaches the refuge (`Evacuations::remaining_min`,
/// the rule of the time announced); a check when it is done, an alert when
/// the fire cut it short.
fn evac_timer(ui: &mut egui::Ui, sim: &Sim, d: usize) {
    use super::icons::{self, Icon};
    let Some(e) = sim.evac.of(d) else { return };
    let left = sim.evac.remaining_min(d, sim.time_s());
    let (frac, minutes, col, icon) = match left {
        _ if e.interrupted => (1.0, None, RED, Some(Icon::Alert)),
        Some(left) => {
            let elapsed = (sim.time_s() - e.started_s).max(0) as f32 / 60.0;
            (left / (elapsed + left).max(0.1), Some(left.ceil() as i64), EVAC, None)
        }
        None => (1.0, None, GREEN, Some(Icon::Check)),
    };
    let size = if minutes.is_some() { 44.0 } else { 30.0 };
    {
        let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let p = ui.painter();
        let c = r.center();
        let radius = size * 0.5 - 4.0;
                p.circle_stroke(c, radius, egui::Stroke::new(4.0, Color32::from_gray(60)));
        // the time left: an arc from the top, clockwise, that shrinks
        let n = ((frac * 64.0).ceil() as usize).max(1);
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac.clamp(0.0, 1.0) * i as f32 / n as f32;
                c + egui::vec2(a.cos(), a.sin()) * radius
            })
            .collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(4.0, col)));
        if let Some(m) = minutes {
            let g = p.layout_no_wrap(format!("{m}"), egui::FontId::proportional(16.0), Color32::WHITE);
            p.galley(c - egui::vec2(g.size().x / 2.0, g.size().y / 2.0 + 3.0), g, Color32::WHITE);
            let g = p.layout_no_wrap(rocca::tr!("min").into(), egui::FontId::proportional(9.0), GREY);
            p.galley(c + egui::vec2(-g.size().x / 2.0, 5.0), g, GREY);
        }
        if let Some(icon) = icon {
            icons::draw(p, egui::Rect::from_center_size(c, egui::vec2(size * 0.5, size * 0.5)), icon, col);
        }
    }
}

/// Frames per second, top left, in the demo (the intro loop) only. Desktop only:
/// the browser has no such counter, and the kiosk's own FPS log stays under `KIOSK_FPS`.
#[cfg(not(target_arch = "wasm32"))]
pub fn fps(
    mut contexts: EguiContexts,
    kiosk: Res<Kiosk>,
    mario: Res<crate::mario::Playground>,
    time: Res<Time<Real>>,
    // (mean frame time in s, shown FPS, s since the shown value changed)
    mut filter: Local<(f32, f32, f32)>,
) {
    // not in a recording (`record`): its frame rate is the video's
    if kiosk.phase != Phase::Intro || mario.active || crate::record::running() {
        return;
    }
    // Low-pass the frame time (about 1.5 s), not the FPS: a hitch then weighs
    // what it costs. The label is rewritten twice a second so it can be read.
    let dt = time.delta_secs().min(0.5);
    if dt <= 0.0 {
        return;
    }
    let (mean, shown, since) = &mut *filter;
    *mean = if *mean == 0.0 { dt } else { *mean + (dt - *mean) * (1.0 - (-dt / 1.5).exp()) };
    *since += dt;
    if *shown == 0.0 || *since >= 0.5 {
        *shown = 1.0 / *mean;
        *since = 0.0;
    }
    let fps = *shown;
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::Area::new(egui::Id::new("fps")).anchor(Align2::LEFT_TOP, [10.0, 10.0]).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(Color32::from_black_alpha(230))
            .stroke(egui::Stroke::new(1.5, Color32::from_gray(140)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("{fps:.0} FPS")).size(18.0).strong().monospace().color(Color32::WHITE));
            });
    });
}

fn notice(ctx: &egui::Context, k: &Kiosk) {
    if k.dragging.is_some() { return; }
    let Some((text, _)) = &k.notice else { return };
    egui::Area::new(egui::Id::new("avviso")).anchor(Align2::CENTER_TOP, [0.0, if compact(ctx) { 116.0 } else if k.phase == Phase::Tutorial { 170.0 } else { 10.0 }]).order(egui::Order::Foreground).show(ctx, |ui| {
        panel().show(ui, |ui| {
            if compact(ctx) { ui.set_width((ctx.viewport_rect().width() - 48.0).min(480.0)); }
            ui.add(egui::Label::new(RichText::new(text).size(16.0).color(Color32::WHITE)).wrap());
        });
    });
}

/// The small button that mutes or unmutes, beside «Impostazioni».
fn mute_button(ui: &mut egui::Ui, prefs: &mut crate::sound::AudioPrefs) -> egui::Response {
    let r = ui.add(secondary(if prefs.muted { "🔇" } else { "🔊" }))
        .on_hover_text(if prefs.muted { rocca::tr!("Attiva l'audio") } else { rocca::tr!("Togli l'audio") });
    if r.clicked() {
        prefs.muted = !prefs.muted;
        prefs.save();
    }
    r
}

/// The settings menu (F2): language, view, audio, graphics.
fn operator(ctx: &egui::Context, k: &mut Kiosk, gfx: &mut crate::quality::Graphics, audio: &mut crate::sound::AudioPrefs, auto: Option<&crate::adaptive::Adaptive>, commands: &mut Commands) {
    if compact(ctx) {
        // On a phone: a page of its own over the screen, big buttons, and
        // «Chiudi» pinned under the scrolling controls.
        let mut close = false;
        super::screens::compact_page_footer(ctx, "operatore_mobile", |ui| {
            ui.heading(rocca::tr!("Impostazioni"));
            operator_body(ui, k, gfx, audio, auto, commands, true);
        }, |ui| {
            close = ui.add_sized([ui.available_width(), 46.0], secondary(rocca::tr!("Chiudi"))).clicked();
        });
        if close { k.operator = false; }
        return;
    }
    egui::Window::new(rocca::tr!("Impostazioni")).id(egui::Id::new("operatore")).anchor(Align2::LEFT_BOTTOM, [8.0, -60.0]).default_width((ctx.viewport_rect().width() - 48.0).min(380.0)).max_height(ctx.viewport_rect().height() - 90.0).resizable(false).collapsible(false).vscroll(true).show(ctx, |ui| {
        panel().show(ui, |ui| {
            ui.label(RichText::new(rocca::tr!("Impostazioni (F2 per chiudere)")).strong().color(Color32::WHITE));
            operator_body(ui, k, gfx, audio, auto, commands, false);
        });
    });
}

/// The controls of the operator menu. `touch`: finger-sized buttons.
fn operator_body(ui: &mut egui::Ui, k: &mut Kiosk, gfx: &mut crate::quality::Graphics, audio: &mut crate::sound::AudioPrefs, auto: Option<&crate::adaptive::Adaptive>, commands: &mut Commands, touch: bool) {
    let button = |ui: &mut egui::Ui, label: &str| -> bool {
        if touch { ui.add_sized([ui.available_width(), 46.0], secondary(label)).clicked() } else { ui.add(secondary(label)).clicked() }
    };
    {
        {
            ui.horizontal(|ui| {
                ui.label(rocca::tr!("Lingua:"));
                super::lang::picker_ui(ui);
            });
            if button(ui, rocca::tr!("Vista iniziale")) {
                k.reset_view = true;
            }
            crate::sound::settings_ui(ui, audio);
            if !cfg!(target_arch = "wasm32") { ui.horizontal(|ui| {
                use crate::quality::Graphics;
                ui.label(rocca::tr!("Grafica:"));
                let now_level = if auto.is_some() { None } else { gfx.level() };
                let label = match auto {
                    Some(a) => format!("Auto ({}/{})", a.step() + 1, a.rungs()),
                    None => "Auto".to_string(),
                };
                if ui.selectable_label(auto.is_some(), label)
                    .on_hover_text(rocca::tr!("Il livello segue la velocità di questo computer: scende se i frame sono lenti, risale se c'è margine"))
                    .clicked() && auto.is_none()
                {
                    let a = crate::adaptive::Adaptive::new(Graphics::ALTA.built_as(&gfx));
                    *gfx = a.initial();
                    commands.insert_resource(a);
                    #[cfg(not(target_arch = "wasm32"))]
                    crate::quality::forget_aa();
                    if let Err(e) = crate::quality::save("auto") {
                        k.error = Some(e);
                    }
                }
                for (name, label, preset) in [("alta", "Alta", Graphics::ALTA), ("media", "Media", Graphics::MEDIA), ("bassa", "Bassa", Graphics::BASSA)] {
                    if ui.selectable_label(now_level == Some(name), label).clicked() && now_level != Some(name) {
                        // a level brings its own antialiasing: the choice made apart is dropped
                        *gfx = preset.built_as(&gfx);
                        #[cfg(not(target_arch = "wasm32"))]
                        crate::quality::forget_aa();
                        if let Err(e) = crate::quality::save(name) {
                            k.error = Some(e);
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(rocca::tr!("Antialiasing:"));
                let now = gfx.aa();
                for (name, label, tip) in [
                    ("msaa", "MSAA 4×", "Il più pulito sui bordi sottili; costa qualche ms a schermo grande"),
                    ("smaa", "SMAA", "Filtro sui bordi dopo il disegno: quasi gratis, bordi quasi uguali"),
                    ("nessuno", "Nessuno", "Bordi seghettati"),
                ] {
                    if ui.selectable_label(now == name, label).on_hover_text(tip).clicked() && now != name {
                        gfx.set_aa(name);
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Err(e) = crate::quality::save_aa(name) {
                            k.error = Some(e);
                        }
                    }
                }
            });
            }
            if let Some(e) = &k.error {
                ui.colored_label(RED, e);
            }
        }
    }
}


fn compact_top(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, cam: (&Camera, &GlobalTransform)) {
    egui::Area::new(egui::Id::new("barra_mobile")).anchor(Align2::LEFT_TOP, [8.0, 8.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        panel().inner_margin(8.0).show(ui, |ui| {
            ui.set_width((ctx.viewport_rect().width() - k.side_w - 32.0).min(460.0));
            ui.horizontal(|ui| {
                pause_dial(ui, k, sim);
                ui.vertical(|ui| {
                    ui.label(RichText::new(clock(sim.time_s())).size(20.0).strong());
                    let w = sim.weather();
                    ui.horizontal(|ui| {
                        wind_arrow(ui, cam, w.wind_dir_deg, 22.0, Color32::WHITE);
                        ui.label(rocca::tr!("{0} km/h · umidità {1}%", format!("{:.0}", w.wind_speed_kmh), format!("{:.0}", w.moisture_pct)));
                    });
                    let frac = sim.time_s() as f32 / sim.case.duration_s().max(1) as f32;
                    ui.add(egui::ProgressBar::new(frac).desired_height(5.0));
                });
            });
            if sim.paused() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(if matches!(k.selection, Selection::Team(_)) { rocca::tr!("Tocca la destinazione") } else { rocca::tr!("Pausa") }).strong().color(GREEN));
                    for (color, label) in crate::forecast_view::LINE_COLOURS.into_iter().zip([rocca::tr!("1 h"), rocca::tr!("3 h")]) {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        ui.painter().line_segment([r.left_center(), r.right_center()], egui::Stroke::new(3.0, color));
                        ui.colored_label(color, label);
                    }
                    if let Some((at, w)) = sim.weather_ahead() {
                        ui.label(rocca::tr!("{0}: {1} km/h da {2}", clock(at), format!("{:.0}", w.wind_speed_kmh), rocca::words::compass(w.wind_dir_deg)));
                    }
                });
            }
        });
    });
}


fn drop_rect(ctx: &egui::Context, k: &Kiosk) -> egui::Rect {
    let mut r = ctx.viewport_rect();
    if compact(ctx) {
        r.min.y += 116.0;
        r.max.x -= k.side_w;
        r.max.y -= if k.phase == Phase::Tutorial && k.side_w == 0.0 { (ctx.viewport_rect().height() * 0.20).clamp(42.0, 120.0) + 126.0 } else { 58.0 };
    }
    r
}

/// Speed ramps up over the last 48 logical pixels of the usable map.
/// Outside it (HUD, tutorial panel or browser edge), the drag must not pan.
fn edge_pan(rect: egui::Rect, p: egui::Pos2) -> Vec2 {
    if !rect.contains(p) { return Vec2::ZERO; }
    let axis = |p: f32, lo: f32, hi: f32| {
        let band = 48.0_f32.min((hi - lo) * 0.25);
        if band <= 0.0 { return 0.0; }
        let negative = (1.0 - (p - lo) / band).clamp(0.0, 1.0);
        let positive = (1.0 - (hi - p) / band).clamp(0.0, 1.0);
        positive * positive - negative * negative
    };
    Vec2::new(axis(p.x, rect.left(), rect.right()), axis(p.y, rect.top(), rect.bottom())).clamp_length_max(1.0)
}
