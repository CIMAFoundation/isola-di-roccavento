//! The screens around the game, in the style of the first kiosk: the opening
//! (the volunteer, the people you work with, the name, today's board), the
//! tutorial's bubble, the day-2 card and the end (big stars, a verdict, your
//! place on the board). Faces speak, words are few, buttons big.
//!
//! What they say comes from the game (the tutorial script, the outcome, the
//! stars); no rule lives here.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, RichText};
use rocca::tutorial::Expect;

use super::characters::{self, Logo, Mood, Portraits, Who};
use super::ui::{card, primary, secondary, AMBER, GREY, RED};
use super::{board, choose_scenario, new_visit, start_game, start_tutorial, Kiosk, Shot};
use scenario::Pos;
use crate::sim::{Sim, SimRestarted};

const INK: Color32 = Color32::from_rgb(18, 20, 24);

/// The dim behind a full-screen card.
fn backdrop(ctx: &egui::Context, id: &str, alpha: u8) {
    let screen = ctx.viewport_rect();
    egui::Area::new(egui::Id::new(id)).fixed_pos(screen.min).order(egui::Order::Middle).interactable(false).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0.0, Color32::from_rgba_premultiplied(8, 10, 14, alpha));
    });
}

/// The big card of the opening and the end.
fn big_card() -> egui::Frame {
    egui::Frame::NONE
        .fill(INK)
        .stroke(egui::Stroke::new(1.5, AMBER.gamma_multiply(0.55)))
        .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 40, spread: 4, color: Color32::from_black_alpha(160) })
        .corner_radius(18.0)
        .inner_margin(egui::Margin { left: 36, right: 36, top: 28, bottom: 24 })
}

/// The volunteer speaking, with her portrait.
fn volunteer(ui: &mut egui::Ui, faces: Option<&Portraits>, mood: Mood, size: f32, text: &str, width: f32) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(6.0);
            characters::portrait(ui, faces, Who::Volontaria, mood, size);
        });
        ui.add_space(18.0);
        characters::bubble(ui, Who::Volontaria, |ui| characters::says(ui, Who::Volontaria, text, width));
    });
}

// --- the title ----------------------------------------------------------------

/// The title screen: the fire plays behind it with the roaming camera, here
/// only the name and «Gioca». True when «Gioca» was pressed.
// --- the opening sequence (user, 2026-10-30) ----------------------------------
// At launch, real seconds: the CIMA logo on black with the jingle's climax;
// then, to its cadence, the black lifts on the island at dawn, the camera
// flying in to it fast and slowing down, as from a plane; the title and
// «Gioca» come as the cadence dies away. A click or a key skips it. The times
// follow the two cuts' envelopes (`tools/sounds/prepare.py`), without gaps
// (user, 2026-10-30). The jingle plays on the real clock (`sound`): this one
// slows down when the first frames are long, on the web above all.

const LOGO_IN_S: f32 = 0.8;
/// The jingle's climax (`sound`), as the logo comes up: 0.3 s after the game
/// starts (user, 2026-11-02).
pub const JINGLE_S: f32 = 0.3;
/// The jingle's cadence, 0.6 s into the climax, while it is still at its
/// loudest (user, 2026-11-02: «diminuisci ancora la distanza»; 1.7 s in, at
/// −22 dB, sounded like a gap, and 0.9 s, at −17 dB, still had a dip).
pub const JINGLE_ISLAND_S: f32 = JINGLE_S + 0.6;
/// The logo starts to fade out (as at 0.9 s, when the climax decays).
const LOGO_OUT_S: f32 = JINGLE_S + 0.9;
/// The black starts to lift on the island.
pub const ISLAND_S: f32 = LOGO_OUT_S + 0.4;
/// No black left.
const CLEAR_S: f32 = 3.8;
/// The title starts to show, and is all there one second later.
const TITLE_S: f32 = 3.6;
/// The end: the camera has arrived and is the director's again (`view`).
pub const OPENING_S: f32 = 8.0;

