//! The plants' fire, plant by plant (user, 2026-11-01): `KIOSK_PIANTE=1`
//! opens a scene of its own instead of the game. Every variety stands in a
//! row; along the row the same plant catches a few seconds after its left
//! neighbour, so one picture shows the stages of its fire side by side, and
//! the clock scrubs them all. The plants are drawn exactly as on the island
//! (the same meshes, material, instancing and fire grid): only the grid's
//! arrival times are set by hand.
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

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::TransmittedShadowReceiver;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::materials::{self, FoliageMaterial};
use crate::veg_instancing::{PlantInstance, VegInstances};
use crate::vegetation::{plant_lag, variety_mesh, Tier, PLANT_FLAMING_S};

/// Plants per row: the stages shown side by side.
const COLUMNS: usize = 8;
/// Metres between rows; between columns, a plant and a half of the row's.
const ROW_M: f32 = 22.0;

fn col_m(size: Vec3) -> f32 {
    // whole cells, or the feet snapped to cell centres space unevenly
    ((size.x * 1.5).max(4.0) / CELL_M).ceil() * CELL_M
}
/// The fire grid's cell: each plant stands at the centre of its own cell, so
/// it reads that cell's arrival alone.
const CELL_M: f32 = 2.0;
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
    /// Each row's entities (plants and remains).
    entities: Vec<[Entity; 2]>,
    row_drawn: Option<usize>,
    rows: Vec<(&'static str, &'static str, Vec3)>,
    grid: Handle<Image>,
    /// The plants', the remains'.
    materials: [Handle<FoliageMaterial>; 2],
    /// Feet of the plants, row by row, in the scenario frame (x east, y north).
    feet: Vec<[Vec2; COLUMNS]>,
    grid_size: (usize, usize),
    /// The camera's tone mapper (the game's is TonyMcMapface), to compare.
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

    /// The grid: every plant's cell holds the arrival that makes it catch at
    /// its column's instant, its own lag taken off (`vegetation::plant_lag`).
    fn grid_bytes(&self) -> Vec<u8> {
        let (cols, rows) = self.grid_size;
        let mut data = vec![0u8; cols * rows * 4];
        for px in data.chunks_mut(4) {
            px.copy_from_slice(&[0xff, 0xff, 0xff, 0]);
        }
        for feet in &self.feet {
            for (k, foot) in feet.iter().enumerate() {
                let lag = plant_lag(scenario::Pos { x: foot.x, y: foot.y });
                let t = (self.ignition(k) - lag).round().max(0.0) as u32;
                let col = (foot.x / CELL_M) as usize;
                let row = ((rows as f32 * CELL_M - foot.y) / CELL_M) as usize;
                let i = (row * cols + col) * 4;
                data[i..i + 4].copy_from_slice(&[t as u8, (t >> 8) as u8, (t >> 16) as u8, 0]);
            }
        }
        data
    }

    /// Bevy position of a plant (column `k` of row `r`), on the ground.
    fn foot(&self, r: usize, k: usize) -> Vec3 {
        let f = self.feet[r][k];
        Vec3::new(f.x, 0.0, -f.y)
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
    .add_plugins(crate::veg_instancing::VegInstancingPlugin)
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
    mut foliage: ResMut<Assets<FoliageMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let rows = rows();
    let widest = rows.iter().map(|r| col_m(r.2)).fold(0.0, f32::max);
    let width = (COLUMNS + 1) as f32 * widest;
    let depth = rows.len() as f32 * ROW_M + ROW_M;
    let grid_size = ((width / CELL_M).ceil() as usize, (depth / CELL_M).ceil() as usize);
    // feet at cell centres, the first row to the north
    let centre = |m: f32| ((m / CELL_M).floor() + 0.5) * CELL_M;
    let feet: Vec<[Vec2; COLUMNS]> = (0..rows.len())
        .map(|r| {
            let step = col_m(rows[r].2);
            std::array::from_fn(|k| Vec2::new(centre(step * (k + 1) as f32), centre(depth - ROW_M - r as f32 * ROW_M)))
        })
        .collect();

    let wanted_row = std::env::var("KIOSK_PIANTE_ROW").ok();
    let row = wanted_row
        .and_then(|w| rows.iter().position(|(species, name, _)| *name == w || *species == w))
        .unwrap_or(0);
    let mut lab = Lab {
        // from a plant about to catch to one burnt out
        clock: env_f32("KIOSK_PIANTE_T").unwrap_or(270.0),
        playing: false,
        speed: 20.0,
        step: env_f32("KIOSK_PIANTE_STEP").unwrap_or(40.0).max(1.0),
        step_drawn: 0.0,
        row,
        alone: true,
        entities: Vec::new(),
        row_drawn: None,
        rows,
        grid: Handle::default(),
        materials: [Handle::default(), Handle::default()],
        feet,
        grid_size,
        snap: std::env::var("KIOSK_PIANTE_SNAP").ok().map(|f| (f, 90)),
        snap_target: Handle::default(),
        tonemap: std::env::var("KIOSK_PIANTE_TONEMAP")
            .ok()
            .and_then(|w| TONEMAPS.iter().position(|(name, _)| name.eq_ignore_ascii_case(&w)))
            .unwrap_or(0),
    };
    lab.step_drawn = lab.step;
    let grid = images.add(Image::new(
        Extent3d { width: grid_size.0 as u32, height: grid_size.1 as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        lab.grid_bytes(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    ));
    let atlas = images.add(materials::foliage_atlas());
    let frame = Vec4::new(CELL_M, grid_size.1 as f32 * CELL_M, grid_size.0 as f32, grid_size.1 as f32);
    let mut plants = FoliageMaterial::new(grid.clone(), atlas.clone(), frame, false);
    plants.layer.y = 1.0;
    let mut remains = FoliageMaterial::new(grid.clone(), atlas, frame, true);
    remains.layer.y = 1.0;
    lab.materials = [foliage.add(plants), foliage.add(remains)];
    lab.grid = grid;

    for (r, &(_, name, size)) in lab.rows.iter().enumerate() {
        let list: Vec<PlantInstance> = (0..COLUMNS)
            .map(|k| PlantInstance {
                pos: lab.foot(r, k).to_array(),
                // all the same way round: the stages compare
                yaw: 0.6,
                scale: size.to_array(),
                // the atlas' own colours (multiplier 1, stored halved)
                tint: [128; 4],
            })
            .collect();
        let list = Arc::new(list);
        let (lo, hi) = (lab.foot(r, 0) - size, lab.foot(r, COLUMNS - 1) + Vec3::new(size.x, size.y * 1.3, size.z));
        let row_entities = [(Tier::Full, &lab.materials[0]), (Tier::RemainsNear, &lab.materials[1])].map(|(tier, material)| {
            commands.spawn((
                Mesh3d(meshes.add(variety_mesh(name, tier))),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                VegInstances(list.clone()),
                Aabb::from_min_max(lo, hi),
                bevy::camera::visibility::NoAutoAabb,
                bevy::render::batching::NoAutomaticBatching,
                TransmittedShadowReceiver,
                if r == lab.row { Visibility::Inherited } else { Visibility::Hidden },
            ))
            .id()
        });
        lab.entities.push(row_entities);
    }

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
    if lab.snap.is_some() {
        let image = Image::new_target_texture(1600, 1000, TextureFormat::Rgba8UnormSrgb, None);
        lab.snap_target = images.add(image);
        cam.insert(bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
            handle: lab.snap_target.clone(),
            scale_factor: 1.0,
        }));
    }
    commands.insert_resource(lab);
}

