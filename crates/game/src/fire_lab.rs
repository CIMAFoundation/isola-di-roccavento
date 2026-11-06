//! The fire's volumes on flat ground (user, 2026-11-05: «fai un lab che
//! mostri un incendio piatto con fiamme e fumo, telecamera ruotabile, le
//! diverse opzioni per modificare i parametri del fuoco e fumo
//! volumetrico, fps … metti anche vento e tempo … e playtime»).
//!
//! `KIOSK_FUOCO=1` opens it instead of the game: 4 km of flat ground, a
//! fire of a chosen shape run by its own clock, drawn by the same volumes
//! as the island (`fire_volumetric`), and a panel with every knob: the
//! clock (play, pause, speed, scrub), the fire (shape, speed, intensity),
//! the wind (speed, direction: a shift bends the plume), the sun, the
//! smoke, the flames, the quality (reduction, steps, history) and the
//! frame time.
//!
//! Controls: drag to turn, right button (or Shift) to pan, wheel to zoom.
//!
//! `KIOSK_FUOCO_SNAP=<file.png>` saves one picture with the window hidden
//! (with `sequenza=<n>` in `KIOSK_FUOCO_SET`, n frames in a row)
//! and quits; `KIOSK_FUOCO_T` (sim seconds), `KIOSK_FUOCO_DIST`,
//! `KIOSK_FUOCO_PITCH`, `KIOSK_FUOCO_YAW` (degrees), `KIOSK_FUOCO_FORMA`
//! (`linea`, `cerchio`, `chiazza`), `KIOSK_VOL_DIV` choose it.
//!
//! The cost (2026-11-05): the picture's log line gives the median frame
//! time and the GPU time of the volumes' passes (`volumi`,
//! `bench::PassStamps`; `KIOSK_VOL_STAMP=marcia|storia|composizione` reads
//! up to the end of that pass). With no window to present to, the frame
//! time is the CPU's: only `volumi` says what the volumes cost, and only
//! with `KIOSK_FUOCO_SIZE=<w>x<h>` several times the pixels (4800x3000)
//! is it steady. `conta=1|2|3` in `KIOSK_FUOCO_SET` draws what the
//! march's steps were spent on (`VolumeSettings::count`); `solo=1` the
//! volumes over black and `marca=1|2|3` marker colours for each stage
//! (`VolumeSettings::mark`): what shakes is found there, not in the
//! picture. `scarto=1` draws how far from itself the history finds each
//! pixel (`VolumeSettings::offset`): black with the camera still. The lab
//! stands at the world's origin: what goes wrong kilometres from it (the
//! island) does not show here.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::fire_view::FLAMING_S;
use crate::fire_volumetric::{rebuild, FireVolumeCamera, PlumeState, VolumeSettings};

/// Fire cells a side, metres a cell: the island's grid.
const CELLS: usize = 200;
const CELL_M: f32 = 20.0;
const SIDE_M: f32 = CELLS as f32 * CELL_M;

/// Whether the lab was asked for instead of the game.
pub fn wanted() -> bool {
    std::env::var_os("KIOSK_FUOCO").is_some() || std::env::var_os("KIOSK_FUOCO_SNAP").is_some()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    /// A line across the wind, lit at once, running downwind.
    Line,
    /// A point, growing into an ellipse downwind.
    Circle,
    /// A disc that burns on and on, never going out: for tuning.
    Patch,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Line => "fronte lineare",
            Shape::Circle => "da un punto",
            Shape::Patch => "chiazza fissa",
        }
    }
}

#[derive(Resource)]
struct Lab {
    /// Sim seconds since ignition, whether it runs, sim seconds a second.
    clock: f32,
    playing: bool,
    speed: f32,
    shape: Shape,
    /// Head-fire spread (m/min), fireline intensity (kW/m), the patch's
    /// radius and the line's length (m).
    spread_m_min: f32,
    fli: f32,
    radius_m: f32,
    line_m: f32,
    /// Wind (km/h) and the bearing it blows from (degrees, 0 north).
    wind_kmh: f32,
    wind_from: f32,
    /// The sun's height and bearing (degrees).
    sun_up: f32,
    sun_az: f32,
    /// Frame time, smoothed (ms).
    frame_ms: f32,
    /// Every frame's time while a picture settles (ms), for the cost.
    frame_times: Vec<f32>,
    since_rebuild: f32,
    drawn: Option<(VolumeSettings, f32, Shape, [f32; 7])>,
    snap: Option<(String, u32)>,
    /// Snapshot mode: this many frames in a row after the first
    /// (`<file>_000.png` …), for measuring what changes frame to frame.
    sequence: u32,
    snap_target: Handle<Image>,
    ground_color: Handle<Image>,
    ground_glow: Handle<Image>,
    /// Degrees of yaw a frame while a picture settles.
    spin: f32,
    /// Sim seconds a frame while a picture settles.
    play_step: f32,
    /// Degrees the smoke's wind is turned from the fire's.
    turn_deg: f32,
    /// The wind turns to a bearing at a clock (sim s).
    turn: Option<(f32, f32)>,
    show_ui: bool,
}

