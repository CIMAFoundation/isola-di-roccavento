//! The language of the game (user, 2026-10-21): chosen by `ROCCA_LANG` (or
//! `?lang=` on the web page), else the player's last choice, else the first
//! supported of the system's or browser's preferred languages
//! (`navigator.languages`, 2026-10-30), else English (2026-10-30). The title and intro
//! screens have a button with the flags, bottom right (2026-10-30); the
//! settings menu (F2) has the list. Klingon is in neither: Ctrl+Shift+K
//! switches it on and off. The texts themselves are in `rocca::i18n` and
//! `data/i18n/`.

use bevy_egui::egui::{self, Align2};

/// What the player chose before, if anything.
#[cfg(not(target_arch = "wasm32"))]
fn saved() -> Option<String> {
    std::fs::read_to_string(path()).ok().map(|s| s.trim().to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".rocca_ventosa").join("lingua.txt")
}

#[cfg(not(target_arch = "wasm32"))]
fn save(code: &str) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, code);
}

#[cfg(target_arch = "wasm32")]
fn saved() -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item("rocca_ventosa_lingua").ok()?
}

#[cfg(target_arch = "wasm32")]
fn save(code: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item("rocca_ventosa_lingua", code);
    }
}

/// The system's preferred language tags (`de-AT`, …), in order.
#[cfg(not(target_arch = "wasm32"))]
fn system() -> Vec<String> {
    sys_locale::get_locales().collect()
}

/// The browser's preferred languages: `navigator.languages`, else
/// `navigator.language`.
#[cfg(target_arch = "wasm32")]
fn system() -> Vec<String> {
    let Ok(nav) = js_sys::Reflect::get(&js_sys::global(), &"navigator".into()) else { return Vec::new() };
    let list: Vec<String> = js_sys::Reflect::get(&nav, &"languages".into())
        .ok()
        .filter(|v| js_sys::Array::is_array(v))
        .map(|v| js_sys::Array::from(&v).iter().filter_map(|t| t.as_string()).collect())
        .unwrap_or_default();
    if !list.is_empty() {
        return list;
    }
    js_sys::Reflect::get(&nav, &"language".into()).ok().and_then(|v| v.as_string()).into_iter().collect()
}

/// The page may force a language (`?lang=de`): `window.roccaLang`.
#[cfg(target_arch = "wasm32")]
fn forced() -> Option<String> {
    js_sys::Reflect::get(&js_sys::global(), &"roccaLang".into()).ok()?.as_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn forced() -> Option<String> {
    std::env::var("ROCCA_LANG").ok()
}

/// The language when the system's or browser's is not one the game speaks
/// (user, 2026-10-30): English. Italian stays the language of the sources.
const FALLBACK: &str = "en";

/// Picks the language at start. The easter egg is only taken when forced; a
/// saved Klingon is never kept (see [`toggle_klingon`]).
pub fn init(data: &std::path::Path) {
    rocca::i18n::set_root(data);
    let code = forced()
        .as_deref()
        .and_then(rocca::i18n::parse)
        .or_else(|| rocca::i18n::first(saved()))
        .or_else(|| rocca::i18n::first(system()))
        .unwrap_or(FALLBACK);
    rocca::i18n::set(code);
}

/// The player picks a language: used from now on and remembered.
pub fn choose(code: &str) {
    if rocca::i18n::set(code) == code {
        save(code);
    }
}

/// Ctrl+Shift+K: Klingon on, and off again back to the language chosen before.
pub fn toggle_klingon() {
    if rocca::i18n::code() == rocca::i18n::EASTER_EGG.0 {
        rocca::i18n::set(rocca::i18n::first(saved()).or_else(|| rocca::i18n::first(system())).unwrap_or(FALLBACK));
    } else {
        rocca::i18n::set(rocca::i18n::EASTER_EGG.0);
    }
}

/// The language list as a combo box with a search field (38 entries).
pub fn picker_ui(ui: &mut egui::Ui) {
    let now = rocca::i18n::code();
    let all = || rocca::i18n::LANGS.iter().chain((now == rocca::i18n::EASTER_EGG.0).then_some(&rocca::i18n::EASTER_EGG));
    let name = all().find(|(c, _)| *c == now).map_or(now, |(_, n)| n);
    let filter_id = egui::Id::new("lingua_filtro");
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt("lingua_scelta").selected_text(format!("🌐 {name}")).height(420.0).show_ui(ui, |ui| {
        let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut filter).hint_text(rocca::tr!("Cerca…")).desired_width(160.0));
        ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
        let needle = filter.to_lowercase();
        for (code, native) in all() {
            if !needle.is_empty() && !native.to_lowercase().contains(&needle) && !code.contains(&needle) {
                continue;
            }
            if ui.selectable_label(*code == now, format!("{native}  ({code})")).clicked() {
                picked = Some(*code);
            }
        }
    });
    if r.inner.is_none() {
        ui.data_mut(|d| d.remove_temp::<String>(filter_id));
    }
    crate::playtest::control("Lingua", &r.response, true);
    if let Some(code) = picked {
        choose(code);
    }
}