/// The clock, the grid when the spacing changed, and the materials.
fn tick(time: Res<Time>, mut lab: ResMut<Lab>, mut images: ResMut<Assets<Image>>, mut foliage: ResMut<Assets<FoliageMaterial>>) {
    if lab.playing {
        lab.clock += time.delta_secs() * lab.speed;
        if lab.clock > lab.max_clock() {
            lab.clock = -10.0;
        }
    }
    if lab.step != lab.step_drawn {
        lab.step_drawn = lab.step;
        let data = lab.grid_bytes();
        if let Some(mut image) = images.get_mut(&lab.grid) {
            image.data = Some(data);
        }
    }
    let want = Vec4::new(0.0, FIRST_S + lab.clock, PLANT_FLAMING_S, 1.0);
    for handle in &lab.materials {
        if foliage.get(handle).is_some_and(|m| m.fire != want) {
            if let Some(mut m) = foliage.get_mut(handle) {
                m.fire = want;
            }
        }
    }
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
    mut visibility: Query<&mut Visibility>,
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
        cam.distance = ((COLUMNS - 1) as f32 * col_m(lab.rows[r].2) * 0.95 + lab.rows[r].2.y)
            * env_f32("KIOSK_PIANTE_DIST").unwrap_or(1.0);
        cam.yaw = 0.0;
        cam.pitch = 0.22;
    }
    for (r, row) in lab.entities.iter().enumerate() {
        let want = if r == lab.row || !lab.alone { Visibility::Inherited } else { Visibility::Hidden };
        for &e in row {
            if let Ok(mut v) = visibility.get_mut(e) {
                v.set_if_neq(want);
            }
        }
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
            if ui.button(if lab.playing { "Pausa" } else { "Avanti" }).clicked() {
                lab.playing = !lab.playing;
            }
            if ui.button("Da capo").clicked() {
                lab.clock = -10.0;
            }
        });
        ui.add(egui::Slider::new(&mut lab.clock, -10.0..=max).text("s dalla prima colonna"));
        ui.add(egui::Slider::new(&mut lab.speed, 1.0..=120.0).logarithmic(true).text("velocità (s al secondo)"));
        ui.add(egui::Slider::new(&mut lab.step, 2.0..=60.0).text("s tra una colonna e l'altra"));
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
    if *done > 20 {
        exit.write(AppExit::Success);
    }
}
