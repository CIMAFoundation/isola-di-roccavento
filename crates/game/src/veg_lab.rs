//! The plants' fire, plant by plant (user, 2026-11-01): `KIOSK_PIANTE=1`
//! opens a scene of its own instead of the game. Every variety stands in a
//! row; along the row the same plant catches a few seconds after its left
//! neighbour, so one picture shows the stages of its fire side by side, and
//! the clock scrubs them all. The plants are drawn exactly as on the island
//! (the same meshes, atlas, pipeline and shaders, `vegetation`): only each
//! plant's instant of ignition is set by hand.
//!
//! Controls: drag to turn, right button (or Shift) to pan, wheel to zoom;
//! the panel sets the clock, plays it, picks the row to look at and the
//! spacing of the columns.
//!
//! `KIOSK_PIANTE_SNAP=<file.png>` saves one picture, with the window hidden,
//! and quits; `KIOSK_PIANTE_T` (seconds after the first column caught),
//! `KIOSK_PIANTE_ROW` (a variety or a species), `KIOSK_PIANTE_STEP` and
//! `KIOSK_PIANTE_DIST` (the camera distance, times the usual) choose it;
//! `KIOSK_PIANTE_TONEMAP` the tone mapper (`TONEMAPS`).

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::sync_world::SyncToRenderWorld;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::materials;
use crate::vegetation::{LabRow, PlantsDrawable, Vegetation, PLANT_FLAMING_S};

/// Plants per row (user, 2026-11-04: one tree, its fire run by the clock;
/// it was eight, the stages side by side).
const COLUMNS: usize = 1;
/// Metres between rows; between columns, a plant and a half of the row's.
const ROW_M: f32 = 22.0;

fn col_m(size: Vec3) -> f32 {
    (size.x * 1.5).max(4.0)
}
/// When the first column catches, in simulated seconds.
const FIRST_S: f32 = 100.0;

/// Whether the lab was asked for instead of the game.
pub fn wanted() -> bool {
    std::env::var_os("KIOSK_PIANTE").is_some() || std::env::var_os("KIOSK_PIANTE_SNAP").is_some()
}

/// Every variety, species by species, and how big it stands (width, height):
/// the middle of the sizes `vegetation.rs` gives its group.
fn rows() -> Vec<(&'static str, &'static str, Vec3)> {
    crate::models::SPECIES
        .iter()
        .flat_map(|&species| {
            let size = match species {
                "pine" | "aleppo" | "cypress" => Vec3::new(18.0, 16.0, 18.0),
                "holm" | "cork" | "carob" | "olive" => Vec3::new(11.5, 10.5, 11.5),
                _ => Vec3::new(3.6, 2.9, 3.6),
            };
            crate::models::variety_names(species).iter().map(move |&name| (species, name, size))
        })
        .collect()
}

#[derive(Resource)]
struct Lab {
    /// Simulated seconds after the first column caught.
    clock: f32,
    playing: bool,
    /// Simulated seconds per real second while playing.
    speed: f32,
    /// Seconds between one column's ignition and the next.
    step: f32,
    step_drawn: f32,
    /// The row the camera looks at, and whether the others are hidden.
    row: usize,
    alone: bool,
    row_drawn: Option<usize>,
    /// The rows in the plants' scene, as `(row, alone)` when it was built.
    scene_drawn: Option<(usize, bool)>,
    rows: Vec<(&'static str, &'static str, Vec3)>,
    /// Feet of the plants, row by row (Bevy frame, on the ground).
    feet: Vec<[Vec3; COLUMNS]>,
    /// The camera's tone mapper (the game's is AcesFitted), to compare.
    tonemap: usize,
    /// Frames left before the picture (`KIOSK_PIANTE_SNAP`), and the image
    /// the camera draws into meanwhile (a hidden window's screenshot comes
    /// out black, as in `kiosk::snap`; the panel and labels are left out).
    snap: Option<(String, u32)>,
    snap_target: Handle<Image>,
}