impl Lab {
    /// The wind the fire spread with (Bevy x, z, m/s).
    fn wind(&self) -> Vec2 {
        Self::wind_vec(self.wind_from, self.wind_kmh)
    }

    /// The wind the smoke rises in: turned by `turn_deg` (as when the
    /// scenario's wind shifts, after the fire has drawn its shape).
    fn plume_wind(&self) -> Vec2 {
        Self::wind_vec(self.wind_from + self.turn_deg, self.wind_kmh)
    }

    fn wind_vec(from_deg: f32, kmh: f32) -> Vec2 {
        // blows *from* the bearing: toward the opposite (Bevy -z is north)
        let from = from_deg.to_radians();
        Vec2::new(-from.sin(), from.cos()) * kmh / 3.6
    }

    /// When the front reached cell `i` (sim s) and its intensity, or
    /// `None` if it has not and will not.
    fn burn(&self, i: usize) -> Option<(f32, f32)> {
        let (r, c) = (i / CELLS, i % CELLS);
        let centre = Vec2::splat(SIDE_M * 0.5);
        // world metres, +y north
        let p = Vec2::new((c as f32 + 0.5) * CELL_M, SIDE_M - (r as f32 + 0.5) * CELL_M) - centre;
        let w = self.wind();
        let down = if w.length() > 0.1 { Vec2::new(w.x, -w.y).normalize() } else { Vec2::Y };
        let v = (self.spread_m_min / 60.0).max(0.01);
        match self.shape {
            Shape::Patch => (p.length() < self.radius_m).then_some((self.clock - FLAMING_S * 0.3, self.fli)),
            Shape::Line => {
                let along = p.dot(down);
                let across = p.perp_dot(down).abs();
                if across > self.line_m * 0.5 {
                    return None;
                }
                // the head runs downwind, the back creeps upwind
                let (t, k) = if along >= 0.0 { (along / v, 1.0) } else { (-along / (v * 0.12), 0.25) };
                Some((t, self.fli * k))
            }
            Shape::Circle => {
                let d = p.length();
                let cos = if d > 1.0 { p.dot(down) / d } else { 1.0 };
                // an ellipse: the head at full speed, the flanks a third,
                // the back an eighth
                let rate = 0.12 + 0.88 * (0.5 + 0.5 * cos).powf(2.0);
                Some((d / (v * rate), self.fli * (0.25 + 0.75 * rate)))
            }
        }
    }

    fn key(&self) -> [f32; 7] {
        [self.spread_m_min, self.fli, self.radius_m, self.line_m, self.wind_kmh, self.wind_from, self.turn_deg]
    }
}

pub fn run() -> anyhow::Result<()> {
    let snap = std::env::var("KIOSK_FUOCO_SNAP").ok();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Isola di Roccavento: fuoco e fumo volumetrici".into(),
            resolution: WindowResolution::new(1600, 1000),
            present_mode: PresentMode::AutoNoVsync,
            // a picture is taken without showing anything (as `KIOSK_SNAP`)
            visible: snap.is_none(),
            focused: snap.is_none(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    .add_plugins(crate::shader_lib::ShaderLibPlugin)
    .add_plugins(crate::fire_volumetric::FireVolumetricPlugin)
    .add_plugins(crate::bench::BenchPlugin)
    .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
    .insert_resource(GlobalAmbientLight { brightness: 900.0, ..default() })
    .add_systems(Startup, setup)
    .add_systems(Update, (tick, fire, camera, sun, snap_when_ready).chain())
    .add_systems(EguiPrimaryContextPass, panel);
    if snap.is_some() {
        // a picture's frames are a sixtieth of a second each, however long
        // they take: the flames and billows move on the shaders' clock, so
        // on the real one every run pictured another instant. Stepped, two
        // runs give the same picture pixel for pixel (checked, 2026-11-05)
        // and two builds' pictures and sequences can be compared
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)));
    }
    app.run();
    Ok(())
}

fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}

#[derive(Component)]
struct LabCamera {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Component)]
struct Sun;

fn ground_image(rgba: [u8; 4]) -> Image {
    let mut image = Image::new_fill(
        Extent3d { width: CELLS as u32, height: CELLS as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &rgba,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut settings: ResMut<VolumeSettings>,
) {
    if let Some(d) = env_f32("KIOSK_VOL_DIV") {
        settings.div = d as u32;
    }
    match std::env::var("KIOSK_VOL_SOLO").as_deref() {
        Ok("fiamme") => settings.show_smoke = false,
        Ok("fumo") => settings.show_flames = false,
        _ => {}
    }
    if let Some(h) = env_f32("KIOSK_VOL_STORIA") {
        settings.history = h;
    }
    let snap = std::env::var("KIOSK_FUOCO_SNAP").ok();
    let shape = match std::env::var("KIOSK_FUOCO_FORMA").as_deref() {
        Ok("cerchio") => Shape::Circle,
        Ok("chiazza") => Shape::Patch,
        _ => Shape::Line,
    };
    let ground_color = images.add(ground_image([112, 104, 70, 255]));
    let ground_glow = images.add(ground_image([0, 0, 0, 255]));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(SIDE_M, SIDE_M))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color_texture: Some(ground_color.clone()),
            emissive: if std::env::var_os("KIOSK_FUOCO_NOBRACE").is_some() { LinearRgba::BLACK } else { LinearRgba::rgb(0.35, 0.09, 0.015) },
            emissive_texture: Some(ground_glow.clone()),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(SIDE_M * 0.5, 0.0, -SIDE_M * 0.5),
    ));
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: true, illuminance: 9000.0, ..default() },
        Transform::default(),
        Sun,
    ));
    let centre = Vec3::new(SIDE_M * 0.5, 0.0, -SIDE_M * 0.5);
    let mut cam = commands.spawn((
        Camera3d::default(),
        bevy::camera::Hdr,
        Tonemapping::AcesFitted,
        Bloom { intensity: 0.20, ..Bloom::NATURAL },
        DistanceFog {
            color: Color::srgba(0.62, 0.70, 0.80, 1.0),
            falloff: FogFalloff::from_visibility(14_000.0),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { near: 0.5, ..default() }),
        Transform::default(),
        LabCamera {
            focus: centre + Vec3::new(0.0, 20.0, 0.0),
            yaw: env_f32("KIOSK_FUOCO_YAW").unwrap_or(200.0).to_radians(),
            pitch: env_f32("KIOSK_FUOCO_PITCH").unwrap_or(20.0).to_radians(),
            distance: env_f32("KIOSK_FUOCO_DIST").unwrap_or(900.0),
        },
        FireVolumeCamera,
    ));
    let cam_id = cam.id();
    let mut snap_target = Handle::default();
    if snap.is_some() {
        // `KIOSK_FUOCO_SIZE=<w>x<h>`: several times the pixels, so that the
        // frame time is the GPU's (as `KIOSK_SNAP_BENCH`, see `bench.rs`)
        let (w, h) = std::env::var("KIOSK_FUOCO_SIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
            .unwrap_or((1600, 1000));
        let image = Image::new_target_texture(w, h, TextureFormat::Rgba8UnormSrgb, None);
        snap_target = images.add(image);
        cam.insert(bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
            handle: snap_target.clone(),
            scale_factor: 1.0,
        }));
    }
    commands.insert_resource(PlumeState::new(&mut images));
    let mut lab = Lab {
        clock: env_f32("KIOSK_FUOCO_T").unwrap_or(600.0),
        playing: snap.is_none(),
        speed: 10.0,
        shape,
        spread_m_min: 12.0,
        fli: 6000.0,
        radius_m: 150.0,
        line_m: 900.0,
        wind_kmh: 18.0,
        wind_from: 300.0,
        sun_up: 38.0,
        sun_az: 150.0,
        frame_ms: 16.0,
        frame_times: Vec::new(),
        since_rebuild: f32::INFINITY,
        drawn: None,
        // frames for the history to settle before the picture
        snap: snap.map(|f| (f, 120)),
        sequence: 0,
        snap_target,
        ground_color,
        ground_glow,
        spin: 0.0,
        play_step: 0.0,
        turn_deg: 0.0,
        turn: None,
        show_ui: true,
    };
    if let Ok(spec) = std::env::var("KIOSK_FUOCO_SET") {
        apply_spec(&spec, &mut lab, &mut settings);
    }
    if settings.count > 0 {
        // the counts are read off the picture: as written
        commands.entity(cam_id).insert(Tonemapping::None).remove::<Bloom>();
    }
    commands.insert_resource(lab);
}