fn ramp(t: f32, from: f32, to: f32) -> f32 {
    let x = ((t - from) / (to - from)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How far the camera has come in to the island, 0..1: from the start (under
/// the black) fast, then slower and slower, landing at rest.
pub fn opening_descent(t: f32) -> f32 {
    1.0 - (1.0 - (t / OPENING_S).clamp(0.0, 1.0)).powi(2)
}

/// The director's share of the camera after the opening (`view`): from
/// nothing to all of it in `HANDOFF_S`, so the flight in slows to rest and
/// the shot drifts off from there, with no jolt.
pub fn handoff(after: f32) -> f32 {
    ramp(after, 0.0, HANDOFF_S)
}
const HANDOFF_S: f32 = 4.0;

/// The title screen's opacity: hidden in the opening until its last seconds.
pub fn title_alpha(opening: Option<f32>) -> f32 {
    opening.map_or(1.0, |t| ramp(t, TITLE_S, TITLE_S + 1.0))
}

/// The black and the logo over everything, `t` seconds into the opening.
pub fn opening(ctx: &egui::Context, t: f32, logo: Option<&Logo>) {
    let screen = ctx.viewport_rect();
    let black = 1.0 - ramp(t, ISLAND_S, CLEAR_S);
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("apertura")));
    if black > 0.0 {
        p.rect_filled(screen, 0.0, Color32::from_black_alpha((black * 255.0) as u8));
    }
    let shown = ramp(t, 0.0, LOGO_IN_S) * (1.0 - ramp(t, LOGO_OUT_S, ISLAND_S));
    if let (Some(l), true) = (logo, shown > 0.0) {
        // the brand file is 77 x 96
        let h = (screen.height() * 0.32).min(screen.width() * 0.4 * 96.0 / 77.0);
        let r = egui::Rect::from_center_size(screen.center(), egui::vec2(h * 77.0 / 96.0, h));
        p.image(l.for_height(ctx, h), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha((shown * 255.0) as u8));
    }
    ctx.request_repaint();
}

pub fn title(ctx: &egui::Context, k: &Kiosk, sim: &Sim, logo: Option<&Logo>, alpha: f32) -> bool {
    let screen = ctx.viewport_rect();
    // phones get the same screen, scaled down to the width
    let sc = (screen.width() / 1000.0).clamp(0.4, 1.0).min((screen.height() / 560.0).max(0.4));
    let mut play = false;
    egui::Area::new(egui::Id::new("titolo_fondo")).fixed_pos(screen.min).order(egui::Order::Middle).interactable(false).show(ctx, |ui| {
        ui.set_opacity(alpha);
        let p = ui.painter();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(70));
        // two darker bands, where the title and the button are, and clear
        // between them, so the middle of the screen keeps the fire: one mesh
        // whose vertex opacity follows two bells, so there is no edge or seam
        let (h, n) = (800.0f32.min(screen.height()), 64);
        let mut mesh = egui::Mesh::default();
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let bell = (std::f32::consts::TAU * t).sin().powi(2);
            let y = screen.center().y - h / 2.0 + h * t;
            let c = Color32::from_black_alpha((170.0 * bell) as u8);
            mesh.colored_vertex(egui::pos2(screen.left(), y), c);
            mesh.colored_vertex(egui::pos2(screen.right(), y), c);
            if i > 0 {
                let j = 2 * i as u32;
                mesh.add_triangle(j - 2, j - 1, j);
                mesh.add_triangle(j - 1, j + 1, j);
            }
        }
        p.add(egui::Shape::mesh(mesh));
    });
    // the title above the middle, the button below it: the fire shows between
    let off = (screen.height() * 0.2).clamp(120.0 * sc, 220.0);
    egui::Area::new(egui::Id::new("titolo")).anchor(Align2::CENTER_CENTER, [0.0, -off]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.set_opacity(alpha);
        ui.set_width((screen.width() - 24.0).min(900.0));
        ui.vertical_centered(|ui| {
            if let Some(l) = logo {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(l.for_height(ctx, 96.0 * sc), egui::vec2(77.0 * sc, 96.0 * sc))));
                ui.add_space(6.0);
            }
            ui.label(RichText::new(super::TITLE).size(84.0 * sc).strong().color(Color32::WHITE));
            ui.label(RichText::new(rocca::tr!("Aiutaci a gestire un incendio.")).size(26.0 * sc.max(0.6)).color(AMBER));
        });
    });
    egui::Area::new(egui::Id::new("titolo_gioca")).anchor(Align2::CENTER_CENTER, [0.0, off]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.set_opacity(alpha);
        ui.vertical_centered(|ui| {
            let b = egui::Button::new(RichText::new(rocca::tr!("Gioca")).size(44.0 * sc.max(0.7)).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(16.0).min_size(egui::vec2(360.0 * sc.max(0.7), 96.0 * sc.max(0.6)));
            let r = ui.add(b);
            crate::playtest::control("Gioca", &r, true);
            play = super::icons::pointing(r).clicked();
        });
    });
    // left of the language button (`lang::button`, 82 px wide)
    egui::Area::new(egui::Id::new("titolo_nota")).anchor(Align2::RIGHT_BOTTOM, [-104.0, -24.0]).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
        ui.set_opacity(alpha);
        ui.label(RichText::new(rocca::tr!("Simulazione dimostrativa · {0} · versione {1}", k.territory.levels.iter().find(|l| l.case == sim.case.name).map_or(sim.case.name.as_str(), |l| l.name.as_str()), super::BUILD)).size(14.0 * sc.max(0.7)).color(GREY.gamma_multiply(0.75)));
    });
    ctx.request_repaint();
    play
}

// --- the opening --------------------------------------------------------------

