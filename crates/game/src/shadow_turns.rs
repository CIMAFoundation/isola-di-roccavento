//! The sun's far cascades take turns (2026-11-13, user: «optimize graphics
//! performances without losing fidelity»).
//!
//! The shadows are the largest single cost of a frame (`KIOSK_SNAP_BENCH`,
//! 1600x1000, realistic style and volumes: 1.6 ms of 14 from the play
//! camera, 6.5 of 22 at 300 m from the front), and two thirds of what the
//! plants cast is in the two far cascades (`KIOSK_VEG_STATS=1`: 3.9 of 4.7
//! million triangles from the play camera, 6.9 of 11.6 at 300 m). Those
//! cascades are read from 200 m of the camera on, and nothing in them
//! changes in a sixtieth of a second that shows from there: the plants' sway
//! is under a texel, the sun moves two hundredths of a degree a frame at
//! x300.
//!
//! So each is redrawn every other frame, one on the even frames and one on
//! the odd. On the frame it is not, it stands as it was drawn: its matrices
//! are put back as they were ([`keep`], in the main world, before the
//! lights' frusta are taken from them), so what is read is what was drawn,
//! where it was drawn; and its view is taken off the camera's list before
//! anything is queued for it ([`skip`], in the render world), so neither
//! Bevy nor the plants draw into it and its layer of the shadow map stays.
//!
//! A cascade is not kept when it has moved: its centre more than
//! [`DRIFT_TEXELS`] of its own texels from the one kept (the camera turning
//! or flying fast: every frame draws every cascade, as before), its size
//! changed, the sun turned. What casts into a cascade is chosen for the
//! screen as it is ([`crate::vegetation`], the slice of the view that reads
//! it): for the cascades that take turns, with [`slack_texels`] more round
//! it, what the view can have moved by while one is kept.
//!
//! `KIOSK_SHADOW_TURNS=0` draws every cascade every frame, to compare.

use bevy::light::cascade::Cascade;
use bevy::light::{Cascades, DirectionalLight, SimulationLightSystems};
use bevy::pbr::{LightEntity, ViewLightEntities, ViewShadowBindings};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::TextureId;
use bevy::render::{Render, RenderApp, RenderSystems};

/// The first cascade that takes turns (they count from 0, the nearest):
/// those read from 200 m on, with four cascades.
pub const FIRST: usize = 2;
/// Texels a cascade's centre may be from the one kept: past it the cascade
/// is redrawn whatever the turn.
const DRIFT_TEXELS: f32 = 8.0;
/// The sun may have turned by this (cosine of a tenth of a degree).
const SUN_COS: f32 = 0.999_998_5;
/// The most cascades a light has (Bevy's).
const MOST: usize = 4;

/// Whether the far cascades take turns: unless `KIOSK_SHADOW_TURNS=0`.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KIOSK_SHADOW_TURNS").as_deref() != Ok("0"))
}

/// Texels more, round the slice of the view that reads cascade `index`, in
/// which a plant still casts into it: the view's far corners move up to
/// three times as far as the cascade's centre when the camera turns.
pub fn slack_texels(index: usize) -> f32 {
    if on() && index >= FIRST { 3.0 * DRIFT_TEXELS } else { 0.0 }
}

/// The cascades that stand as they were drawn, this frame.
#[derive(Resource, Clone, Copy, Default, PartialEq, ExtractResource)]
#[extract_app(RenderApp)]
pub struct KeptCascades(pub [bool; MOST]);

/// What each cascade was when last drawn, and for which sun and camera.
#[derive(Resource, Default)]
struct Drawn {
    of: Option<(Entity, Entity)>,
    cascades: Vec<Option<Cascade>>,
    frame: u32,
}

pub struct ShadowTurnsPlugin;