/// `KIOSK_FUOCO_SET`: `chiave=valore` separated by commas, for the
/// pictures without a window: every knob of the panel, and `spin`
/// (degrees of yaw a frame while the picture settles), `play` (sim s a
/// frame meanwhile), `gira=<s>:<gradi>` (the wind turns to that bearing
/// at that clock), `ui=0`.
fn apply_spec(spec: &str, lab: &mut Lab, s: &mut VolumeSettings) {
    for kv in spec.split(',').filter(|kv| !kv.is_empty()) {
        let Some((k, v)) = kv.split_once('=') else { continue };
        if k == "gira" {
            if let Some((t, d)) = v.split_once(':') {
                lab.turn = t.parse().ok().zip(d.parse().ok());
            }
            continue;
        }
        let Ok(x) = v.parse::<f32>() else {
            warn!("KIOSK_FUOCO_SET: {kv}?");
            continue;
        };
        match k {
            "t" => lab.clock = x,
            "spin" => lab.spin = x,
            "play" => lab.play_step = x,
            "sequenza" => lab.sequence = x as u32,
            "fotogrammi" => {
                if let Some((_, f)) = &mut lab.snap {
                    *f = x as u32;
                }
            }
            "ui" => lab.show_ui = x != 0.0,
            "spread" => lab.spread_m_min = x,
            "fli" => lab.fli = x,
            "raggio" => lab.radius_m = x,
            "linea" => lab.line_m = x,
            "vento" => lab.wind_kmh = x,
            "vento_da" => lab.wind_from = x,
            "gira_di" => lab.turn_deg = x,
            "sole" => lab.sun_up = x,
            "sole_da" => lab.sun_az = x,
            _ if s.set(k, x) => {}
            _ => warn!("KIOSK_FUOCO_SET: {k}?"),
        }
    }
}

fn tick(
    time: Res<Time>,
    mut lab: ResMut<Lab>,
    settings: Res<VolumeSettings>,
    mut state: ResMut<PlumeState>,
    mut last: Local<Option<std::time::Instant>>,
) {
    let mut dt = time.delta_secs();
    // what the frame took, whatever the clock says it lasted
    let now = std::time::Instant::now();
    let took = last.replace(now).map_or(dt, |t| (now - t).as_secs_f32());
    lab.frame_ms = lab.frame_ms * 0.95 + took * 1000.0 * 0.05;
    if lab.snap.is_some() {
        lab.frame_times.push(took * 1000.0);
    }
    if lab.playing {
        lab.clock = (lab.clock + dt * lab.speed).min(6.0 * 3600.0);
        state.clock += dt * settings.pace;
    } else if lab.snap.as_ref().is_some_and(|(_, f)| *f > 0) && lab.play_step > 0.0 {
        // a picture of a running fire: fixed steps, as at 60 fps
        dt = 1.0 / 60.0;
        lab.clock += lab.play_step;
        state.clock += dt * settings.pace;
    }
    if let Some((at, to)) = lab.turn {
        if lab.clock >= at {
            lab.turn_deg = to - lab.wind_from;
            lab.turn = None;
        }
    }
    lab.since_rebuild += dt;
}

