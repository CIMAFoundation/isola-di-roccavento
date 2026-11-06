//! The scenery's materials, one type per kind of surface.
//!
//! - [`TerrainMaterial`]: the ground, its roads and meadows (`terrain.wgsl`).
//! - [`StructureMaterial`]: buildings, landmarks, boats, rocks and the units:
//!   StandardMaterial plus a procedural [`Finish`] and a PBR response per part
//!   (`surface_alpha`, baked per vertex) (`structure.wgsl`).
//! - [`masonry::MasonryMaterial`]: the buildings and landmarks in photographed
//!   plaster, tiles and stone, only when the plants are scans (`models::scans`).
//! - The vegetation is not a material: it has a pipeline of its own
//!   (`vegetation/render.rs`).
//! - Everything else (pins, rings, jets, fields) is a plain `StandardMaterial`.
//!
//! All of them are lit by Bevy's PBR (`apply_pbr_lighting`) and fogged by it.
//! The terrain is a material of its own rather than extensions of
//! StandardMaterial because of WebGL2: it has 16 texture units per stage, and
//! every texture in a pipeline's layout takes one whether the shader reads it or
//! not. StandardMaterial's six would leave the terrain no room for its four.
//! The shared surface functions are in `scenery.wgsl` (`rocca::scenery`).

pub mod masonry;
mod structure;
mod terrain;

pub use structure::{structure, surface_alpha, Finish, StructureMaterial};
pub use terrain::TerrainMaterial;

use bevy::asset::uuid_handle;
use bevy::prelude::*;

const SCENERY_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f14");

pub struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SCENERY_SHADER_HANDLE, "../shaders/scenery.wesl");
        app.add_plugins((terrain::TerrainPlugin, structure::StructurePlugin, masonry::MasonryPlugin));
    }
}