pub fn intro(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, logo: Option<&Logo>, faces: Option<&Portraits>) {
    if super::ui::compact(ctx) {
        compact_intro(ctx, k, sim, restarted);
        return;
    }
    let screen = ctx.viewport_rect();
    backdrop(ctx, "intro_fondo", 190);
    // one centred card: the volunteer on the left, today's board on the right
    let (panel_w, gap) = (290.0, 28.0);
    let card_w = (screen.width() - 72.0 - 48.0).min(900.0);
    let mut go = None;
    egui::Area::new(egui::Id::new("intro")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        big_card().show(ui, |ui| {
            ui.set_width(card_w);
            ui.horizontal(|ui| {
                if let Some(l) = logo {
                    ui.add(egui::Image::new(egui::load::SizedTexture::new(l.for_height(ui.ctx(), 96.0), egui::vec2(77.0, 96.0))));
                    ui.add_space(14.0);
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(super::TITLE).size(44.0).strong().color(Color32::WHITE));
                    ui.label(RichText::new(rocca::tr!("Aiutaci a gestire questa emergenza.")).size(22.0).color(AMBER));
                });
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            let teams = k.territory.roster.len();
            let text = rocca::tr!("Un incendio minaccia i borghi dell'isola! Abbiamo {0} squadre AIB, ma non bastano per tutti. Aiutaci a proteggere persone e case.", teams);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(card_w - panel_w - gap);
                    volunteer(ui, faces, Mood::Preoccupato, 110.0, &text, (card_w - panel_w - gap - 230.0).max(260.0));
                });
                ui.add_space(gap);
                egui::Frame::NONE
                    .fill(Color32::from_white_alpha(8))
                    .stroke(egui::Stroke::new(1.0, AMBER.gamma_multiply(0.35)))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin { left: 16, right: 16, top: 14, bottom: 10 })
                    .show(ui, |ui| {
                        ui.set_width(panel_w - 34.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(rocca::tr!("Classifica di oggi")).size(22.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(rocca::tr!("le stelle di Persone, Case, Ettari")).size(13.0).color(GREY));
                            ui.add_space(8.0);
                            let rows = if screen.height() >= 900.0 { rocca::score::BOARD_SHOWN } else { 4 };
                            board_rows(ui, k, &sim.case.name, None, rows);
                        });
                    });
            });
            ui.add_space(12.0);
            ui.label(RichText::new(rocca::tr!("LE PERSONE CON CUI LAVORI")).size(13.0).strong().color(GREY.gamma_multiply(0.7)));
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for who in [Who::Volontaria, Who::Esperto, Who::Vigili] {
                    egui::Frame::NONE
                        .fill(who.colour().gamma_multiply(0.10))
                        .stroke(egui::Stroke::new(1.0, who.colour().gamma_multiply(0.35)))
                        .corner_radius(40.0)
                        .inner_margin(egui::Margin { left: 6, right: 14, top: 6, bottom: 6 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                characters::portrait(ui, faces, who, Mood::Calmo, 48.0);
                                ui.label(RichText::new(who.short()).size(16.0).strong().color(who.colour()));
                            });
                        });
                }
            });
            ui.add_space(12.0);
            ui.label(RichText::new(rocca::tr!("Alla fine vedrai com'è andata, confrontata con lo stesso incendio senza ordini.")).size(15.0).italics().color(GREY));
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(rocca::tr!("Come ti chiami?")).size(20.0).strong().color(Color32::WHITE));
                ui.add_space(8.0);
                let edit = egui::TextEdit::singleline(&mut k.player)
                    .hint_text(rocca::tr!("Il tuo nome"))
                    .font(egui::FontId::proportional(24.0))
                    .char_limit(rocca::score::NAME_MAX)
                    .desired_width(320.0)
                    .margin(egui::vec2(12.0, 8.0));
                let r = ui.add(edit);
                crate::playtest::control("Nome", &r, true);
                // nothing else on this screen takes typing
                if !r.has_focus() {
                    r.request_focus();
                }
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let w = (card_w - 16.0) / 2.0;
                let b = egui::Button::new(RichText::new(rocca::tr!("TUTORIAL")).size(28.0).strong().color(Color32::WHITE))
                    .fill(Color32::from_gray(62))
                    .stroke(egui::Stroke::new(1.0, Color32::from_gray(110)))
                    .corner_radius(12.0)
                    .min_size(egui::vec2(w, 64.0));
                let r = ui.add(b);
                crate::playtest::control("Tutorial", &r, true);
                if super::icons::pointing(r).clicked() {
                    go = Some(true);
                }
                let b = egui::Button::new(RichText::new(rocca::tr!("INIZIA")).size(28.0).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(12.0).min_size(egui::vec2(w, 64.0));
                let r = ui.add(b);
                crate::playtest::control("Inizia", &r, true);
                if super::icons::pointing(r).clicked() {
                    go = Some(false);
                }
            });
            ui.add_space(10.0);
            ui.label(
                RichText::new(rocca::tr!("Territorio immaginario, fuoco simulato con PROPAGATOR di Fondazione CIMA. Nella realtà segui sempre le indicazioni delle autorità."))
                    .size(13.0)
                    .color(GREY.gamma_multiply(0.75)),
            );
        });
    });
    if go.is_some() {
        k.player = rocca::score::name(&k.player);
    }
    match go {
        Some(true) => start_tutorial(k, sim, restarted),
        Some(false) => choose_scenario(k),
        None => {}
    }
}

// --- the tutorial -------------------------------------------------------------

/// The volunteer's bubble at the top of the map: the step's text, «Avanti»
/// when the step only asks to read, «Salta» always. The map stays usable:
/// the step asks the player to act on it.
pub fn tutorial(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, faces: Option<&Portraits>, _cam: (&Camera, &GlobalTransform)) {
    let Some(step) = k.script.as_ref().and_then(|s| s.steps.get(k.step)).cloned() else { return };
    if k.shot_for != Some(k.step) {
        fly_to(k, sim, step.expect);
    }
    highlight(ctx, k, step.expect);
    if super::ui::compact(ctx) {
        compact_tutorial(ctx, k, sim, restarted, &step.touch, step.expect);
        return;
    }
    let total = k.script.as_ref().map_or(0, |s| s.steps.len());
    let mut skip = false;
    let t = ctx.input(|i| i.time) as f32;
    egui::Area::new(egui::Id::new("tutorial")).anchor(Align2::CENTER_TOP, [0.0, 10.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                characters::portrait(ui, faces, Who::Volontaria, Mood::Calmo, 130.0);
            });
            ui.add_space(18.0);
            characters::bubble(ui, Who::Volontaria, |ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(650.0);
                    // «benvenuto/a» would guess a gender: greet by the name
                    let hello = if k.player.is_empty() { rocca::tr!("Ciao, grazie dell'aiuto!").to_string() } else { rocca::tr!("Ciao {0}, grazie dell'aiuto!", k.player) };
                    let text = if k.step == 0 { format!("{hello} {}", step.text) } else { step.text.clone() };
                    characters::says(ui, Who::Volontaria, &text, 650.0);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let r = ui.add(secondary(rocca::tr!("Salta il tutorial")));
                        crate::playtest::control("Salta il tutorial", &r, true);
                        skip = r.clicked();
                        ui.label(RichText::new(format!("{}/{total}", k.step + 1)).size(15.0).color(Color32::from_gray(110)));
                        if step.expect == Expect::Next {
                            let label = rocca::tr!("Avanti");
                            let w = 180.0;
                            ui.add_space((ui.available_width() - w).max(0.0));
                            let r = ui.add(primary(label).min_size(egui::vec2(w, 48.0)));
                            crate::playtest::control("Avanti", &r, true);
                            if r.clicked() {
                                k.done(Expect::Next);
                            }
                        } else {
                            // the step waits for the player's own action
                            let glow = 0.55 + 0.45 * (t * 3.0).sin();
                            let hint = if matches!(step.expect, Expect::Run(_)) { rocca::tr!("guarda") } else { rocca::tr!("tocca a te") };
                            ui.label(RichText::new(hint).size(16.0).strong().color(AMBER.gamma_multiply(glow)));
                        }
                    });
                });
            });
        });
    });
    ctx.request_repaint();
    if skip || (!ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
        start_game(k, sim, restarted);
    }
}