/// The plume map and the ground's colours, from the lab's fire.
fn fire(mut lab: ResMut<Lab>, settings: Res<VolumeSettings>, mut state: ResMut<PlumeState>, mut images: ResMut<Assets<Image>>) {
    let now = (*settings, lab.clock, lab.shape, lab.key());
    let changed = lab.drawn.is_none_or(|(s, _, shape, key)| s != now.0 || shape != now.2 || key != now.3);
    // every tenth of a second: the smoke's clock and its sources move on
    // even while the fire's stands still
    let snapping = lab.snap.as_ref().is_some_and(|(_, f)| *f > 0);
    let due = if snapping { lab.since_rebuild >= 0.1 || lab.play_step > 0.0 } else { lab.since_rebuild >= 0.1 };
    if !changed && !due {
        return;
    }
    let dt = std::mem::replace(&mut lab.since_rebuild, 0.0);
    let dt = if snapping && lab.play_step > 0.0 { 1.0 / 60.0 } else { dt };
    lab.drawn = Some(now);
    let world = scenario::World { width_m: SIDE_M, height_m: SIDE_M, fire_rows: CELLS, fire_cols: CELLS, cellsize: CELL_M };
    let burns: Vec<Option<(f32, f32)>> = (0..CELLS * CELLS).map(|i| lab.burn(i).filter(|(t, _)| *t <= lab.clock)).collect();
    rebuild(&mut state, &mut images, &world, lab.clock, dt, lab.plume_wind(), 60.0, &settings, |i| burns[i], |_| 0.0);
    // the ground: dry grass, black where burnt, glowing in the band
    let (color, glow) = (lab.ground_color.clone(), lab.ground_glow.clone());
    if let Some(mut img) = images.get_mut(&color) {
        if let Some(data) = img.data.as_mut() {
            for (i, b) in burns.iter().enumerate() {
                let px = match b {
                    None => [112, 104, 70, 255],
                    Some((t, _)) => {
                        let since = lab.clock - t;
                        let k = (since / 120.0).clamp(0.0, 1.0);
                        let c = |a: f32, b: f32| (a + (b - a) * k) as u8;
                        [c(60.0, 26.0), c(48.0, 24.0), c(30.0, 22.0), 255]
                    }
                };
                data[i * 4..i * 4 + 4].copy_from_slice(&px);
            }
        }
    }
    if let Some(mut img) = images.get_mut(&glow) {
        if let Some(data) = img.data.as_mut() {
            for (i, b) in burns.iter().enumerate() {
                let g = b.map_or(0.0, |(t, _)| (1.0 - (lab.clock - t) / FLAMING_S).clamp(0.0, 1.0).powf(1.5));
                let v = (g * 255.0) as u8;
                data[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
}

fn sun(lab: Res<Lab>, mut suns: Query<&mut Transform, With<Sun>>) {
    let (up, az) = (lab.sun_up.to_radians(), lab.sun_az.to_radians());
    // toward the sun: bearing from north, clockwise (Bevy -z north, +x east)
    let to_sun = Vec3::new(az.sin() * up.cos(), up.sin(), -az.cos() * up.cos());
    for mut tf in &mut suns {
        *tf = Transform::default().looking_to(-to_sun, Vec3::Y);
    }
}

fn camera(
    mut cams: Query<(&mut LabCamera, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut contexts: EguiContexts,
    lab: Res<Lab>,
) {
    let Ok((mut cam, mut tf)) = cams.single_mut() else { return };
    if lab.snap.is_some() {
        cam.yaw += lab.spin.to_radians();
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
            cam.pitch = (cam.pitch + motion.delta.y * 0.005).clamp(0.02, 1.55);
        }
        if scroll.delta.y != 0.0 {
            cam.distance = (cam.distance * (1.0 - scroll.delta.y * 0.1)).clamp(10.0, 6000.0);
        }
    }
    let dir = Vec3::new(cam.yaw.sin() * cam.pitch.cos(), cam.pitch.sin(), cam.yaw.cos() * cam.pitch.cos());
    *tf = Transform::from_translation(cam.focus + dir * cam.distance).looking_at(cam.focus, Vec3::Y);
}

fn slider<'a>(v: &'a mut f32, range: std::ops::RangeInclusive<f32>, text: &str) -> egui::Slider<'a> {
    egui::Slider::new(v, range).text(text)
}

fn panel(
    mut contexts: EguiContexts,
    mut lab: ResMut<Lab>,
    mut settings: ResMut<VolumeSettings>,
    mut cams: Query<&mut LabCamera>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    if !lab.show_ui {
        return;
    }
    let mut s = *settings;
    egui::Window::new("Fuoco e fumo volumetrici").default_pos([12.0, 12.0]).default_width(330.0).show(ctx, |ui| {
        let fps = 1000.0 / lab.frame_ms.max(0.01);
        ui.heading(format!("{fps:.0} fps · {:.1} ms", lab.frame_ms));
        egui::ScrollArea::vertical().max_height(820.0).show(ui, |ui| {
            egui::CollapsingHeader::new("Tempo").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(if lab.playing { "⏸ pausa" } else { "▶ avvia" }).clicked() {
                        lab.playing = !lab.playing;
                    }
                    if ui.button("⏮ da capo").clicked() {
                        lab.clock = 0.0;
                    }
                });
                let min = lab.clock / 60.0;
                let mut m = min;
                ui.add(slider(&mut m, 0.0..=240.0, "minuti dall'innesco"));
                if m != min {
                    lab.clock = m * 60.0;
                }
                ui.add(slider(&mut lab.speed, 0.1..=300.0, "velocità (s simulati al secondo)").logarithmic(true));
            });
            egui::CollapsingHeader::new("Fuoco").default_open(true).show(ui, |ui| {
                egui::ComboBox::from_label("forma").selected_text(lab.shape.name()).show_ui(ui, |ui| {
                    for shape in [Shape::Line, Shape::Circle, Shape::Patch] {
                        ui.selectable_value(&mut lab.shape, shape, shape.name());
                    }
                });
                ui.add(slider(&mut lab.spread_m_min, 1.0..=80.0, "velocità della testa (m/min)").logarithmic(true));
                ui.add(slider(&mut lab.fli, 200.0..=40_000.0, "intensità (kW/m)").logarithmic(true));
                ui.add(slider(&mut lab.line_m, 100.0..=3000.0, "lunghezza della linea (m)"));
                ui.add(slider(&mut lab.radius_m, 20.0..=800.0, "raggio della chiazza (m)"));
                ui.add(slider(&mut s.smoke_s, 600.0..=6000.0, "s di fumo dopo il fronte"));
                ui.add(slider(&mut s.smoulder, 0.0..=0.6, "fumo delle braci"));
            });
            egui::CollapsingHeader::new("Vento e sole").default_open(true).show(ui, |ui| {
                ui.add(slider(&mut lab.wind_kmh, 0.0..=80.0, "vento (km/h)"));
                ui.add(slider(&mut lab.wind_from, 0.0..=360.0, "vento da (gradi, 0 nord)"));
                ui.add(slider(&mut lab.turn_deg, -180.0..=180.0, "il vento gira di (gradi, solo il fumo)"));
                ui.add(slider(&mut lab.sun_up, 2.0..=89.0, "altezza del sole (gradi)"));
                ui.add(slider(&mut lab.sun_az, 0.0..=360.0, "sole da (gradi)"));
            });
            egui::CollapsingHeader::new("Fumo").default_open(false).show(ui, |ui| {
                ui.add(slider(&mut s.top_m, 100.0..=2000.0, "cima del pennacchio (m)"));
                ui.add(slider(&mut s.rise_ms, 1.0..=25.0, "risalita (m/s)"));
                ui.add(slider(&mut s.spread_m, 10.0..=200.0, "allargamento (m per raddoppio)"));
                ui.add(slider(&mut s.sigma, 0.002..=0.2, "densità (estinzione /m)").logarithmic(true));
                ui.add(slider(&mut s.period_m, 30.0..=600.0, "volute (m)").logarithmic(true));
                ui.add(slider(&mut s.edge, 0.2..=0.7, "bordo delle volute"));
                ui.add(slider(&mut s.lift_m, 5.0..=400.0, "stacco da terra (m)"));
                ui.add(slider(&mut s.shadow, 0.0..=4.0, "ombra propria"));
                ui.add(slider(&mut s.smoke_gain, 0.2..=3.0, "luminosità"));
                ui.add(slider(&mut s.clear_m, 0.0..=1500.0, "diradato vicino alla camera (m)"));
                ui.add(slider(&mut s.pace, 0.5..=60.0, "ritmo del fumo (s suoi per s reale)").logarithmic(true));
                ui.add(slider(&mut s.smooth_s, 0.0..=6.0, "sorgenti ammorbidite (s reali)"));
                ui.add(slider(&mut s.shear, 0.0..=3.0, "dispersione col vento che cambia"));
            });
            egui::CollapsingHeader::new("Fiamme").default_open(false).show(ui, |ui| {
                ui.add(slider(&mut s.flame_gain, 0.05..=3.0, "luce emessa"));
                ui.add(slider(&mut s.flame_absorb, 0.05..=3.0, "opacità (/m)").logarithmic(true));
                ui.add(slider(&mut s.flame_height, 0.2..=3.0, "altezza"));
                ui.add(slider(&mut s.flame_period_m, 2.0..=60.0, "lingue (m)").logarithmic(true));
                ui.add(slider(&mut s.flame_sharp, 0.02..=0.5, "morbidezza del bordo"));
                ui.add(slider(&mut s.flame_temp, 0.3..=2.0, "temperatura"));
                ui.add(slider(&mut s.flame_full_m, 20.0..=3000.0, "altezza vera fino a (m)").logarithmic(true));
                ui.add(slider(&mut s.flame_end_m, 30.0..=6000.0, "scala giocattolo da (m)").logarithmic(true));
                s.flame_end_m = s.flame_end_m.max(s.flame_full_m + 10.0);
                ui.add(slider(&mut s.flame_toy, 1.0..=6.0, "scala giocattolo (volte)"));
                ui.add(slider(&mut s.detail, 0.0..=2.0, "dettaglio fine sui bordi (0: spento)"));
                ui.add(slider(&mut s.detail_m, 0.5..=20.0, "dettaglio: lingue da vicino (m)").logarithmic(true));
            });
            egui::CollapsingHeader::new("Qualità e costo").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.show_flames, "fiamme");
                    ui.checkbox(&mut s.show_smoke, "fumo");
                });
                let mut div = s.div as f32;
                ui.add(slider(&mut div, 1.0..=8.0, "riduzione (pixel per lato)").step_by(1.0));
                s.div = div as u32;
                ui.add(slider(&mut s.steps, 8.0..=300.0, "passi massimi").logarithmic(true));
                ui.add(slider(&mut s.history, 0.0..=0.97, "stabilizzazione (quota dei fotogrammi prima)"));
            });
            egui::CollapsingHeader::new("Camera").default_open(true).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, dist, pitch) in
                        [("60 m", 60.0, 8.0), ("150 m", 150.0, 12.0), ("400 m", 400.0, 18.0), ("1 km", 1000.0, 22.0), ("3 km", 3000.0, 28.0), ("dall'alto", 2000.0, 80.0)]
                    {
                        if ui.button(label).clicked() {
                            if let Ok(mut c) = cams.single_mut() {
                                c.distance = dist;
                                c.pitch = f32::to_radians(pitch);
                            }
                        }
                    }
                });
                if ui.button("centra sul fuoco").clicked() {
                    if let Ok(mut c) = cams.single_mut() {
                        c.focus = Vec3::new(SIDE_M * 0.5, 20.0, -SIDE_M * 0.5);
                    }
                }
            });
            ui.separator();
            if ui.button("parametri di partenza").clicked() {
                s = VolumeSettings::default();
            }
            ui.small(format!("{s:?}"));
        });
    });
    if s != *settings {
        *settings = s;
    }
}

