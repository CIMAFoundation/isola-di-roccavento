//! Map overlays for the kiosk: the rule
//! that 3D interface marks cast and take no shadows. The orders' signs are in
//! `kiosk::marks`.
//!
//! Everything sits well above the canopy (finding 13: a correct overlay
//! under the plants is seen never), is unlit and glowing so the bloom pass
//! catches it, and is rebuilt from `Sim` state with no latch of its own — so a
//! restart needs only the key cleared (finding 21).

use bevy::prelude::*;



#[derive(Component)]
pub struct DynamicMarker;

/// Any other 3D UI mesh that must not take part in shadows.
#[derive(Component)]
pub struct UiGlyph;
#[derive(Component)]
pub struct BeaconLayer;

/// Rings for the player's own orders: a warned district, an engine's post.
#[derive(Component, Clone, Copy)]
pub struct OrderRing;

/// Routes on the map: the ones being driven and the proposed ones.
#[derive(Component, Clone, Copy)]
pub struct RouteMark;

#[derive(Resource)]
pub struct OverlayAssets {
    key: (usize, usize, i64),
    /// What the order rings were built for: (warned districts, engine posts).
    order_key: (usize, usize),
    /// What the routes were built for.
    route_key: u64,
}

// The two glowing ring materials and their pulse (`pulse_rings`) are gone
// (2026-11-01): no entity drew them, and changing two materials every frame
// made two new GPU buffers a frame on WebGL2.
pub fn setup(mut commands: Commands) {
    commands.insert_resource(OverlayAssets { key: (usize::MAX, 0, 0), order_key: (usize::MAX, 0), route_key: u64::MAX });
}

/// 3D UI is a graphic layer, not part of the scene: it casts and receives no
/// shadows. Runs on whatever was just spawned.
pub fn no_ui_shadows(
    mut commands: Commands,
    new: Query<
        Entity,
        (
            Or<(Added<OrderRing>, Added<RouteMark>, Added<DynamicMarker>, Added<BeaconLayer>, Added<UiGlyph>, Added<crate::units::OrderMarker>)>,
            Without<bevy::light::NotShadowCaster>,
        ),
    >,
) {
    for e in &new {
        // may already be despawned this frame (rings are rebuilt eagerly)
        commands.entity(e).try_insert((bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver));
    }
}

pub fn reset(mut restarted: MessageReader<crate::sim::SimRestarted>, mut assets: ResMut<OverlayAssets>) {
    if !restarted.is_empty() {
        restarted.clear();
        assets.key = (usize::MAX, 0, 0);
        assets.order_key = (usize::MAX, 0);
        assets.route_key = u64::MAX;
    }
}