/// The camera flies to what the step is about; when the fire runs by itself
/// it follows it again.
fn fly_to(k: &mut Kiosk, sim: &Sim, expect: Expect) {
    k.shot_for = Some(k.step);
    k.step_t = 0.0;
    let ignition = Some(sim.case.ignition());
    let mid = |a: Pos, b: Pos| Some(Pos { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5 });
    k.follow = false;
    k.shot = Some(match expect {
        // the first step: where the fire started
        _ if k.step == 0 => Shot { at: ignition, zoom: 0.5 },
        Expect::Move(t, hex) => {
            let team = &sim.teams[t];
            let home = Pos::from(sim.territory.stations[team.station].pos);
            let from = team.hex.map_or(home, |h| h.centre());
            Shot { at: mid(from, hex.centre()), zoom: 0.7 }
        }
        Expect::Evacuate(d) => Shot { at: Some(sim.districts[d].centre), zoom: 0.55 },
        Expect::Over | Expect::Run(_) => {
            k.follow = true;
            return k.shot = None;
        }
        _ => Shot { at: None, zoom: 1.0 },
    });
}

/// A glowing frame on what the step asks the player to touch, for a few
/// seconds: the pause button, the team's flag and the hex, the district.
fn highlight(ctx: &egui::Context, k: &Kiosk, expect: Expect) {
    if k.step == 0 {
        return;
    }
    let area = |id: egui::Id| ctx.memory(|m| m.area_rect(id));
    let targets: Vec<Option<egui::Rect>> = match expect {
        Expect::Pause | Expect::Resume => vec![area(egui::Id::new("barra")), area(egui::Id::new("barra_mobile"))],
        // no box on a drag: the pulsing rings on the map (`ui::tutorial_pulse`) mark it
        Expect::Move(..) => vec![],
        Expect::Evacuate(d) => vec![area(egui::Id::new(("borgo", d)))],
        _ => vec![],
    };
    // the frame keeps pulsing until the player does what the step asks
    let fade = 1.0;
    let t = ctx.input(|i| i.time) as f32;
    let glow = (0.7 + 0.3 * (t * 4.0).sin()) * fade;
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("tutorial_evidenza")));
    for r in targets.into_iter().flatten() {
        let r = r.expand(8.0);
        // a dark edge under the amber one, so it reads on any ground
        p.rect_stroke(r, 12.0, egui::Stroke::new(8.0, Color32::from_black_alpha((160.0 * fade) as u8)), egui::StrokeKind::Middle);
        p.rect_stroke(r, 12.0, egui::Stroke::new(5.0, AMBER.gamma_multiply(glow)), egui::StrokeKind::Middle);
        p.rect_stroke(r.expand(8.0), 18.0, egui::Stroke::new(2.0, AMBER.gamma_multiply(glow * 0.6)), egui::StrokeKind::Middle);
    }
}

// --- the scenario ---------------------------------------------------------------

/// The label of a scenario's difficulty and its colour.
fn difficulty_style(d: &str) -> (String, Color32) {
    let (word, colour) = match d.to_lowercase().as_str() {
        "facile" => (rocca::tr!("facile"), Color32::from_rgb(120, 200, 110)),
        "medio" => (rocca::tr!("medio"), Color32::from_rgb(240, 200, 70)),
        "difficile" => (rocca::tr!("difficile"), Color32::from_rgb(240, 140, 60)),
        _ => (rocca::tr!("estremo"), Color32::from_rgb(230, 80, 70)),
    };
    let word = if matches!(d.to_lowercase().as_str(), "facile" | "medio" | "difficile") { word.to_string() } else { d.to_string() };
    (format!(" {} ", word.to_uppercase()), colour)
}

