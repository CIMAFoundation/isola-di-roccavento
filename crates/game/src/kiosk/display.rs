//! The screen around the interface: full screen on and off (F11, or the
//! button beside «Impostazioni») and the interface's size (− 100 % +, in the
//! settings). A player on a 1920×1080 laptop found the map crowded and F11 did
//! nothing in the browser, which hands every key to the game (2026-11-19).
//!
//! The size is egui's zoom: egui points are then no longer the camera's
//! logical pixels. Everything that crosses between the two goes through
//! [`to_points`] / [`to_logical`] (labels and badges on the map, the ground
//! under the pointer, the side bar that narrows the camera).

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMode};

/// The interface's size steps, as a share of the normal one.
pub const STEPS: [f32; 9] = [0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.25, 1.4, 1.6];

static ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

/// The interface's size now (1 the normal one).
pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(Ordering::Relaxed))
}

pub(super) fn set(z: f32) {
    ZOOM.store(z.to_bits(), Ordering::Relaxed);
}

/// A camera point (logical pixels) in egui points.
pub fn to_points(v: Vec2) -> bevy_egui::egui::Pos2 {
    let z = zoom();
    bevy_egui::egui::pos2(v.x / z, v.y / z)
}

/// An egui point in the camera's logical pixels.
pub fn to_logical(p: bevy_egui::egui::Pos2) -> Vec2 {
    Vec2::new(p.x, p.y) * zoom()
}

/// One step smaller (`-1`) or bigger (`1`), saved for the next visit.
pub fn step(by: i32) {
    let now = STEPS.iter().position(|&s| (s - zoom()).abs() < 0.01).unwrap_or(4) as i32;
    let z = STEPS[(now + by).clamp(0, STEPS.len() as i32 - 1) as usize];
    set(z);
    store(&format!("{z:.2}"));
}

/// The saved size, at launch; not in a snapshot, which must not depend on
/// this computer's choices.
pub fn load() {
    if super::snap::active() {
        return;
    }
    if let Some(z) = saved().and_then(|s| s.trim().parse::<f32>().ok()).filter(|z| (STEPS[0]..=STEPS[STEPS.len() - 1]).contains(z)) {
        set(z);
    }
}

/// Gives egui the size, every frame before the interface is drawn. egui's own
/// Ctrl +/− is off: one size, the one shown in the settings.
pub fn apply(ctx: &bevy_egui::egui::Context) {
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    if (ctx.zoom_factor() - zoom()).abs() > 1e-4 {
        ctx.set_zoom_factor(zoom());
    }
}

/// Asked by the interface this frame, done by [`toggle_system`].
static TOGGLE: AtomicU32 = AtomicU32::new(0);

/// Full screen on or off, at the end of this frame.
pub fn toggle() {
    TOGGLE.store(1, Ordering::Relaxed);
}

/// F11 (not while typing) or the button: full screen on or off.
pub fn toggle_system(keys: Res<ButtonInput<KeyCode>>, focus: Res<crate::ui::UiFocus>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    FULL.store(is_full(windows.single().ok()) as u32, Ordering::Relaxed);
    let asked = TOGGLE.swap(0, Ordering::Relaxed) == 1 || (keys.just_pressed(KeyCode::F11) && !focus.keyboard);
    if !asked || super::snap::active() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        // The whole page, not the canvas alone (Bevy's mode would take the
        // canvas): a key press or a click a moment ago lets the browser do it.
        let _ = &mut windows;
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else if let Some(root) = doc.document_element() {
            let _ = root.request_fullscreen();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(mut w) = windows.single_mut() else { return };
        w.mode = match w.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}

static FULL: AtomicU32 = AtomicU32::new(0);

/// Whether the game filled the screen at the start of this frame.
pub fn full() -> bool {
    FULL.load(Ordering::Relaxed) == 1
}

fn is_full(window: Option<&Window>) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = window;
        web_sys::window().and_then(|w| w.document()).is_some_and(|d| d.fullscreen_element().is_some())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        window.is_some_and(|w| !matches!(w.mode, WindowMode::Windowed))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".rocca_ventosa").join("interfaccia.txt")
}

#[cfg(not(target_arch = "wasm32"))]
fn saved() -> Option<String> {
    std::fs::read_to_string(saved_path()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn store(text: &str) {
    let p = saved_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

#[cfg(target_arch = "wasm32")]
fn saved() -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item("rocca_ventosa_interfaccia").ok()?
}

#[cfg(target_arch = "wasm32")]
fn store(text: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item("rocca_ventosa_interfaccia", text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_and_logical_pixels_round_trip_at_any_size() {
        for &z in &STEPS {
            set(z);
            let p = to_points(Vec2::new(800.0, 450.0));
            assert!((to_logical(p) - Vec2::new(800.0, 450.0)).length() < 1e-3);
        }
        set(1.0);
    }
}