fn snap_when_ready(
    mut commands: Commands,
    mut lab: ResMut<Lab>,
    costs: Res<crate::bench::Costs>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<u32>,
) {
    let target = lab.snap_target.clone();
    // the cost: the median of the frames since the first third settled, and
    // of the GPU's time for the volumes' passes in them (with no window to
    // present to, the frame time alone says little; the whole frame's GPU
    // stamps, `Costs::gpu_pass`, read 0.4 or 9 ms for the same picture here)
    let median = |all: &[f32]| {
        let mut times = all[all.len() / 3..].to_vec();
        times.sort_by(f32::total_cmp);
        times.get(times.len() / 2).copied()
    };
    let ms = median(&lab.frame_times).unwrap_or(lab.frame_ms);
    let vol = median(&costs.volume.lock().unwrap()).unwrap_or(0.0);
    let sequence = lab.sequence;
    let Some((file, frames)) = &mut lab.snap else { return };
    if *frames > 0 {
        *frames -= 1;
        if *frames == 0 && sequence == 0 {
            info!("fuoco_lab: {ms:.2} ms a fotogramma, volumi {vol:.2} ms");
            commands.spawn(Screenshot::image(target)).observe(save_to_disk(file.clone()));
        }
        return;
    }
    let stem = file.trim_end_matches(".png").to_string();
    if *done < sequence {
        commands.spawn(Screenshot::image(target)).observe(save_to_disk(format!("{stem}_{:03}.png", *done)));
    }
    *done += 1;
    let last = if sequence > 0 { format!("{stem}_{:03}.png", sequence - 1) } else { file.clone() };
    if (*done > sequence + 20 && std::path::Path::new(last.as_str()).exists()) || *done > sequence + 1200 {
        exit.write(AppExit::Success);
    }
}
