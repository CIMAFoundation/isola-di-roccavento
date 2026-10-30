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
    ring_cover: Handle<StandardMaterial>,
    ring_white: Handle<StandardMaterial>,
    /// What the routes were built for.
    route_key: u64,
}

/// The colour of a glowing UI element: `c` as sRGB, its linear value scaled by
/// `gain`. Unlit output ignores emissive and goes through the tone mapper, so
/// anything near 1.0 comes out as muted as the terrain; >1 stays saturated and
/// feeds the bloom. No gain (0) is a plain colour.
fn hdr(c: [f32; 3], a: f32, gain: f32) -> Color {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    let g = if gain > 0.0 { gain } else { 1.0 };
    Color::LinearRgba(LinearRgba::new(l.red * g, l.green * g, l.blue * g, a))
}

fn glow(c: [f32; 3], a: f32, boost: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: hdr(c, a, boost),
        emissive: LinearRgba::rgb(c[0] * boost, c[1] * boost, c[2] * boost),
        unlit: true,
        alpha_mode: if a < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

pub fn setup(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut mat = |c: [f32; 3], a: f32, boost: f32| materials.add(glow(c, a, boost));
    // UI in 3D is all blues: cyan = defended area and active routes, ice = proposed plan.
    let ring_cover = mat([0.00, 0.90, 1.00], 0.9, 1.6);
    let ring_white = mat([0.65, 0.90, 1.00], 0.9, 1.6);
    commands.insert_resource(OverlayAssets {
        key: (usize::MAX, 0, 0),
        order_key: (usize::MAX, 0),
        ring_cover,
        ring_white,
        route_key: u64::MAX,
    });
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

/// Rings and routes breathe: a slow pulse of alpha and glow, out of phase per
/// colour, so they read as UI laid over the landscape and not as scenery.
pub fn pulse_rings(time: Res<Time>, assets: Res<OverlayAssets>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let t = time.elapsed_secs();
    let set = [
        (&assets.ring_cover, [0.00, 0.90, 1.00], 2.2),
        (&assets.ring_white, [0.65, 0.90, 1.00], 3.3),
    ];
    for (h, c, phase) in set {
        let k = 0.5 + 0.5 * (t * 2.6 + phase).sin();
        if let Some(mut m) = materials.get_mut(h) {
            m.base_color = hdr(c, 0.70 + 0.30 * k, 2.2 + 1.8 * k);
        }
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