/// After «Inizia»: the fire to fight (the four named scenarios) and the level.
pub fn scenario(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, icons: Option<&super::characters::ScenarioIcons>) {
    let compact = super::ui::compact(ctx);
    let mut go = false;
    let mut back = false;
    let body = |ui: &mut egui::Ui, k: &mut Kiosk, wide: bool| {
        ui.label(RichText::new(rocca::tr!("Scegli l'incendio")).size(if wide { 36.0 } else { 22.0 }).strong().color(Color32::WHITE));
        ui.add_space(6.0);
        let levels = k.territory.levels.clone();
        let card_w = if wide { (ui.available_width() - 14.0) / 2.0 } else { ui.available_width() };
        let pick = |ui: &mut egui::Ui, k: &mut Kiosk, i: usize| {
            let l = &levels[i];
            let on = k.selected_case == l.case;
            let fill = if on { AMBER.gamma_multiply(0.20) } else { Color32::from_white_alpha(8) };
            let stroke = if on { egui::Stroke::new(3.0, AMBER) } else { egui::Stroke::new(1.0, Color32::from_gray(90)) };
            let icon = if wide { 148.0 } else { 88.0 };
            let (name, colour) = difficulty_style(&l.difficulty);
            let r = egui::Frame::NONE.fill(fill).stroke(stroke).corner_radius(14.0).inner_margin(10.0).show(ui, |ui| {
                ui.set_width(card_w - 22.0);
                ui.set_min_height(icon);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(icon, icon), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 10.0, Color32::from_gray(30));
                    if let Some(t) = icons.and_then(|x| x.get(&l.case)) {
                        egui::Image::new(egui::load::SizedTexture::new(t, rect.size())).corner_radius(10.0).paint_at(ui, rect);
                    }
                    ui.painter().rect_stroke(rect, 10.0, egui::Stroke::new(2.0, colour), egui::StrokeKind::Middle);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(RichText::new(name.trim()).size(if wide { 14.0 } else { 12.0 }).strong().color(Color32::BLACK).background_color(colour));
                        ui.add(egui::Label::new(RichText::new(&l.name).size(if wide { 21.0 } else { 17.0 }).strong().color(if on { AMBER } else { Color32::WHITE })).wrap());
                        ui.add(egui::Label::new(RichText::new(&l.about).size(if wide { 15.0 } else { 13.0 }).color(GREY)).wrap());
                    });
                });
            }).response.interact(egui::Sense::click());
            crate::playtest::control(&l.name, &r, true);
            if super::icons::pointing(r).clicked() {
                k.selected_case = l.case.clone();
            }
        };
        if wide {
            for row in 0..levels.len().div_ceil(2) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    for i in [row * 2, row * 2 + 1] {
                        if i < levels.len() { pick(ui, k, i); }
                    }
                });
                ui.add_space(10.0);
            }
        } else {
            for i in 0..levels.len() {
                pick(ui, k, i);
                ui.add_space(6.0);
            }
        }
    };
    let footer = |ui: &mut egui::Ui, go: &mut bool, back: &mut bool, wide: bool| {
        ui.add_space(8.0);
        let full = ui.available_width();
        ui.horizontal(|ui| {
            let back_w = if wide { 160.0 } else { full * 0.3 };
            let r = ui.add_sized([back_w, 60.0], secondary(rocca::tr!("Indietro")));
            crate::playtest::control("Indietro", &r, true);
            if r.clicked() { *back = true; }
            let r = ui.add_sized([full - back_w - ui.spacing().item_spacing.x, 60.0], egui::Button::new(RichText::new(rocca::tr!("PARTI")).size(28.0).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(12.0));
            crate::playtest::control("Parti", &r, true);
            if super::icons::pointing(r).clicked() { *go = true; }
        });
    };
    if compact {
        // «Indietro» and «PARTI» stay in view: on a landscape phone the cards fill the screen.
        compact_page_footer(ctx, "scenario_mobile", |ui| body(ui, k, false), |ui| footer(ui, &mut go, &mut back, false));
    } else {
        backdrop(ctx, "scenario_fondo", 170);
        egui::Area::new(egui::Id::new("scenario")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            big_card().show(ui, |ui| {
                ui.set_width(860.0);
                body(ui, k, true);
                footer(ui, &mut go, &mut back, true);
            });
        });
    }
    if go {
        start_game(k, sim, restarted);
    } else if back {
        new_visit(k, sim, restarted);
    }
}

// --- the end ------------------------------------------------------------------

/// One star, filled or empty, `size` across.
fn star(ui: &mut egui::Ui, fill: Color32, size: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let c = r.center() + egui::vec2(0.0, size * 0.05);
    let pts: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rad = if i % 2 == 0 { size * 0.5 } else { size * 0.2 };
            c + egui::vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    let p = ui.painter();
    // a star is not convex: the inner pentagon, then a triangle per point
    p.add(egui::Shape::convex_polygon((0..5).map(|i| pts[2 * i + 1]).collect(), fill, egui::Stroke::NONE));
    for i in 0..5 {
        p.add(egui::Shape::convex_polygon(vec![pts[2 * i], pts[(2 * i + 1) % 10], pts[(2 * i + 9) % 10]], fill, egui::Stroke::NONE));
    }
}

/// The three star rows, big: the category, its stars, what it counts.
fn big_stars(ui: &mut egui::Ui, st: &rocca::score::Stars, size: f32) {
    for (name, n, what) in [
        (rocca::tr!("Persone"), st.persone, rocca::tr!("persone che il fuoco non ha raggiunto")),
        (rocca::tr!("Case"), st.case, rocca::tr!("case che il fuoco non ha colpito")),
        (rocca::tr!("Ettari"), st.ettari, rocca::tr!("bosco che non è bruciato")),
    ] {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(120.0, size), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(120.0);
                ui.label(RichText::new(name).size(24.0).strong().color(Color32::WHITE));
            });
            match n {
                Some(n) => {
                    for i in 0..5 {
                        star(ui, if i < n as usize { AMBER } else { Color32::from_gray(70) }, size);
                    }
                }
                None => {
                    ui.label(RichText::new(rocca::tr!("nessun danno nel riferimento")).size(15.0).color(GREY));
                }
            }
        });
        ui.label(RichText::new(what).size(14.0).color(GREY));
        ui.add_space(4.0);
    }
}

/// The volunteer's verdict on the people, from the two outcomes.
fn verdict(name: &str, mine: &rocca::Outcome, none: &rocca::Outcome) -> (Mood, String) {
    let (caught, saved) = (mine.caught(), none.caught().saturating_sub(mine.caught()));
    let (mood, text) = if caught == 0 && saved > 0 {
        (Mood::Calmo, rocca::tr!("Il fuoco non ha raggiunto nessuno. Senza i tuoi ordini avrebbe colpito {0} persone: hai fatto la differenza!", saved))
    } else if caught == 0 {
        (Mood::Calmo, rocca::tr!("Il fuoco non ha raggiunto nessuno. Vediamo insieme come sono andate anche le altre cose.").to_string())
    } else if saved > 0 {
        (Mood::Preoccupato, rocca::tr!("Con i tuoi ordini il fuoco ha raggiunto {0} persone in meno. Ne ha colpite {1}: proviamo a migliorare ancora.", saved, caught))
    } else {
        (Mood::Preoccupato, rocca::tr!("Il fuoco ha raggiunto {0} persone. Puoi riprovare: la previsione ti aiuta a capire quando evacuare.", caught))
    };
    (mood, characters::to(name, &text))
}

