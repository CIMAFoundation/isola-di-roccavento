//! The wildfire kiosk demo: the one and only mode of this binary.
//!
//! The model runs in-process on the PROPAGATOR Rust core, stepped from the Bevy
//! loop. Python is used only offline, to bake the scenario assets under `data/`.

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    /// Nothing loaded yet: `kiosk::launch` loads the territory and the first game.
    SelectingScenario,
    Playing,
}

mod bench;
mod boats;
mod buildings;
mod camera;
mod coast;
mod field;
mod fields;
mod fire_shader;
mod fire_lab;
mod fire_view;
mod fire_volumetric;
mod forecast_view;
mod forecast_zone;
mod reach_zone;
mod frame;
mod kiosk;
mod landmarks;
#[cfg(feature = "mario")]
mod mario;
#[cfg(not(feature = "mario"))]
#[path = "mario_off.rs"]
mod mario;
mod models;
mod overlays;
mod pad;
mod pick;
mod playtest;
mod adaptive;
mod quality;
mod quality_live;
mod render_scale;
mod record;
mod flat;
mod fonts;
mod godrays;
mod horizon;
mod tactical;
mod tape;
mod materials;
mod rings;
mod ground_mask;
mod roads;
mod shader_lib;
mod sea;
mod sky;
mod sky_light;
mod sim;
mod sound;
mod streamed_mesh;
mod terrain_mesh;
mod terrain_tin;
mod textures;
mod ui;
mod near_cover;
mod units;
mod vegetation;
#[cfg(not(target_arch = "wasm32"))]
mod hi_trees;
#[cfg(not(target_arch = "wasm32"))]
mod veg_lab;
mod visual_scale;
#[cfg(not(target_arch = "wasm32"))]
mod window_state;
#[cfg(not(target_arch = "wasm32"))]
mod memory;
#[cfg(target_arch = "wasm32")]
mod web_input;
#[cfg(all(target_arch = "wasm32", not(feature = "webgpu")))]
mod web_gpu;
#[cfg(any(feature = "webgpu", test))]
mod webgpu_shaders;

use bevy::post_process::bloom::Bloom;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::CascadeShadowConfigBuilder;
use bevy_egui::EguiPlugin;

use crate::camera::OrbitCamera;
use crate::sim::Sim;

/// Where the territory lives (`ROCCA_DATA`, default `data`).
#[derive(Resource)]
pub struct DataPath(pub std::path::PathBuf);

