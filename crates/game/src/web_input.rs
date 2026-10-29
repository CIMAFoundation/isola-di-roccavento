//! Winit's web pointer coordinates use the browser DPR, even when Bevy's
//! backing canvas uses a scale-factor override. Normalize before both Bevy
//! and egui read them, so a 1x canvas has CSS-pixel hit targets on a 3x phone.
//! bevy_egui 0.43 takes the pointer from the `WindowEvent` stream, not from
//! `CursorMoved`: both are rescaled (2026-10-29: egui was off by the DPR).
use bevy::ecs::message::MessageCursor;
use bevy::input::touch::TouchInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent, WindowResized};

pub fn normalize(
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut resized: MessageWriter<WindowResized>,
    mut touches: ResMut<Messages<TouchInput>>,
    mut cursors: ResMut<Messages<CursorMoved>>,
    mut window_events: ResMut<Messages<WindowEvent>>,
    mut touch_reader: Local<MessageCursor<TouchInput>>,
    mut cursor_reader: Local<MessageCursor<CursorMoved>>,
    mut window_reader: Local<MessageCursor<WindowEvent>>,
) {
    let Ok((entity, mut window)) = windows.single_mut() else { return };
    // WebKit's ResizeObserver reports CSS size × browser DPR even with our
    // 1x override. Keep rendering, egui layout and input in CSS pixels after
    // every resize (including rotation and Safari's changing browser bars).
    if let Some(canvas) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("rocca"))
    {
        let (width, height) = (canvas.client_width() as u32, canvas.client_height() as u32);
        if width > 0 && height > 0
            && (window.physical_width(), window.physical_height()) != (width, height)
        {
            window.resolution.set_physical_resolution(width, height);
            resized.write(WindowResized { window: entity, width: window.width(), height: window.height() });
        }
    }
    let ratio = window.resolution.scale_factor() / window.resolution.base_scale_factor();
    let touch_events: Vec<_> = touch_reader.read(&touches).cloned().map(|mut e| {
        e.position *= ratio;
        e
    }).collect();
    let scale = |mut e: CursorMoved| {
        e.position *= ratio;
        e.delta = e.delta.map(|d| d * ratio);
        e
    };
    let cursor_events: Vec<_> = cursor_reader.read(&cursors).cloned().map(scale).collect();
    let all_events: Vec<_> = window_reader.read(&window_events).cloned().map(|e| match e {
        WindowEvent::CursorMoved(c) => WindowEvent::CursorMoved(scale(c)),
        e => e,
    }).collect();
    if let Some(last) = cursor_events.last() { window.set_cursor_position(Some(last.position)); }
    // Publish only fresh events. Advance our readers past the replacement
    // events so the next frame never repeats a press/release or rescales it.
    touches.clear();
    touches.write_batch(touch_events);
    touch_reader.clear(&touches);
    cursors.clear();
    cursors.write_batch(cursor_events);
    cursor_reader.clear(&cursors);
    window_events.clear();
    window_events.write_batch(all_events);
    window_reader.clear(&window_events);
}

/// bevy_egui 0.28 emits PointerGone on touchCancel without releasing its
/// emulated mouse button. Explicitly release it, while retaining Touch::Cancel
/// so the game can discard the order rather than treat this as a drop.
pub fn finish_canceled_touch(mut inputs: Query<&mut bevy_egui::EguiInput>) {
    use bevy_egui::egui;
    for mut input in &mut inputs {
        let canceled = input.events.iter().find_map(|event| match event {
            egui::Event::Touch { phase: egui::TouchPhase::Cancel, pos, .. } => Some(*pos),
            _ => None,
        });
        if let Some(pos) = canceled {
            let modifiers = egui::Modifiers::NONE; // egui 0.36: RawInput no longer carries modifiers
            input.events.push(egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed: false, modifiers,
            });
            input.events.push(egui::Event::PointerGone);
        }
    }
}