/// Same debrief on every screen size, including when the baseline fails.
fn learning(ui: &mut egui::Ui, mine: &rocca::Outcome, none: Option<&rocca::Outcome>) {
    if let Some(none) = none {
        let report = rocca::score::debrief(mine, none);
        ui.label(RichText::new(rocca::tr!("Confronto con la partita senza ordini")).strong().size(18.0).color(Color32::WHITE));
        for text in &report.results {
            ui.add(egui::Label::new(RichText::new(text).size(15.0).color(GREY)).wrap());
        }
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(report.next_try).size(15.0).color(AMBER)).wrap());
    }
    if mine.units_lost > 0 {
        ui.label(RichText::new(rocca::tr!("Unità di soccorso perse: {0}.", mine.units_lost)).color(AMBER));
    }
    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new(rocca::tr!("Nella realtà: protezione civile")).strong().size(18.0).color(Color32::WHITE));
    for (title, text) in rocca::score::civil_protection() {
        ui.label(RichText::new(title).strong().size(15.0).color(AMBER));
        ui.add(egui::Label::new(RichText::new(text).size(15.0).color(GREY)).wrap());
    }
    ui.hyperlink_to(rocca::tr!("Fonte: Io non rischio · Protezione Civile"), rocca::score::CIVIL_PROTECTION_SOURCE);
    ui.add(egui::Label::new(RichText::new(rocca::tr!("Quest'isola è immaginaria. Nella realtà, segui sempre le indicazioni delle autorità.")).size(13.0).italics().color(GREY)).wrap());
}

/// A level's board, the best `rows`; the player's own row (`mine`: name and
/// time stamp) lit, and appended with its place when it ranks lower.
fn board_rows(ui: &mut egui::Ui, k: &Kiosk, case: &str, mine: Option<(&str, u64)>, rows: usize) {
    let (day, _) = board::now();
    let ranking = k.board.ranking(case, &day);
    if ranking.is_empty() {
        ui.label(RichText::new(rocca::tr!("Nessuna partita oggi")).size(14.0).color(GREY.gamma_multiply(0.8)));
        return;
    }
    let is_mine = |e: &rocca::score::Played| mine.is_some_and(|(n, at)| e.at == at && e.name == n);
    let mut shown: Vec<(usize, &rocca::score::Played)> = ranking.iter().copied().enumerate().take(rows).collect();
    if let Some(p) = ranking.iter().position(|e| is_mine(e)).filter(|&p| p >= rows) {
        shown.push((p, ranking[p]));
    }
    for (i, e) in shown {
        let me = is_mine(e);
        let ink = if me { Color32::BLACK } else { Color32::WHITE };
        let s: Vec<String> = e.stars.iter().map(|s| s.map_or("–".into(), |n| n.to_string())).collect();
        egui::Frame::NONE.fill(if me { AMBER } else { Color32::TRANSPARENT }).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_sized([22.0, 20.0], egui::Label::new(RichText::new(format!("{}", i + 1)).size(15.0).strong().color(if me { ink } else { GREY })));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(s.join(" ")).size(16.0).strong().monospace().color(ink));
                    star(ui, if me { Color32::BLACK } else { AMBER }, 15.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&e.name).size(16.0).strong().color(ink)).truncate());
                    });
                });
            });
        });
    }
}

/// «Sei al 3º posto su 12 oggi».
fn place_words(place: usize, of: usize) -> String {
    match place {
        0 if of == 1 => rocca::tr!("Sei il primo di oggi").to_string(),
        0 => rocca::tr!("Sei al 1º posto su {0} oggi", of),
        p => rocca::tr!("Sei al {0}º posto su {1} oggi", p + 1, of),
    }
}