fn main() -> anyhow::Result<()> {
    // `KIOSK_PIANTE=1`: the plants' fire, plant by plant, instead of the game
    #[cfg(not(target_arch = "wasm32"))]
    if veg_lab::wanted() {
        return veg_lab::run();
    }
    // `KIOSK_FUOCO=1`: the fire's volumes on flat ground, every knob in a panel
    if fire_lab::wanted() {
        return fire_lab::run();
    }
    let data = std::env::var("ROCCA_DATA").unwrap_or_else(|_| "data".to_string());
    let mut gfx = quality::Graphics::from_env();
    // a machine short of memory builds a lighter scene (see `memory`)
    #[cfg(not(target_arch = "wasm32"))]
    match memory::begin(quality::Graphics::chosen()) {
        Some(memory::Short::Forced) if std::env::var_os("KIOSK_QUALITY").is_none() => gfx = quality::Graphics::RISPARMIO,
        Some(_) => gfx = gfx.lighter(),
        None => {}
    }
    // the level follows the machine unless it was chosen by hand (see `adaptive`)
    let auto = quality::Graphics::auto_wanted().then(|| adaptive::Adaptive::new(gfx));
    if let Some(a) = &auto {
        gfx = a.initial();
    }
    info!("grafica: {gfx:?}{}", if auto.is_some() { " (adattiva)" } else { "" });

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: kiosk::TITLE.into(),
            // KIOSK_SCALE=1 renders one pixel per point on a Retina screen,
            // as a 1080p kiosk monitor would.
            resolution: {
                // KIOSK_SIZE=900x600 checks the layout on a small screen.
                let (w, h) = std::env::var("KIOSK_SIZE").ok()
                    .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))))
                    .unwrap_or((1600, 1000));
                let mut r: bevy::window::WindowResolution = (w, h).into();
                if let Some(s) = std::env::var("KIOSK_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) {
                    r = r.with_scale_factor_override(s);
                }
                // the browser: the canvas at its own density, at most 2x (`web_input`)
                #[cfg(target_arch = "wasm32")]
                {
                    r = r.with_scale_factor_override(web_input::backing_scale());
                }
                r
            },
            mode: kiosk::window_mode(),
            // KIOSK_SNAP renders to file: a hidden window, so it never takes the focus
            // (KIOSK_SNAP_SHOW=1 shows it: macOS paces a hidden window at ~57 Hz)
            visible: std::env::var("KIOSK_SNAP").is_err() || std::env::var("KIOSK_SNAP_SHOW").is_ok(),
            focused: std::env::var("KIOSK_SNAP").is_err(),
            // in the browser: the page's canvas, filling it
            canvas: Some("#rocca".into()),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: true,
            // KIOSK_FPS measures the real frame cost, so it must not be vsync-capped.
            present_mode: if std::env::var("KIOSK_FPS").is_ok() || kiosk::snap::bench_s().is_some() { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::AutoVsync },
            ..default()
        }),
        ..default()
    }).set(bevy::render::RenderPlugin {
        // Without texture binding arrays Bevy samples the sky's environment map
        // as one view probe (`light_probe::binding_arrays_are_usable`) and binds
        // materials one by one, as before 0.20. With them (Metal has them since
        // wgpu 24) every lit pixel walks the clustered reflection-probe list and
        // samples cube arrays indexed per pixel: measured 2026-10-21 at
        // 5600x3500 «alta» about 61 -> 55 ms, snapshots unchanged (the scene has
        // no reflection probes). Disabling only the non-uniform indexing instead
        // breaks the bindless StandardMaterial shaders.
        render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(bevy::render::settings::WgpuSettings {
            disabled_features: Some(bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY),
            ..default()
        })),
        ..default()
    }))
    // KIOSK_SNAP renders to file: no sound either
    .insert_resource(bevy::audio::GlobalVolume::new(bevy::audio::Volume::Linear(if std::env::var("KIOSK_SNAP").is_ok() { 0.0 } else { 1.0 })))
    .insert_resource(gfx)
    .insert_resource(bevy::light::DirectionalLightShadowMap { size: gfx.shadow_map })
    .add_plugins(EguiPlugin::default())
    .add_plugins(fonts::FontsPlugin)
    .add_plugins(shader_lib::ShaderLibPlugin)
    .add_plugins(fire_shader::FireShaderPlugin)
    .add_plugins(materials::MaterialsPlugin)
    .add_plugins(near_cover::NearCoverPlugin)
    .add_plugins(flat::FlatPlugin)
    .add_plugins(forecast_zone::ZonePlugin)
    .add_plugins(reach_zone::ReachPlugin)
    .add_plugins(reach_zone::RoutePlugin)
    .add_plugins(tape::TapePlugin)
    .add_plugins(tactical::TacticalPlugin)
    // sun, sky and ambient light from the game's clock; the sea and its wind; the boats
    .add_plugins((sky::SkyPlugin, sea::SeaPlugin, boats::BoatsPlugin, landmarks::LandmarksPlugin, coast::CoastPlugin))
    .init_state::<AppState>()
    .init_resource::<units::Glide>()
    .init_resource::<kiosk::snap::SnapView>()
    .init_resource::<kiosk::snap::SnapHide>()
    .init_resource::<record::Recorder>()
    .init_resource::<ui::UiFocus>()
    .add_message::<sim::SimRestarted>()
    .insert_resource(DataPath(std::path::PathBuf::from(&data)))
    // Everything that spawns a scene entity or caches a per-scenario handle
    // builds on entry and is torn down on exit, so a town change is a genuine
    // round trip rather than a second scene laid on top of the first.
    .add_systems(
        OnEnter(AppState::Playing),
        (
            setup_scene,
            fire_view::setup,
            vegetation::spawn,
            near_cover::spawn,
            buildings::spawn,
            overlays::setup,
            units::setup,
            kiosk::marks::setup,
            kiosk::pins::setup,
            kiosk::characters::load_portraits,
        ),
    )
    .add_systems(OnExit(AppState::Playing), teardown_scene);

    #[cfg(not(target_arch = "wasm32"))]
    window_state::install(&mut app);

    // The FPS counter in the demo reads these diagnostics; KIOSK_FPS also logs them.
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default());
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("KIOSK_FPS").is_ok() {
        app.add_plugins(bevy::diagnostic::LogDiagnosticsPlugin::filtered([bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS].into_iter().collect()));
    }
    #[cfg(target_arch = "wasm32")]
    app.add_systems(PreUpdate, (
        web_input::normalize.before(bevy::input::InputSystems).before(bevy_egui::EguiPreUpdateSet::ProcessInput),
        web_input::finish_canceled_touch.after(bevy_egui::EguiPreUpdateSet::ProcessInput).before(bevy_egui::EguiPreUpdateSet::BeginPass),
    ));
    #[cfg(not(target_arch = "wasm32"))]
    if kiosk::snap::active() {
        app.add_plugins(kiosk::snap::SnapPacingPlugin);
    }
    if kiosk::snap::bench_s().is_some() {
        app.add_plugins(bench::BenchPlugin);
    }
    app.add_plugins(streamed_mesh::StreamedMeshPlugin);
    #[cfg(all(target_arch = "wasm32", not(feature = "webgpu")))]
    app.add_plugins(web_gpu::WebGpuPlugin);
    #[cfg(feature = "webgpu")]
    app.add_plugins(webgpu_shaders::WebGpuShadersPlugin);
    app.add_plugins(mario::MarioPlugin);
    app.add_plugins(pad::PadPlugin);
    app.add_plugins(sound::SoundPlugin);
    app.add_plugins(horizon::HorizonPlugin);
    app.add_plugins(godrays::GodRaysPlugin);
    app.add_plugins(fire_volumetric::FireVolumetricPlugin);
    app.add_plugins(render_scale::RenderScalePlugin);
    app.add_plugins(vegetation::PlantRenderPlugin);
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(hi_trees::HiTreesPlugin);
    kiosk_systems(&mut app);
    app.add_plugins(forecast_view::ForecastPlugin);
    app.add_systems(PostUpdate, kiosk::snap::redirect_camera);
    app.add_systems(PostUpdate, kiosk::snap::hide_ui.after(bevy_egui::EguiPostUpdateSet::ProcessOutput));
    app.add_systems(Update, quality_live::apply_live.run_if(resource_changed::<quality::Graphics>.and_then(not(resource_added::<quality::Graphics>))));
    // MSAA is per camera since Bevy 0.15: every camera takes the profile's.
    app.add_systems(PostUpdate, quality_live::msaa_on_new_cameras);
    #[cfg(all(target_arch = "wasm32", not(feature = "webgpu")))]
    app.add_systems(PostUpdate, camera::web_near_plane.before(bevy::camera::CameraUpdateSystems));
    if let Some(a) = auto {
        app.insert_resource(a);
        // not while loading: those long frames are no measure of the machine
        app.add_systems(Update, adaptive::adapt.run_if(in_state(AppState::Playing)));
        app.add_systems(OnEnter(AppState::Playing), adaptive::resettle);
    }
    let exit = app.run();
    #[cfg(target_arch = "wasm32")]
    let _ = exit;
    #[cfg(not(target_arch = "wasm32"))]
    if let bevy::app::AppExit::Error(code) = exit {
        std::process::exit(i32::from(code.get()));
    }
    Ok(())
}

