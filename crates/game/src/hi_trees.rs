//! A trial (2026-11-04), desktop only and off unless asked: full-resolution
//! tree models (Poly Haven, CC0, glTF) standing in for the broadleaves near a
//! point, to judge what a detailed model is worth before any LOD is built.
//!
//! - `KIOSK_HI=<file.glb>`: the model, under Bevy's asset root
//!   (`BEVY_ASSET_ROOT/assets/`); its foot at the origin, Y up.
//! - `KIOSK_HI_AT=<x>:<y>`: around where (scenario metres; default the
//!   island's middle); `KIOSK_HI_R=<m>` how far (120); `KIOSK_HI_MAX=<n>` at
//!   most how many, the nearest (60).
//!
//! The plants it replaces are left out of the instanced vegetation
//! (`vegetation::spawn`). Looks only, no fire: these trees never burn.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};

/// A model is still loading: the snapshot waits (`kiosk::snap`).
pub static PENDING: AtomicBool = AtomicBool::new(false);

pub struct HiSpec {
    pub path: String,
    pub at: Vec2,
    pub radius: f32,
    pub max: usize,
}

pub fn spec() -> Option<HiSpec> {
    let path = std::env::var("KIOSK_HI").ok().filter(|p| !p.is_empty())?;
    let at = std::env::var("KIOSK_HI_AT")
        .ok()
        .and_then(|s| s.split_once(':').and_then(|(x, y)| Some(Vec2::new(x.parse().ok()?, y.parse().ok()?))))
        .unwrap_or(Vec2::new(4000.0, 4000.0));
    let num = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    Some(HiSpec { path, at, radius: num("KIOSK_HI_R", 120.0), max: num("KIOSK_HI_MAX", 60.0) as usize })
}

/// The varieties the model may stand in for: the broadleaves.
/// `KIOSK_HI_ALL=1`: every plant, whatever its species (shrubs become the
/// tree, scaled to their height).
pub fn replaces(name: &str) -> bool {
    if std::env::var("KIOSK_HI_ALL").is_ok_and(|v| v == "1") {
        return true;
    }
    ["holm_", "cork_", "carob_", "olive_"].iter().any(|p| name.starts_with(p))
}

/// The full models to stand: which file, where (Bevy frame foot), turned
/// by `yaw`, scaled to `height` metres (`None`: as modelled).
#[derive(Resource)]
pub struct HiTrees {
    pub spots: Vec<HiSpot>,
}

pub struct HiSpot {
    pub path: String,
    pub foot: Vec3,
    pub yaw: f32,
    pub height: Option<f32>,
}

#[derive(Component)]
pub struct HiTree;

/// Each model's height, metres, once measured.
#[derive(Resource, Default)]
struct ModelHeight(std::collections::HashMap<String, f32>);

pub struct HiTreesPlugin;

impl Plugin for HiTreesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelHeight>().add_systems(Update, (spawn_hi, failed)).add_observer(on_ready);
    }
}

fn spawn_hi(mut commands: Commands, trees: Option<Res<HiTrees>>, assets: Res<AssetServer>, old: Query<Entity, With<HiTree>>) {
    let Some(trees) = trees.filter(|t| t.is_changed()) else { return };
    for e in &old {
        commands.entity(e).despawn();
    }
    if trees.spots.is_empty() {
        return;
    }
    PENDING.store(true, Ordering::Relaxed);
    info!("hi trees: {} plants", trees.spots.len());
    for s in &trees.spots {
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(s.path.clone()));
        commands.spawn((
            HiTree,
            WorldAssetRoot(scene),
            // scaled once the model's height is known (`on_ready`)
            Transform::from_translation(s.foot).with_rotation(Quat::from_rotation_y(s.yaw)),
            HeightWanted(s.height, s.path.clone()),
        ));
    }
}

#[derive(Component)]
struct HeightWanted(Option<f32>, String);

/// A model that cannot load must not hold the snapshot forever.
fn failed(roots: Query<&WorldAssetRoot, (With<HiTree>, Without<Scaled>)>, assets: Res<AssetServer>) {
    let mut waiting = false;
    for r in &roots {
        if let bevy::asset::LoadState::Failed(e) = assets.load_state(&r.0) {
            error!("hi trees: a model did not load: {e}");
        } else {
            waiting = true;
        }
    }
    if !waiting {
        PENDING.store(false, Ordering::Relaxed);
    }
}

/// A model has come in: its height measured once, each tree scaled to its
/// plant's, drawn from both sides, cut-out leaves kept as they are and
/// anything blended made opaque (the glTF's blend mode would sort and drop
/// their shadows).
#[allow(clippy::too_many_arguments)]
fn on_ready(
    ev: On<WorldInstanceReady>,
    mut roots: Query<(&mut Transform, &HeightWanted)>,
    children: Query<&Children>,
    meshes_q: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut height: ResMut<ModelHeight>,
    pending: Query<(), (With<HiTree>, Without<Scaled>)>,
    mut commands: Commands,
) {
    let root = ev.event_target();
    let Ok((mut t, want)) = roots.get_mut(root) else { return };
    let h = *height.0.entry(want.1.clone()).or_insert_with(|| {
        let mut hi: f32 = 0.0;
        for d in children.iter_descendants(root) {
            if let Ok((m, mat, _)) = meshes_q.get(d) {
                if let Some(mesh) = meshes.get(&m.0) {
                    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok() {
                        hi = p.iter().fold(hi, |a, v| a.max(v[1]));
                    }
                }
                if let Some(mut m) = materials.get_mut(&mat.0) {
                    // leaf cards keep their cut-out (glTF MASK, `tools/vegetation/alpha.py`);
                    // anything blended is drawn opaque (sorting, no shadows)
                    if !matches!(m.alpha_mode, AlphaMode::Mask(_)) {
                        m.alpha_mode = AlphaMode::Opaque;
                    }
                    m.double_sided = true;
                    m.cull_mode = None;
                }
            }
        }
        info!("hi trees: {} {hi:.2} m tall", want.1);
        hi.max(0.5)
    });
    if let Some(w) = want.0 {
        t.scale = Vec3::splat(w / h);
    }
    commands.entity(root).insert(Scaled);
    if pending.iter().count() <= 1 {
        PENDING.store(false, Ordering::Relaxed);
    }
}

#[derive(Component)]
struct Scaled;
