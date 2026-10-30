//! Diorama proportions, applied only to rendered geometry.
//!
//! World positions, terrain, fuel cells, routes and simulation distances stay
//! in scenario metres. Scale each object around its own ground-level origin,
//! never the scene or the grid. No extra plants or mesh detail are needed.

pub const BUILDING_FOOTPRINT: f32 = 1.9;
pub const BUILDING_HEIGHT: f32 = 2.5;
pub const CAR: f32 = 3.0;
pub const UNIT: f32 = 7.0;
/// Sailing boats on the sea (a 9 m sloop is a speck from the camera otherwise).
pub const BOAT: f32 = 6.5;
pub const TREE: f32 = 1.6;
pub const SHRUB: f32 = 1.25;