pub fn end(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, faces: Option<&Portraits>) {
    let mine = sim.outcome();
    let base = k.baseline();
    // on the board once, when the yardstick is ready
    if let (Some(Ok(none)), None) = (&base, &k.saved) {
        let st = rocca::score::stars(&mine, none);
        let (day, at) = board::now();
        let name = if k.player.trim().is_empty() { "Anonimo".to_string() } else { rocca::score::name(&k.player) };
        k.board.push(rocca::score::Played { name, level: sim.case.name.clone(), stars: st.all(), day: day.clone(), at });
        if let Err(e) = board::save(&k.board) {
            k.error = Some(e);
        }
        k.saved = Some((day, at));
    }
    if super::ui::compact(ctx) {
        let mut again = false;
        let mut retry = false;
        // «Nuova partita» stays in view under the scrolling results
        compact_page_footer(ctx, "fine_mobile", |ui| {
            ui.heading(if k.player.trim().is_empty() { rocca::tr!("Com'è andata").to_string() } else { rocca::tr!("Com'è andata, {0}", k.player.trim()) });
            match &base {
                None => { ui.label(rocca::tr!("Confronto con lo stesso incendio senza ordini…")); }
                Some(Err(e)) => { ui.colored_label(RED, e); }
                Some(Ok(none)) => {
                    let st = rocca::score::stars(&mine, none);
                    for (label, n) in [(rocca::tr!("Persone"), st.persone), (rocca::tr!("Case"), st.case), (rocca::tr!("Ettari"), st.ettari)] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).size(18.0));
                            if let Some(n) = n {
                                for i in 0..5 { star(ui, if i < n { AMBER } else { GREY.gamma_multiply(0.3) }, 22.0); }
                            } else { ui.label(rocca::tr!("nessun danno nel riferimento")); }
                        });
                    }
                    ui.label(verdict(&k.player, &mine, none).1);
                }
            }
            learning(ui, &mine, base.as_ref().and_then(|r| r.as_ref().ok()));
            ui.separator();
            ui.heading(rocca::tr!("Classifica di oggi"));
            let name = rocca::score::name(&k.player);
            board_rows(ui, k, &sim.case.name, k.saved.as_ref().map(|(_, at)| (name.as_str(), *at)), rocca::score::BOARD_SHOWN);
        }, |ui| {
            retry = action(ui, rocca::tr!("Riprova"));
            again = action(ui, rocca::tr!("Nuova partita"));
        });
        if retry { super::retry(k, sim, restarted); }
        else if again { new_visit(k, sim, restarted); }
        return;
    }
    backdrop(ctx, "fine_fondo", 200);
    let screen = ctx.viewport_rect();
    let side = 340.0;
    let inner = (screen.width() - 48.0 - 72.0).min(1060.0);
    let left = (inner - side - 24.0).max(420.0);
    let name = if k.player.trim().is_empty() { "Anonimo".to_string() } else { rocca::score::name(&k.player) };
    let mut again = false;
    let mut retry = false;
    egui::Area::new(egui::Id::new("fine")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        big_card().show(ui, |ui| {
            ui.set_width(inner);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(left, (screen.height() - 150.0).max(160.0)), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(left);
                    egui::ScrollArea::vertical().id_salt("fine_spiegazione").max_height((screen.height() - 150.0).max(160.0)).show(ui, |ui| {
                        let title = if k.player.trim().is_empty() { rocca::tr!("Com'è andata").to_string() } else { rocca::tr!("Com'è andata, {0}", k.player.trim()) };
                        ui.label(RichText::new(title).size(34.0).strong().color(Color32::WHITE));
                        ui.add_space(12.0);
                        match &base {
                            None => {
                                ui.label(RichText::new(rocca::tr!("Confronto con lo stesso incendio senza ordini…")).size(20.0).color(GREY));
                            }
                            Some(Err(e)) => {
                                ui.colored_label(RED, e);
                            }
                            Some(Ok(none)) => {
                                big_stars(ui, &rocca::score::stars(&mine, none), 36.0);
                                ui.add_space(10.0);
                                let (mood, text) = verdict(&k.player, &mine, none);
                                volunteer(ui, faces, mood, 90.0, &text, left - 190.0);
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(rocca::tr!(
                                        "Tu: {0} persone e {1} case colpite, {2} ettari bruciati. Senza ordini: {3} persone, {4} case, {5} ettari.",
                                        mine.caught(),
                                        mine.homes_hit(),
                                        format!("{:.0}", mine.hectares),
                                        none.caught(),
                                        none.homes_hit(),
                                        format!("{:.0}", none.hectares)
                                    ))
                                    .size(15.0)
                                    .color(GREY),
                                );
                            }
                        }
                        ui.add_space(10.0);
                        learning(ui, &mine, base.as_ref().and_then(|r| r.as_ref().ok()));
                    });
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.set_width(side);
                    card(AMBER).show(ui, |ui| {
                        ui.set_width(side - 36.0);
                        let stamp = k.saved.as_ref().map(|(_, at)| (name.as_str(), *at));
                        if let Some((day, at)) = &k.saved {
                            let (place, of) = (k.board.place(&sim.case.name, day, &name, *at), k.board.count(&sim.case.name, day));
                            if let Some(p) = place {
                                ui.add(egui::Label::new(RichText::new(place_words(p, of)).size(22.0).strong().color(Color32::WHITE)).wrap());
                            }
                        }
                        ui.label(RichText::new(rocca::tr!("Classifica di oggi")).size(14.0).color(GREY));
                        ui.add_space(6.0);
                        board_rows(ui, k, &sim.case.name, stamp, rocca::score::BOARD_SHOWN);
                    });
                    ui.add_space(12.0);
                    ui.add(egui::Label::new(RichText::new(rocca::tr!("Nella realtà, quando arriva l'ordine di evacuazione si parte subito. Aspettare di vedere il fuoco è uno degli errori più pericolosi.")).size(13.0).italics().color(AMBER)).wrap());
                    ui.add_space(12.0);
                    let again_btn = egui::Button::new(RichText::new(rocca::tr!("Riprova")).size(26.0).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(12.0);
                    let r = ui.add_sized([side, 64.0], again_btn);
                    crate::playtest::control("Riprova", &r, true);
                    retry = super::icons::pointing(r).clicked();
                    ui.add_space(8.0);
                    let big = egui::Button::new(RichText::new(rocca::tr!("Nuova partita")).size(26.0).strong().color(Color32::BLACK)).fill(AMBER).corner_radius(12.0);
                    let r = ui.add_sized([side, 64.0], big);
                    crate::playtest::control("Nuova partita", &r, true);
                    again = super::icons::pointing(r).clicked();
                });
            });
        });
    });
    if retry {
        super::retry(k, sim, restarted);
    } else if again {
        new_visit(k, sim, restarted);
    }
}


/// Narrow screens keep readable text and scroll, instead of scaling the kiosk UI.
fn compact_page(ctx: &egui::Context, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    compact_page_footer(ctx, id, content, |_| {});
}

/// `compact_page` with a `footer` pinned under the scrolling content.
pub(super) fn compact_page_footer(ctx: &egui::Context, id: &str, content: impl FnOnce(&mut egui::Ui), footer: impl FnOnce(&mut egui::Ui)) {
    backdrop(ctx, "mobile_backdrop", 200);
    let rect = ctx.viewport_rect();
    let width = (rect.width() - 40.0).min(600.0).max(120.0);
    // the footer, when there is one, takes its row out of the scroll area
    let mut footer_h = 0.0;
    let mut footer = Some(footer);
    egui::Area::new(egui::Id::new(id)).anchor(Align2::CENTER_TOP, [0.0, 8.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        panel_mobile().show(ui, |ui| {
            ui.set_width(width);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            let footer_room: f32 = ctx.memory(|m| m.data.get_temp(egui::Id::new((id, "footer_h"))).unwrap_or(0.0));
            // Areas remember their last measured size. Let the scroll viewport
            // grow again when results arrive asynchronously or the phone rotates.
            let max_h = (rect.height() - 92.0 - footer_room).max(80.0);
            ui.set_max_height(max_h + footer_room);
            egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                content(ui);
                ui.add_space(8.0);
                ui.label(RichText::new(rocca::tr!("versione {0}", super::BUILD)).size(12.0).color(GREY));
                if cfg!(target_arch = "wasm32") && ui.add(secondary(rocca::tr!("Dai un feedback"))).clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(super::ui::feedback_url()));
                }
            });
            let before = ui.cursor().top();
            if let Some(f) = footer.take() {
                ui.add_space(6.0);
                f(ui);
            }
            footer_h = ui.min_rect().bottom() - before;
        });
    });
    ctx.memory_mut(|m| m.data.insert_temp(egui::Id::new((id, "footer_h")), footer_h));
}