impl Lab {
    /// When column `k` catches.
    fn ignition(&self, k: usize) -> f32 {
        FIRST_S + k as f32 * self.step
    }

    /// Bevy position of a plant (column `k` of row `r`), on the ground.
    fn foot(&self, r: usize, k: usize) -> Vec3 {
        self.feet[r][k]
    }

    /// The rows the plants' scene holds: the one on show, or all of them.
    fn shown(&self) -> Vec<usize> {
        if self.alone { vec![self.row] } else { (0..self.rows.len()).collect() }
    }

    /// The plants of the rows on show, the instants they catch set.
    fn vegetation(&self) -> Vegetation {
        let shown = self.shown();
        let rows: Vec<LabRow> = shown
            .iter()
            .map(|&r| LabRow {
                name: self.rows[r].1,
                feet: self.feet[r].to_vec(),
                size: self.rows[r].2,
                // all the same way round: the stages compare
                yaw: 0.6,
            })
            .collect();
        let mut veg = Vegetation::lab(&rows);
        self.ignite(&mut veg, shown.len());
        veg
    }

    fn ignite(&self, veg: &mut Vegetation, rows: usize) {
        let at: Vec<f32> = (0..COLUMNS).map(|k| self.ignition(k)).collect();
        for r in 0..rows {
            veg.set_lab_ignitions(r, &at);
        }
    }
}

/// The tone mappers to compare, the game's first (`main.rs`).
const TONEMAPS: [(&str, Tonemapping); 7] = [
    ("AcesFitted", Tonemapping::AcesFitted),
    ("TonyMcMapface", Tonemapping::TonyMcMapface),
    ("AgX", Tonemapping::AgX),
    ("BlenderFilmic", Tonemapping::BlenderFilmic),
    ("SomewhatBoring", Tonemapping::SomewhatBoringDisplayTransform),
    ("Reinhard", Tonemapping::Reinhard),
    ("ReinhardLuminance", Tonemapping::ReinhardLuminance),
];

