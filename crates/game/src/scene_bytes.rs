//! What the scene made at launch weighs before it goes to the graphics card
//! (2026-11-13): one line in the log, and with `KIOSK_SCENE_BYTES=1` (in the
//! browser `?scene_bytes=1`) the largest meshes and textures one by one.
//!
//! Why it matters in the browser: Bevy hands every mesh and texture over in
//! the first frame it draws. One kept on both sides
//! (`RenderAssetUsages::default()`) is copied for the renderer, one made for
//! the card only is moved; then each waits for the end of the frame in the
//! buffer it is sent from. A phone's page with the realistic style grew by
//! 219 MB in that frame, past the 320 MiB the wasm starts with
//! (`.cargo/config.toml`): about half the copies, half the buffers waiting.
//! So what nothing reads back should be made for the card only, and this
//! line says what is not.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;

pub struct SceneBytesPlugin;

impl Plugin for SceneBytesPlugin {
    fn build(&self, app: &mut App) {
        // at the end of the first frame with the scene in it: it is built on
        // entering the game, and handed to the renderer after this
        app.add_systems(Last, report.run_if(in_state(crate::AppState::Playing)));
    }
}

fn listed() -> bool {
    if cfg!(target_arch = "wasm32") {
        crate::quality::web_param("scene_bytes").as_deref() == Some("1")
    } else {
        std::env::var("KIOSK_SCENE_BYTES").as_deref() == Ok("1")
    }
}

/// Bytes of a mesh as the card takes it: its vertices and its indices.
pub(crate) fn mesh_bytes(mesh: &Mesh) -> Option<(usize, usize, usize)> {
    let vertex: usize = mesh.try_attributes().ok()?.map(|(a, _)| a.format.size() as usize).sum();
    let count = mesh.count_vertices();
    let indices = mesh.try_indices_option().ok()?.map_or(0, |i| i.len() * if matches!(i, bevy::mesh::Indices::U16(_)) { 2 } else { 4 });
    Some((vertex * count + indices, vertex, count))
}

pub(crate) fn report(meshes: Res<Assets<Mesh>>, images: Res<Assets<Image>>, users: Query<(&Mesh3d, &GlobalTransform)>, mut done: Local<bool>) {
    const MB: f32 = 1_048_576.0;
    if std::mem::replace(&mut *done, true) {
        return;
    }
    let both = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
    let mut kept = [0usize; 2];
    let mut card = [0usize; 2];
    let mut rows: Vec<(usize, String)> = Vec::new();
    // where each mesh is drawn, and by how many entities: what it is
    let mut drawn: std::collections::HashMap<AssetId<Mesh>, (usize, Vec3)> = Default::default();
    if listed() {
        for (mesh, at) in &users {
            let e = drawn.entry(mesh.id()).or_insert((0, at.translation()));
            e.0 += 1;
        }
    }
    for (id, mesh) in meshes.iter() {
        let Some((bytes, vertex, count)) = mesh_bytes(mesh) else { continue };
        if mesh.asset_usage == both { kept[0] += bytes } else { card[0] += bytes }
        let side = if mesh.asset_usage == both { "kept on both sides" } else { "for the card only" };
        let size = match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
            Ok(Some(bevy::mesh::VertexAttributeValues::Float32x3(p))) if !p.is_empty() => {
                let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| (lo.min(Vec3::from_array(*v)), hi.max(Vec3::from_array(*v))));
                hi - lo
            }
            _ => Vec3::ZERO,
        };
        let (n, at) = drawn.get(&id).copied().unwrap_or((0, Vec3::ZERO));
        rows.push((bytes, format!("mesh, {count} vertices of {vertex} B, {side}; {:.0} x {:.0} x {:.0} m, drawn {n} times, first at ({:.0}, {:.0}, {:.0})", size.x, size.y, size.z, at.x, at.y, at.z)));
    }
    for (_, image) in images.iter() {
        let bytes = image.data.as_ref().map_or(0, Vec::len);
        if image.asset_usage == both { kept[1] += bytes } else { card[1] += bytes }
        let size = image.texture_descriptor.size;
        rows.push((bytes, format!("texture, {}x{}x{} {:?}, {}", size.width, size.height, size.depth_or_array_layers, image.texture_descriptor.format, if image.asset_usage == both { "kept on both sides" } else { "for the card only" })));
    }
    info!(
        "scene: meshes {:.0} MB kept on both sides and {:.0} MB for the card only; textures {:.0} MB and {:.0} MB",
        kept[0] as f32 / MB, card[0] as f32 / MB, kept[1] as f32 / MB, card[1] as f32 / MB
    );
    if listed() {
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        for (bytes, what) in rows.iter().take(40) {
            info!("scene: {:6.1} MB  {what}", *bytes as f32 / MB);
        }
    }
}
