//! The orbit camera's state. The kiosk drives it (`kiosk::view`).

use bevy::prelude::*;

/// Offscreen helper views do not drive scenery LOD or camera-facing effects.
#[derive(Component)]
pub struct AuxiliaryCamera;

#[derive(Component)]
pub struct OrbitCamera {
    /// Point on the ground the camera looks at, in Bevy space.
    pub focus: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { focus: Vec3::ZERO, distance: 3500.0, yaw: 0.35, pitch: -0.75 }
    }
}

/// The browser's near plane, a share of the camera's distance. WebGL keeps
/// OpenGL's depth range, and wgpu's mapping to it costs the reversed depth
/// its precision: with Bevy's 0.1 m the depth steps were metres a few
/// kilometres out, and the sea (0.8 m) lost to the sea floor beside the
/// islets, in dark patches that moved with the view (2026-11-02). The camera
/// stays 90 m from its focus at the closest (Mario's, about 36 m).
#[cfg(target_arch = "wasm32")]
pub fn web_near_plane(mut cameras: Query<(&OrbitCamera, &mut Projection), Without<AuxiliaryCamera>>) {
    for (orbit, mut projection) in &mut cameras {
        let near = (orbit.distance * 0.02).clamp(0.1, 50.0);
        if let Projection::Perspective(p) = projection.as_ref() {
            if (p.near - near).abs() <= p.near * 0.05 {
                continue;
            }
        }
        if let Projection::Perspective(p) = projection.as_mut() {
            p.near = near;
        }
    }
}
