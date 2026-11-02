//! Stands in for `mario.rs` when the `mario` feature is off (the published
//! desktop builds): Mario never starts, so `active` stays false.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Playground {
    pub(crate) simulation_speed: f32,
    pub active: bool,
}

impl Playground {
    pub(crate) fn request_start_for_snapshot(&mut self, _wing_cap: bool, _at: Option<Vec2>, _walk: bool) {}
    pub(crate) fn snapshot_ready(&self) -> bool { true }
}

pub struct MarioPlugin;
impl Plugin for MarioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Playground>();
    }
}