#[derive(Component)]
struct LabCamera {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

pub fn run() -> anyhow::Result<()> {
    let snap = std::env::var("KIOSK_PIANTE_SNAP").ok();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Isola di Roccavento: piante in fiamme".into(),
            resolution: WindowResolution::new(1600, 1000),
            present_mode: PresentMode::AutoVsync,
            // a picture is taken without showing anything (as `KIOSK_SNAP`)
            visible: snap.is_none(),
            focused: snap.is_none(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    .add_plugins(crate::shader_lib::ShaderLibPlugin)
    .add_plugins(materials::MaterialsPlugin)
    .add_plugins(crate::vegetation::PlantRenderPlugin)
    .add_plugins(crate::hi_trees::HiTreesPlugin)
    // the flames on the burning plants, as on the island (`fire_view`)
    .add_plugins(crate::fire_view::LabTorchesPlugin)
    .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
    .insert_resource(GlobalAmbientLight { brightness: 900.0, ..default() })
    .add_systems(Startup, setup)
    .add_systems(Update, (tick, camera, snap_when_ready))
    .add_systems(EguiPrimaryContextPass, panel);
    app.run();
    Ok(())
}

fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let rows = rows();
    let widest = rows.iter().map(|r| col_m(r.2)).fold(0.0, f32::max);
    let width = (COLUMNS + 1) as f32 * widest;
    let depth = rows.len() as f32 * ROW_M + ROW_M;
    // the first row to the north (Bevy -z)
    let feet: Vec<[Vec3; COLUMNS]> = (0..rows.len())
        .map(|r| {
            let step = col_m(rows[r].2);
            std::array::from_fn(|k| Vec3::new(step * (k + 1) as f32, 0.0, -(depth - ROW_M - r as f32 * ROW_M)))
        })
        .collect();

    let wanted_row = std::env::var("KIOSK_PIANTE_ROW").ok();
    let row = wanted_row
        .and_then(|w| rows.iter().position(|(species, name, _)| *name == w || *species == w))
        .unwrap_or(0);
    let mut lab = Lab {
        // from a plant about to catch to one burnt out
        clock: env_f32("KIOSK_PIANTE_T").unwrap_or(-10.0),
        playing: std::env::var_os("KIOSK_PIANTE_SNAP").is_none(),
        speed: 20.0,
        step: env_f32("KIOSK_PIANTE_STEP").unwrap_or(40.0).max(1.0),
        step_drawn: 0.0,
        row,
        alone: true,
        row_drawn: None,
        scene_drawn: Some((row, true)),
        rows,
        feet,
        snap: std::env::var("KIOSK_PIANTE_SNAP").ok().map(|f| (f, 90)),
        snap_target: Handle::default(),
        tonemap: std::env::var("KIOSK_PIANTE_TONEMAP")
            .ok()
            .and_then(|w| TONEMAPS.iter().position(|(name, _)| name.eq_ignore_ascii_case(&w)))
            .unwrap_or(0),
    };
    lab.step_drawn = lab.step;
    commands.insert_resource(lab.vegetation());
    // the one entity the plants' draws hang on (`vegetation::spawn`)
    commands.spawn((PlantsDrawable, Transform::default(), SyncToRenderWorld::default()));

    // bare dry ground, as under the island's macchia
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(width * 3.0, depth * 3.0))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.47, 0.42, 0.30),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(width * 0.5, 0.0, -depth * 0.5),
    ));
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: true, illuminance: 9000.0, ..default() },
        Transform::from_xyz(0.0, 0.0, 0.0).looking_to(Vec3::new(-0.45, -0.75, -0.5), Vec3::Y),
    ));
    let mut cam = commands.spawn((
        Camera3d::default(),
        bevy::camera::Hdr,
        TONEMAPS[lab.tonemap].1,
        Bloom { intensity: 0.20, ..Bloom::NATURAL },
        Transform::default(),
        LabCamera { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.22, distance: 120.0 },
    ));
    let mut gallery = None;
    // `KIOSK_PIANTE_HI=1`: the full-resolution models (`hi_trees`), every
    // `.glb` under `BEVY_ASSET_ROOT/assets/`, in a row of their own south of
    // the plants, as modelled; the camera starts on them (2026-11-04)
    if std::env::var("KIOSK_PIANTE_HI").is_ok_and(|v| v == "1") {
        let root = std::env::var("BEVY_ASSET_ROOT").map(std::path::PathBuf::from).unwrap_or_default().join("assets");
        let mut files: Vec<String> = std::fs::read_dir(&root)
            .map(|d| d.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".glb")).collect())
            .unwrap_or_default();
        files.sort();
        let step = 12.0;
        let z = 40.0;
        info!("lab: {} full models in {}: {:?}", files.len(), root.display(), files);
        let spots: Vec<crate::hi_trees::HiSpot> = files
            .iter()
            .enumerate()
            .map(|(k, f)| crate::hi_trees::HiSpot { path: f.clone(), foot: Vec3::new(k as f32 * step, 0.0, z), yaw: 0.0, height: None })
            .collect();
        if !spots.is_empty() {
            let mid = (spots.len() - 1) as f32 * step * 0.5;
            cam.insert(LabCamera { focus: Vec3::new(mid, 3.0, z), yaw: 0.0, pitch: 0.18, distance: (mid * 1.6).max(20.0) });
            lab.row_drawn = Some(lab.row);
            gallery = Some(crate::hi_trees::HiTrees { spots });
        }
    }
    if lab.snap.is_some() {
        let image = Image::new_target_texture(1600, 1000, TextureFormat::Rgba8UnormSrgb, None);
        lab.snap_target = images.add(image);
        cam.insert(bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
            handle: lab.snap_target.clone(),
            scale_factor: 1.0,
        }));
    }
    if let Some(g) = gallery {
        commands.insert_resource(g);
    }
    commands.insert_resource(lab);
}

