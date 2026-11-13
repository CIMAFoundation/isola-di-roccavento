//! In the browser on WebGL2 the scene made at launch goes to the graphics
//! card a few MB a frame, not whole in the first one (2026-11-13).
//!
//! Bevy hands every new mesh and texture to the renderer at the end of the
//! frame it was made in: one kept on both sides is copied, and each waits for
//! the end of the frame in the buffer it is sent from. For that frame the
//! scene is in memory two or three times, and the wasm's memory, which never
//! shrinks, grows to hold it: a phone's page with the realistic style went
//! 128 MB past the 320 MiB it starts with, and a page that grows is what an
//! iPhone closes (`.cargo/config.toml`).
//!
//! So the large ones are held back and let through a few MB a frame
//! ([`PER_FRAME_MB`]). Held: its `asset_usage` says «not for the renderer»
//! for a while, set without marking the asset changed, so the renderer's
//! extraction passes it over; let through: the usage it had, and marked
//! changed. The asset never leaves `Assets`: what reads a mesh back (the
//! buildings' colours, Mario's floor) finds it all along. Until its turn a
//! mesh is not drawn and a material waits for its texture, as with any asset
//! still loading; the page keeps its loading screen until all have arrived
//! ([`arrived`], `page_ready`).
//!
//! Bevy's own limit (`RenderAssetBytesPerFrame`) was tried first and did not
//! help: the mesh allocator copies every extracted mesh at once whatever the
//! limit, and the ones waiting are kept longer.
//!
//! `?feed=<MB>` tries another pace, `?feed=0` hands everything over at once
//! as before. Not on the desktop nor in the WebGPU build, whose machines are
//! not short of memory.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

/// MB of meshes and textures let through in a frame.
const PER_FRAME_MB: f32 = 8.0;
/// Smaller than this an asset goes at once: what the first frames need
/// (fonts, icons, the tables the shaders read) is small.
const HELD_FROM_BYTES: usize = 128 * 1024;

/// Whether something is still held.
static HOLDING: AtomicBool = AtomicBool::new(false);

/// Bytes a frame, or `None` for everything at once.
fn per_frame() -> Option<usize> {
    if !crate::quality::WEBGL2 {
        return None;
    }
    let mb = crate::quality::web_param("feed").and_then(|v| v.parse::<f32>().ok()).unwrap_or(PER_FRAME_MB);
    (mb > 0.0).then_some((mb * 1_048_576.0) as usize)
}

/// Whether the scene made at launch has all been handed to the renderer.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn arrived() -> bool {
    !HOLDING.load(Ordering::Relaxed)
}

/// What is held, in the order it will go: the textures, then the meshes,
/// each with the usage it had and its bytes.
#[derive(Resource, Default)]
struct Held {
    images: VecDeque<(AssetId<Image>, RenderAssetUsages, usize)>,
    meshes: VecDeque<(AssetId<Mesh>, RenderAssetUsages, usize)>,
    taken: bool,
}

pub struct SceneFeedPlugin;

impl Plugin for SceneFeedPlugin {
    fn build(&self, app: &mut App) {
        if per_frame().is_none() {
            return;
        }
        HOLDING.store(true, Ordering::Relaxed);
        // at the end of the frame, after the scene is built and before the
        // renderer takes what is new
        // (and after the log's line on what the scene weighs, which reads the usages)
        app.init_resource::<Held>().add_systems(Last, (hold, feed).chain().after(crate::scene_bytes::report).run_if(in_state(crate::AppState::Playing)));
    }
}

/// On the first frame with the scene in it: holds every large mesh and
/// texture meant for the renderer.
fn hold(mut held: ResMut<Held>, mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>) {
    if std::mem::replace(&mut held.taken, true) {
        return;
    }
    let large: Vec<_> = images
        .iter()
        .filter(|(_, image)| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .map(|(id, image)| (id, image.asset_usage, image.data.as_ref().map_or(0, Vec::len)))
        .filter(|&(_, _, bytes)| bytes >= HELD_FROM_BYTES)
        .collect();
    for &(id, _, _) in &large {
        if let Some(image) = images.get_mut_untracked(id) {
            image.asset_usage = RenderAssetUsages::MAIN_WORLD;
        }
    }
    held.images = large.into();
    let large: Vec<_> = meshes
        .iter()
        .filter(|(_, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .filter_map(|(id, mesh)| Some((id, mesh.asset_usage, crate::scene_bytes::mesh_bytes(mesh)?.0)))
        .filter(|&(_, _, bytes)| bytes >= HELD_FROM_BYTES)
        .collect();
    for &(id, _, _) in &large {
        if let Some(mesh) = meshes.get_mut_untracked(id) {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD;
        }
    }
    held.meshes = large.into();
    let mb = |bytes: usize| bytes as f32 / 1_048_576.0;
    info!(
        "scene: {} textures ({:.0} MB) and {} meshes ({:.0} MB) go to the card a few MB a frame",
        held.images.len(),
        mb(held.images.iter().map(|h| h.2).sum()),
        held.meshes.len(),
        mb(held.meshes.iter().map(|h| h.2).sum())
    );
}

/// Lets the next few MB through. An asset replaced meanwhile (no longer
/// marked as held) has gone to the renderer by itself and is left alone.
fn feed(mut held: ResMut<Held>, mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>) {
    let Some(budget) = per_frame() else { return };
    if held.images.is_empty() && held.meshes.is_empty() {
        HOLDING.store(false, Ordering::Relaxed);
        return;
    }
    let mut sent = 0;
    while sent < budget {
        if let Some((id, usage, bytes)) = held.images.pop_front() {
            if images.get(id).is_some_and(|image| image.asset_usage == RenderAssetUsages::MAIN_WORLD) {
                if let Some(mut image) = images.get_mut(id) {
                    image.asset_usage = usage;
                    sent += bytes;
                }
            }
        } else if let Some((id, usage, bytes)) = held.meshes.pop_front() {
            if meshes.get(id).is_some_and(|mesh| mesh.asset_usage == RenderAssetUsages::MAIN_WORLD) {
                if let Some(mut mesh) = meshes.get_mut(id) {
                    mesh.asset_usage = usage;
                    sent += bytes;
                }
            }
        } else {
            break;
        }
    }
}
