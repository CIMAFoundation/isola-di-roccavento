//! egui fonts: the default ones, plus Hack as the last fallback of the
//! proportional family. Ubuntu Light and the emoji fonts lack the arrows and
//! geometric shapes the UI uses (▲ ▼ ● △ □ →), which only Hack has: without it
//! they draw as empty boxes.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, setup);
    }
}

fn setup(mut contexts: EguiContexts, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    ctx.set_fonts(fonts());
    *done = true;
}

pub fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let hack = "Hack".to_owned();
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if !family.contains(&hack) {
            family.push(hack);
        }
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character the UI and the chat put in proportional text must
    /// have a glyph (U+FE0F has none: it must not appear in the catalogues).
    #[test]
    fn the_ui_symbols_have_a_glyph() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let _ = ctx.run_ui(Default::default(), |_| {});
        let id = egui::FontId::proportional(16.0);
        // Not the emoji: `has_glyph` answers false for anything in the face that
        // also holds the replacement glyph (NotoEmoji), even when it draws it.
        for ch in "▲▼●△□→−≈".chars() {
            assert!(ctx.fonts_mut(|f| f.has_glyph(&id, ch)), "no glyph for {ch:?}");
        }
        let catalogues = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/i18n")).unwrap();
        for entry in catalogues {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains('\u{FE0F}'), "U+FE0F in {}", path.display());
        }
    }
}