fn setup_scene(
    mut commands: Commands,
    sim: Res<Sim>,
    gfx: Res<quality::Graphics>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut terrain: ResMut<Assets<materials::TerrainMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    terrain_mesh::build(&sim.scn, &mut commands, &mut meshes, &mut terrain, &mut images, &gfx);
    fields::build(&sim.scn, &mut commands, &mut meshes, &mut standard);
    // Roads are drawn by the terrain shader from `ground_mask`.

    // The sun: `sky::update_sky` places and colours it from the game's clock
    // (11:00 to 14:00), so the shadows lengthen and the light warms as it runs.
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: gfx.shadows, ..default() },
        bevy::light::CascadeShadowConfig::from(CascadeShadowConfigBuilder {
            num_cascades: gfx.cascades,
            minimum_distance: 2.0,
            maximum_distance: gfx.shadow_distance,
            first_cascade_far_bound: 80.0,
            overlap_proportion: 0.3,
        }),
        sky::Sun,
        bevy::camera::visibility::RenderLayers::from_layers(&[0, terrain_mesh::SHADOW_LAYER]),
    ));

    let (p, h) = terrain_mesh::cell_ground(&sim.scn, sim.scn.world.cell_of(sim.case.ignition()));
    let distance = 2600.0;
    let focus = frame::to_bevy(p, h);
    let mut camera = commands.spawn((
        Camera3d::default(),
        Tonemapping::AcesFitted,
        gfx.msaa(),
        grading(),
        Transform::from_xyz(focus.x, focus.y + distance * 0.77, focus.z + distance * 0.77),
        // far enough for the land on the horizon (`horizon`, up to ~65 km)
        Projection::from(PerspectiveProjection { far: 70_000.0, ..default() }),
        OrbitCamera { focus, distance, ..default() },
        // colour and reach are set every frame by `sky::update_sky`
        DistanceFog::default(),
    ));
    // Metal has no multi-draw-indirect: the GPU-driven path only adds an indirect
    // buffer per batch (measured 2026-10-21: frame 16.5 -> 13.5 ms at 1600x1000).
    // WebGL2 has no indirect drawing at all.
    if !quality::WEBGL2 {
        camera.insert(bevy::render::view::NoIndirectDrawing);
    }
    // an HDR target on the desktop and WebGPU (WebGL2 has no multisampled HDR targets)
    if !quality::WEBGL2 {
        camera.insert(bevy::camera::Hdr);
    }
    if gfx.smaa {
        camera.insert(bevy::anti_alias::smaa::Smaa::default());
    }
    if gfx.bloom {
        camera.insert(Bloom { intensity: 0.20, ..Bloom::NATURAL });
    }
    // The depth prepass is there for the depth of field only, which reads the
    // depth at screen resolution: off while the scene is scaled (`render_scale`).
    if gfx.dof && render_scale::whole(gfx.render_scale) {
        camera.insert((
            bevy::core_pipeline::prepass::DepthPrepass,
            bevy::post_process::dof::DepthOfField {
                mode: bevy::post_process::dof::DepthOfFieldMode::Gaussian,
                focal_distance: distance,
                sensor_height: 0.01866,
                aperture_f_stops: 0.012,
                max_circle_of_confusion_diameter: 48.0,
                max_depth: 12_000.0,
            },
        ));
    }
}