fn panel_mobile() -> egui::Frame {
    card(AMBER).inner_margin(12.0)
}

fn action(ui: &mut egui::Ui, label: &str) -> bool {
    let r = ui.add_sized([ui.available_width(), 46.0], primary(label));
    crate::playtest::control(label, &r, true);
    r.clicked()
}

fn compact_intro(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>) {
    let mut go = None;
    compact_page(ctx, "intro_mobile", |ui| {
        ui.heading(super::TITLE);
        ui.label(rocca::tr!("Metti in pausa per spostare le squadre e ordinare l'evacuazione dei borghi."));
        ui.add_space(6.0);
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            let window = web_sys::window().unwrap();
            if let Ok(v) = js_sys::Reflect::get(&window, &"roccaPlayerName".into()) {
                if let Some(name) = v.as_string() {
                    k.player = rocca::score::name(&name);
                    let _ = js_sys::Reflect::delete_property(&window, &"roccaPlayerName".into());
                }
            }
            let label = if k.player.is_empty() { rocca::tr!("Come ti chiami?").to_string() } else { rocca::tr!("Nome: {0}", k.player) };
            let r = ui.add_sized([ui.available_width(), 44.0], secondary(&label));
            crate::playtest::control("Nome", &r, true);
            if r.clicked() {
                if let Ok(f) = js_sys::Reflect::get(&window, &"roccaEditName".into()) {
                    if let Some(f) = f.dyn_ref::<js_sys::Function>() { let _ = f.call1(&window, &k.player.clone().into()); }
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        { ui.add(egui::TextEdit::singleline(&mut k.player).hint_text(rocca::tr!("Come ti chiami?"))); }
        if action(ui, rocca::tr!("TUTORIAL")) { go = Some(true); }
        let r = ui.add_sized([ui.available_width(), 44.0], secondary(rocca::tr!("INIZIA")));
        crate::playtest::control("Inizia", &r, true);
        if r.clicked() { go = Some(false); }
        ui.label(rocca::tr!("Tocca una squadra, poi il punto dove mandarla. Tocca un borgo per evacuare. Puoi anche trascinare le squadre."));
        ui.separator();
        ui.collapsing(rocca::tr!("Classifica di oggi"), |ui| board_rows(ui, k, &sim.case.name, None, rocca::score::BOARD_SHOWN));
        ui.label(RichText::new(rocca::tr!("Territorio immaginario. Nella realtà segui le indicazioni delle autorità.")).size(13.0));
    });
    if go.is_some() { k.player = rocca::score::name(&k.player); }
    match go {
        Some(true) => start_tutorial(k, sim, restarted),
        Some(false) => choose_scenario(k),
        None => {}
    }
}

fn compact_tutorial(ctx: &egui::Context, k: &mut Kiosk, sim: &mut Sim, restarted: &mut MessageWriter<SimRestarted>, text: &str, expect: Expect) {
    // The evacuation card carries the requested action while it is open.
    if matches!(k.selection, super::Selection::District(_)) { return; }
    let mut skip = false;
    let rect = ctx.viewport_rect();
    let landscape = rect.width() > rect.height();
    let anchor = if landscape { Align2::RIGHT_TOP } else { Align2::CENTER_BOTTOM };
    let offset = if landscape { [-8.0, 8.0] } else { [0.0, -58.0] };
    let text = text.replace("**", "").replace("__", "");
    let steps = k.script.as_ref().map_or(0, |s| s.steps.len());
    let progress = (k.step + 1) as f32 / steps.max(1) as f32;
    egui::Area::new(egui::Id::new("tutorial_mobile")).anchor(anchor, offset).order(egui::Order::Foreground).show(ctx, |ui| {
        panel_mobile().show(ui, |ui| {
            ui.set_width(if landscape { k.side_w - 40.0 } else { (rect.width() - 40.0).min(600.0) });
            let buttons = |ui: &mut egui::Ui, skip: &mut bool, k: &mut Kiosk| {
                ui.horizontal(|ui| {
                    // «Avanti» takes the room left: the easiest thing to hit
                    let r = ui.add(secondary(rocca::tr!("Salta")).min_size(egui::vec2(60.0, 44.0)));
                    crate::playtest::control("Salta il tutorial", &r, true);
                    *skip = r.clicked();
                    if expect == Expect::Next {
                        let r = ui.add_sized([ui.available_width(), 44.0], primary(rocca::tr!("Avanti")));
                        crate::playtest::control("Avanti", &r, true);
                        if r.clicked() { k.done(Expect::Next); }
                    }
                });
            };
            if landscape {
                // A column the height of the screen: where we are on top, the
                // text filling the middle, the buttons pinned under it.
                ui.horizontal(|ui| {
                    ui.label(RichText::new(rocca::tr!("TUTORIAL")).size(13.0).strong().color(AMBER));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{}/{}", k.step + 1, steps)).size(13.0).color(GREY));
                    });
                });
                ui.add(egui::ProgressBar::new(progress).desired_height(4.0));
                ui.add_space(6.0);
                let text_h = (rect.height() - 16.0 - 24.0 - 38.0 - 56.0).max(60.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(text_h).min_scrolled_height(text_h).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&text).size(19.0).color(Color32::WHITE)).wrap());
                });
                ui.add_space(6.0);
                buttons(ui, &mut skip, k);
            } else {
                egui::ScrollArea::vertical().max_height((rect.height() * 0.20).clamp(42.0, 120.0)).show(ui, |ui| {
                    ui.label(RichText::new(&text).size(17.0));
                });
                ui.add(egui::ProgressBar::new(progress).desired_height(4.0));
                ui.add_space(4.0);
                buttons(ui, &mut skip, k);
            }
        });
    });
    if skip { start_game(k, sim, restarted); }
}