impl Plugin for ShadowTurnsPlugin {
    fn build(&self, app: &mut App) {
        if !on() {
            return;
        }
        app.init_resource::<KeptCascades>()
            .init_resource::<Drawn>()
            .add_plugins(ExtractResourcePlugin::<KeptCascades>::default())
            .add_systems(
                PostUpdate,
                keep.after(SimulationLightSystems::UpdateDirectionalLightCascades).before(SimulationLightSystems::UpdateLightFrusta),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.add_systems(Render, skip.after(bevy::pbr::prepare_lights).before(RenderSystems::Queue));
    }
}

/// Whether a cascade fitted now is the one kept, near enough.
fn same(was: &Cascade, now: &Cascade) -> bool {
    let turn = was.world_from_cascade.z_axis.truncate().dot(now.world_from_cascade.z_axis.truncate());
    let moved = (now.world_from_cascade.w_axis - was.world_from_cascade.w_axis).truncate().length();
    was.texel_size == now.texel_size && turn >= SUN_COS && moved <= DRIFT_TEXELS * now.texel_size
}

/// Puts back the matrices of the cascades whose turn it is not, and says
/// which they are.
fn keep(mut suns: Query<(Entity, &DirectionalLight, &mut Cascades)>, mut drawn: ResMut<Drawn>, mut kept: ResMut<KeptCascades>) {
    let mut now = [false; MOST];
    drawn.frame = drawn.frame.wrapping_add(1);
    // the one sun's cascades for the one camera: with a second view (Mario's)
    // the shadow map's layers are shared out anew
    let mut lit = suns.iter_mut().filter(|(_, light, _)| light.shadow_maps_enabled);
    let one = match (lit.next(), lit.next()) {
        (Some((sun, _, cascades)), None) if cascades.cascades.len() == 1 => Some((sun, cascades)),
        _ => None,
    };
    let Some((sun, mut cascades)) = one else {
        drawn.of = None;
        kept.set_if_neq(KeptCascades(now));
        return;
    };
    let Some((&view, list)) = cascades.cascades.iter_mut().next() else { return };
    if drawn.of != Some((sun, view)) || drawn.cascades.len() != list.len() {
        drawn.of = Some((sun, view));
        drawn.cascades = vec![None; list.len()];
    }
    let frame = drawn.frame as usize;
    for (i, cascade) in list.iter_mut().enumerate().take(MOST).skip(FIRST) {
        let turn = (frame + i) % 2 == 0;
        match &drawn.cascades[i] {
            Some(was) if !turn && same(was, cascade) => {
                *cascade = was.clone();
                now[i] = true;
            }
            _ => drawn.cascades[i] = Some(cascade.clone()),
        }
    }
    kept.set_if_neq(KeptCascades(now));
}

/// Takes the kept cascades' views off the camera's list, so nothing is
/// queued for them nor drawn into them, where their layer of the shadow map
/// holds what was drawn before.
fn skip(
    kept: Option<Res<KeptCascades>>,
    mut views: Query<(&mut ViewLightEntities, &ViewShadowBindings)>,
    light_views: Query<&LightEntity>,
    // the shadow map the layers were drawn into, and which of them have been
    mut held: Local<(Option<TextureId>, [bool; MOST])>,
) {
    let mut cameras = views.iter_mut();
    let (Some(kept), Some((mut lights, bindings)), None) = (kept, cameras.next(), cameras.next()) else {
        *held = default();
        return;
    };
    let map = bindings.directional_light_depth_texture.id();
    if held.0 != Some(map) {
        // another shadow map (its size, a level of graphics): nothing in it yet
        *held = (Some(map), [false; MOST]);
    }
    let index = |entity: &Entity| match light_views.get(*entity) {
        Ok(&LightEntity::Directional { cascade_index, .. }) if cascade_index < MOST => Some(cascade_index),
        _ => None,
    };
    let stays = |entity: &Entity| index(entity).is_some_and(|i| kept.0[i] && held.1[i]);
    if lights.lights.iter().any(stays) {
        lights.lights.retain(|entity| !stays(entity));
    }
    for entity in &lights.lights {
        if let Some(i) = index(entity) {
            held.1[i] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cascade(centre: Vec3, texel: f32) -> Cascade {
        Cascade { world_from_cascade: Mat4::from_translation(centre), texel_size: texel, ..default() }
    }

    /// A cascade is kept while its centre is within a few of its texels of
    /// the one drawn, and not past that, nor at another size, nor with the
    /// sun turned.
    #[test]
    fn a_cascade_is_kept_while_it_has_hardly_moved() {
        let was = cascade(Vec3::new(4000.0, 300.0, -5000.0), 3.2);
        assert!(same(&was, &cascade(Vec3::new(4000.0 + 0.9 * DRIFT_TEXELS * 3.2, 300.0, -5000.0), 3.2)));
        assert!(!same(&was, &cascade(Vec3::new(4000.0 + 1.1 * DRIFT_TEXELS * 3.2, 300.0, -5000.0), 3.2)));
        assert!(!same(&was, &cascade(Vec3::new(4000.0, 300.0, -5000.0), 3.3)));
        let mut turned = was.clone();
        turned.world_from_cascade = Mat4::from_translation(Vec3::new(4000.0, 300.0, -5000.0)) * Mat4::from_rotation_x(0.2f32.to_radians());
        assert!(!same(&was, &turned));
    }

    /// Only the far cascades take turns, and only they have the slack.
    #[test]
    fn the_near_cascades_are_drawn_every_frame() {
        assert_eq!(slack_texels(0), 0.0);
        assert_eq!(slack_texels(1), 0.0);
    }
}