/// The clock, the plants' scene when the rows on show changed, the instants
/// of ignition when the spacing did.
fn tick(time: Res<Time>, mut lab: ResMut<Lab>, mut veg: ResMut<Vegetation>) {
    if lab.playing {
        lab.clock += time.delta_secs() * lab.speed;
        if lab.clock > lab.max_clock() {
            lab.clock = -10.0;
        }
    }
    let shown = (lab.row, lab.alone);
    if lab.scene_drawn != Some(shown) {
        lab.scene_drawn = Some(shown);
        lab.step_drawn = lab.step;
        *veg = lab.vegetation();
    } else if lab.step != lab.step_drawn {
        lab.step_drawn = lab.step;
        let rows = lab.shown().len();
        lab.ignite(&mut veg, rows);
    }
    veg.set_lab_clock(FIRST_S + lab.clock, time.elapsed_secs_wrapped());
}

impl Lab {
    /// Long enough for the last column to burn out, and a while after.
    fn max_clock(&self) -> f32 {
        (COLUMNS - 1) as f32 * self.step + PLANT_FLAMING_S + 120.0
    }
}

/// Orbit about the chosen row: drag to turn, right button or Shift to pan,
/// wheel to zoom.
#[allow(clippy::too_many_arguments)]
fn camera(
    mut lab: ResMut<Lab>,
    mut cams: Query<(&mut LabCamera, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut contexts: EguiContexts,
) {
    let Ok((mut cam, mut tf)) = cams.single_mut() else { return };
    if lab.row_drawn != Some(lab.row) {
        lab.row_drawn = Some(lab.row);
        let r = lab.row;
        // the row seen from the front, its eight plants across the picture
        cam.focus = (lab.foot(r, 0) + lab.foot(r, COLUMNS - 1)) * 0.5 + Vec3::Y * lab.rows[r].2.y * 0.4;
        cam.distance = ((COLUMNS - 1) as f32 * col_m(lab.rows[r].2) * 0.95 + lab.rows[r].2.y * 1.8)
            * env_f32("KIOSK_PIANTE_DIST").unwrap_or(1.0);
        cam.yaw = 0.0;
        cam.pitch = 0.22;
    }
    let over_ui = contexts.ctx_mut().is_ok_and(|c| c.is_pointer_over_egui() || c.egui_wants_pointer_input());
    if !over_ui {
        let pan = buttons.pressed(MouseButton::Right) || (buttons.pressed(MouseButton::Left) && keys.pressed(KeyCode::ShiftLeft));
        if pan {
            let right = tf.right().as_vec3();
            let forward = Vec3::new(tf.forward().x, 0.0, tf.forward().z).normalize_or_zero();
            let k = cam.distance * 0.0015;
            cam.focus += (-right * motion.delta.x + forward * motion.delta.y) * k;
        } else if buttons.pressed(MouseButton::Left) {
            cam.yaw -= motion.delta.x * 0.005;
            cam.pitch = (cam.pitch + motion.delta.y * 0.005).clamp(0.02, 1.5);
        }
        if scroll.delta.y != 0.0 {
            cam.distance = (cam.distance * (1.0 - scroll.delta.y * 0.1)).clamp(4.0, 900.0);
        }
    }
    let dir = Vec3::new(cam.yaw.sin() * cam.pitch.cos(), cam.pitch.sin(), cam.yaw.cos() * cam.pitch.cos());
    *tf = Transform::from_translation(cam.focus + dir * cam.distance).looking_at(cam.focus, Vec3::Y);
}

fn panel(mut contexts: EguiContexts, mut lab: ResMut<Lab>, mut cams: Query<(&Camera, &GlobalTransform, &mut Tonemapping)>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let max = lab.max_clock();
    egui::Window::new("Piante in fiamme").default_pos([12.0, 12.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button(if lab.playing { "Pausa" } else { "Play" }).clicked() {
                lab.playing = !lab.playing;
            }
            if ui.button("Da capo").clicked() {
                lab.clock = -10.0;
            }
        });
        ui.add(egui::Slider::new(&mut lab.clock, -10.0..=max).text("s dall'innesco"));
        ui.add(egui::Slider::new(&mut lab.speed, 1.0..=120.0).logarithmic(true).text("velocità (s al secondo)"));
        if COLUMNS > 1 {
            ui.add(egui::Slider::new(&mut lab.step, 2.0..=60.0).text("s tra una colonna e l'altra"));
        }
        ui.label(format!("Una pianta brucia per {PLANT_FLAMING_S:.0} s, poi resta carbonizzata."));
        let current = lab.rows[lab.row].1;
        let mut row = lab.row;
        egui::ComboBox::from_label("Varietà").selected_text(current).show_ui(ui, |ui| {
            for (i, (_, name, _)) in lab.rows.iter().enumerate() {
                ui.selectable_value(&mut row, i, *name);
            }
        });
        lab.row = row;
        ui.checkbox(&mut lab.alone, "Solo questa varietà");
        let mut tm = lab.tonemap;
        egui::ComboBox::from_label("Tone mapper").selected_text(TONEMAPS[tm].0).show_ui(ui, |ui| {
            for (i, (name, _)) in TONEMAPS.iter().enumerate() {
                ui.selectable_value(&mut tm, i, *name);
            }
        });
        lab.tonemap = tm;
    });
    // over each plant of the row on show: how long it has been alight
    let Ok((camera, cam_tf, mut tonemapping)) = cams.single_mut() else { return };
    tonemapping.set_if_neq(TONEMAPS[lab.tonemap].1);
    let painter = ctx.layer_painter(egui::LayerId::background());
    let r = lab.row;
    let height = lab.rows[r].2.y;
    for k in 0..COLUMNS {
        let age = FIRST_S + lab.clock - lab.ignition(k);
        let text = if age < 0.0 { format!("tra {:.0} s", -age) } else { format!("{age:.0} s") };
        let p = lab.foot(r, k) + Vec3::Y * (height * 1.15);
        if let Ok(v) = camera.world_to_viewport(cam_tf, p) {
            let colour = if age < 0.0 {
                egui::Color32::from_rgb(60, 140, 60)
            } else if age < PLANT_FLAMING_S {
                egui::Color32::from_rgb(230, 120, 30)
            } else {
                egui::Color32::from_gray(40)
            };
            painter.text(egui::pos2(v.x, v.y), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(18.0), colour);
        }
    }
    if let Ok(v) = camera.world_to_viewport(cam_tf, lab.foot(r, 0) - Vec3::X * col_m(lab.rows[r].2) * 0.6) {
        painter.text(egui::pos2(v.x, v.y), egui::Align2::RIGHT_CENTER, lab.rows[r].1, egui::FontId::proportional(18.0), egui::Color32::WHITE);
    }
}

/// `KIOSK_PIANTE_SNAP`: one picture once the shaders are ready, then quit.
fn snap_when_ready(mut commands: Commands, mut lab: ResMut<Lab>, mut exit: MessageWriter<AppExit>, mut done: Local<u32>) {
    let target = lab.snap_target.clone();
    let Some((file, frames)) = &mut lab.snap else { return };
    if *frames > 0 {
        *frames -= 1;
        if *frames == 0 {
            commands.spawn(Screenshot::image(target)).observe(save_to_disk(file.clone()));
        }
        return;
    }
    // a few frames for the picture to be written
    *done += 1;
    // (once written: a pipeline still compiling holds the picture back)
    if (*done > 20 && std::path::Path::new(file.as_str()).exists()) || *done > 1200 {
        exit.write(AppExit::Success);
    }
}