/// The language button, bottom right of the title and intro screens
/// (2026-10-30): the globe and the flag in use; it opens a panel with one flag
/// per language, and a flag switches at once (through [`choose`], the same
/// choice and memory as the list in the settings menu). `lift`: room to leave
/// under it (the gamepad help).
pub fn button(ctx: &egui::Context, lift: f32) {
    egui::Area::new(egui::Id::new("lingua_bandiere")).anchor(Align2::RIGHT_BOTTOM, [-10.0, -10.0 - lift]).order(egui::Order::Foreground).show(ctx, |ui| {
        let now = rocca::i18n::code();
        let (rect, r) = ui.allocate_exact_size(egui::vec2(82.0, 44.0), egui::Sense::click());
        let open = egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(&r));
        let p = ui.painter();
        let edge = if r.hovered() || open { egui::Stroke::new(3.0, egui::Color32::WHITE) } else { egui::Stroke::new(1.0, egui::Color32::from_gray(110)) };
        p.rect(rect, 8.0, egui::Color32::from_gray(62), edge, egui::StrokeKind::Inside);
        p.text(rect.left_center() + egui::vec2(10.0, 0.0), Align2::LEFT_CENTER, "🌐", egui::FontId::proportional(22.0), egui::Color32::WHITE);
        super::flags::paint(p, egui::Rect::from_center_size(rect.right_center() - egui::vec2(26.0, 0.0), egui::vec2(33.0, 22.0)), now);
        let r = r.on_hover_text(name(now));
        crate::playtest::control("Lingua", &r, true);
        egui::Popup::from_toggle_button_response(&r)
            .align(egui::RectAlign::TOP_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| flag_grid(ui));
    });
}

fn name(code: &str) -> &'static str {
    rocca::i18n::LANGS.iter().chain([&rocca::i18n::EASTER_EGG]).find(|(c, _)| *c == code).map_or("", |(_, n)| n)
}

/// One flag per language, six to a row, the code under each; the one in use
/// has an amber frame. Clicking (or tapping) a flag chooses its language.
fn flag_grid(ui: &mut egui::Ui) {
    let now = rocca::i18n::code();
    ui.label(egui::RichText::new(name(now)).size(17.0).strong().color(egui::Color32::WHITE));
    ui.add_space(4.0);
    let mut picked = None;
    egui::Grid::new("lingua_griglia").spacing([4.0, 4.0]).show(ui, |ui| {
        for (i, (code, native)) in rocca::i18n::LANGS.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(egui::vec2(46.0, 44.0), egui::Sense::click());
            let p = ui.painter();
            if *code == now {
                p.rect(rect, 6.0, egui::Color32::from_white_alpha(18), egui::Stroke::new(2.5, super::ui::AMBER), egui::StrokeKind::Inside);
            } else if r.hovered() {
                p.rect_stroke(rect, 6.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
            }
            super::flags::paint(p, egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 15.0), egui::vec2(33.0, 22.0)), code);
            p.text(rect.center_bottom() - egui::vec2(0.0, 3.0), Align2::CENTER_BOTTOM, code.to_uppercase(), egui::FontId::proportional(11.0), egui::Color32::from_gray(220));
            let r = r.on_hover_text(*native);
            crate::playtest::control(native, &r, true);
            if r.clicked() {
                picked = Some(*code);
            }
            if i % 6 == 5 {
                ui.end_row();
            }
        }
    });
    if let Some(code) = picked {
        choose(code);
    }
}