/// Empty the world on the way back to the loading state.
///
/// Everything the scene is made of is a root entity with a `Transform`, and
/// nothing else in the app is, so the query *is* the definition of "the scene"
/// and cannot go stale when something new is spawned.
fn teardown_scene(
    mut commands: Commands,
    roots: Query<Entity, (With<Transform>, Without<ChildOf>)>,
) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

/// The kiosk's schedule.
fn kiosk_systems(app: &mut App) {
    // Since bevy_egui 0.42 the interface is drawn only inside the egui pass
    // (`EguiPrimaryContextPass`, in `PostUpdate`), no longer in `Update`
    // before `kiosk::step`: what is clicked there reaches `step` on the next frame.
    app.add_systems(bevy_egui::EguiPrimaryContextPass, kiosk::ui::draw.run_if(in_state(AppState::Playing)));
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(bevy_egui::EguiPrimaryContextPass, kiosk::ui::fps.after(kiosk::ui::draw).run_if(in_state(AppState::Playing)));
    app.add_systems(Update, near_cover::update.run_if(in_state(AppState::Playing)));
    app.add_systems(Update, kiosk::view::curtain.after(sky::update_sky).run_if(in_state(AppState::Playing)));
    app.add_systems(
        Update,
        (
            kiosk::launch.run_if(in_state(AppState::SelectingScenario)),
            (kiosk::step, kiosk::snap::drive, kiosk::view::camera.run_if(map_camera_allowed), kiosk::snap::hold, playtest::observe)
                .chain()
                .run_if(in_state(AppState::Playing)),
            kiosk::snap::hide_marks
                .after(kiosk::pins::update)
                .after(kiosk::marks::update_teams)
                .after(forecast_view::update)
                .run_if(in_state(AppState::Playing)),
            (fire_view::reset, buildings::reset, overlays::reset, units::reset, kiosk::marks::reset)
                .after(kiosk::step)
                .run_if(in_state(AppState::Playing)),
            (
                fire_view::update_fire_map,
                fire_view::update_flames,
                (vegetation::burn, vegetation::clock),
                buildings::damage,
                units::update_units,
                units::animate_units,
                rings::spin,
                overlays::no_ui_shadows,
                units::sync_orders,
                units::update_work_overlay,
                units::animate_spray,
                kiosk::marks::update_hexes,
                kiosk::marks::update_orders,
                kiosk::marks::update_district_zone,
                kiosk::marks::update_closed,
                kiosk::marks::update_teams,
                kiosk::marks::bob_zone,
                kiosk::pins::update,
            )
                .after(fire_view::reset)
                .after(buildings::reset)
                .after(units::reset)
                .after(kiosk::marks::reset)
                .run_if(in_state(AppState::Playing)),
        ),
    );
}

fn map_camera_allowed(mario: Res<mario::Playground>) -> bool {
    !mario.active
}

/// The film look on top of the tone mapper. With TonyMcMapface it added
/// contrast and saturation (it washes bright, saturated colours out, flames
/// included: salmon instead of orange); since 2026-11-01 the tone mapper is
/// AcesFitted (user's choice, compared in `veg_lab`), contrasty and saturated
/// of itself, and the grading is neutral: on top of it the sea went navy.
fn grading() -> bevy::render::view::ColorGrading {
    bevy::render::view::ColorGrading::default()
}
