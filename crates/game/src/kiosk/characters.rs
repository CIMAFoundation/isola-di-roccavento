//! The people who talk to the player: the volunteer who guides (opening,
//! tutorial, day 2, the end), the expert who gives the forecast, the fire
//! brigade. Faces and speech bubbles only: what they say comes from the
//! screens, which take it from the game.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    Volontaria,
    Esperto,
    Vigili,
    Sindaca,
}

impl Who {
    pub fn name(self) -> &'static str {
        match self {
            Who::Volontaria => rocca::tr!("La volontaria di protezione civile"),
            Who::Esperto => rocca::tr!("L'esperto di previsioni"),
            Who::Vigili => rocca::tr!("Il caposquadra AIB"),
            Who::Sindaca => rocca::tr!("La sindaca"),
        }
    }

    /// A short label for the team row of the opening screen.
    pub fn short(self) -> &'static str {
        match self {
            Who::Volontaria => rocca::tr!("La volontaria"),
            Who::Esperto => rocca::tr!("L'esperto"),
            Who::Vigili => rocca::tr!("I caposquadra AIB"),
            Who::Sindaca => rocca::tr!("La sindaca"),
        }
    }

    /// The colour of their frame, so they are told apart without reading.
    pub fn colour(self) -> egui::Color32 {
        match self {
            Who::Volontaria => egui::Color32::from_rgb(250, 195, 70),
            Who::Esperto => egui::Color32::from_rgb(110, 185, 255),
            Who::Vigili => egui::Color32::from_rgb(200, 215, 60),
            Who::Sindaca => egui::Color32::from_rgb(120, 205, 140),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Calmo,
    Preoccupato,
}

/// The portraits, compiled in so the browser build has them too.
#[derive(Resource)]
pub struct Portraits(Vec<((Who, Mood), egui::TextureId)>);

impl Portraits {
    pub fn get(&self, who: Who, mood: Mood) -> Option<egui::TextureId> {
        self.0.iter().find(|(k, _)| *k == (who, mood)).map(|(_, t)| *t)
    }
}

/// The CIMA logo, as egui textures: 360 px for the title and the screens, and
/// 1200 px for the opening, where it fills a third of the screen (egui has no
/// mipmaps: one size would be either blurred big or jagged small).
#[derive(Resource)]
pub struct Logo(pub egui::TextureId, pub Option<egui::TextureId>);

/// The scenario screen's icons (`tools/factory/scenario_icons.py`), by case.
#[derive(Resource)]
pub struct ScenarioIcons(pub Vec<(&'static str, egui::TextureId)>);

impl ScenarioIcons {
    pub fn get(&self, case: &str) -> Option<egui::TextureId> {
        self.0.iter().find(|(c, _)| *c == case).map(|(_, t)| *t)
    }
}

fn texture(bytes: &[u8], images: &mut Assets<Image>, contexts: &mut EguiContexts) -> Option<egui::TextureId> {
    let img = Image::from_buffer(
        bytes,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        true,
        bevy::image::ImageSampler::linear(),
        // Not RENDER_WORLD alone: Bevy then drops the image from the main world
        // after upload, and bevy_egui forgets the texture on that removal.
        bevy::asset::RenderAssetUsages::all(),
    )
    .ok()?;
    Some(contexts.add_image(bevy_egui::EguiTextureHandle::Strong(images.add(img))))
}

/// A picture in `assets/`: compiled into the desktop binary, with the data
/// files in the browser (`datafs`), so the wasm stays small (2026-10-30).
macro_rules! picture {
    ($path:expr) => {{
        #[cfg(not(target_arch = "wasm32"))]
        let bytes: std::borrow::Cow<'static, [u8]> = include_bytes!(concat!("../../../../assets/", $path)).as_slice().into();
        #[cfg(target_arch = "wasm32")]
        let bytes: std::borrow::Cow<'static, [u8]> = datafs::take(concat!("assets/", $path)).unwrap_or_default().into();
        bytes
    }};
}

pub fn load_portraits(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut contexts: EguiContexts) {
    macro_rules! png {
        ($f:literal) => {
            picture!(concat!("characters/", $f, ".png"))
        };
    }
    let all: [((Who, Mood), std::borrow::Cow<'static, [u8]>); 8] = [
        ((Who::Volontaria, Mood::Calmo), png!("volontaria_calmo")),
        ((Who::Volontaria, Mood::Preoccupato), png!("volontaria_preoccupato")),
        ((Who::Esperto, Mood::Calmo), png!("meteo_calmo")),
        ((Who::Esperto, Mood::Preoccupato), png!("meteo_preoccupato")),
        ((Who::Vigili, Mood::Calmo), png!("dos_calmo")),
        ((Who::Vigili, Mood::Preoccupato), png!("dos_preoccupato")),
        ((Who::Sindaca, Mood::Calmo), png!("sindaca_calmo")),
        ((Who::Sindaca, Mood::Preoccupato), png!("sindaca_preoccupato")),
    ];
    let out = all.into_iter().filter_map(|(key, bytes)| texture(&bytes, &mut images, &mut contexts).map(|t| (key, t))).collect();
    commands.insert_resource(Portraits(out));
    macro_rules! icon {
        ($f:literal) => {
            ($f, picture!(concat!("scenarios/", $f, ".png")))
        };
    }
    let icons: [(&'static str, std::borrow::Cow<'static, [u8]>); 4] = [
        icon!("medium"),
        icon!("impatti_valico_due_lati_30_6"),
        icon!("impatti_nuclei_valico_costante_lungo_28_7_0_60"),
        icon!("impatti_nuclei_est_giro_lungo_28_6_75_60"),
    ];
    let icons = icons.into_iter().filter_map(|(c, bytes)| texture(&bytes, &mut images, &mut contexts).map(|t| (c, t))).collect();
    commands.insert_resource(ScenarioIcons(icons));
    let logo = picture!("brand/cima_logo_white.png");
    if let Some(t) = texture(&logo, &mut images, &mut contexts) {
        // The browser draws one pixel per CSS pixel (`main.rs`): the opening's
        // logo is never taller than the 360 px one, and the large one cost
        // 4.4 MiB of GPU texture plus its copy in the wasm memory (2026-11-01).
        #[cfg(not(target_arch = "wasm32"))]
        let large = texture(&picture!("brand/cima_logo_white_1200.png"), &mut images, &mut contexts);
        #[cfg(target_arch = "wasm32")]
        let large = None;
        commands.insert_resource(Logo(t, large));
    }
}

/// A portrait in a coloured ring.
pub fn portrait(ui: &mut egui::Ui, portraits: Option<&Portraits>, who: Who, mood: Mood, size: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), size * 0.5, egui::Color32::from_rgb(29, 36, 48));
    if let Some(t) = portraits.and_then(|x| x.get(who, mood)) {
        egui::Image::new(egui::load::SizedTexture::new(t, r.size())).corner_radius(size * 0.5).paint_at(ui, r.shrink(3.0));
    }
    ui.painter().circle_stroke(r.center(), size * 0.5 - 1.5, egui::Stroke::new(4.0, who.colour()));
}

/// The speech bubble: a frame with a tail pointing left at the portrait.
pub fn bubble<R>(ui: &mut egui::Ui, who: Who, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let fill = egui::Color32::from_rgb(245, 245, 240);
    let inner = egui::Frame::NONE.fill(fill).corner_radius(14.0).inner_margin(egui::Margin::symmetric(16, 12)).stroke(egui::Stroke::new(3.0, who.colour())).show(ui, add);
    let r = inner.response.rect;
    let y = r.bottom() - 34.0;
    let tail = vec![egui::pos2(r.left() + 1.5, y - 10.0), egui::pos2(r.left() - 14.0, y + 6.0), egui::pos2(r.left() + 1.5, y + 10.0)];
    ui.painter().add(egui::Shape::convex_polygon(tail.clone(), fill, egui::Stroke::NONE));
    ui.painter().line_segment([tail[0], tail[1]], egui::Stroke::new(3.0, who.colour()));
    ui.painter().line_segment([tail[1], tail[2]], egui::Stroke::new(3.0, who.colour()));
    inner.inner
}

/// Text inside a bubble: dark on light, with the speaker's role on top.
/// `**x**` marks an action (bold, orange), `__x__` a place or thing (bold, blue).
pub fn says(ui: &mut egui::Ui, who: Who, text: &str, width: f32) {
    ui.vertical(|ui| {
        ui.set_max_width(width);
        ui.label(egui::RichText::new(who.name()).size(16.0).strong().color(egui::Color32::from_gray(90)));
        ui.add(egui::Label::new(marked(text, 27.0)).wrap());
    });
}

/// The text as a layout job, with the `**` and `__` marks turned into colour.
fn marked(text: &str, size: f32) -> egui::text::LayoutJob {
    use egui::{text::TextFormat, Color32, FontId};
    let base = Color32::from_rgb(20, 22, 26);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut mode = 0; // 0 plain, 1 action, 2 place
    let mut rest = text;
    while !rest.is_empty() {
        let next = [("**", 1), ("__", 2)].iter().filter_map(|&(m, k)| rest.find(m).map(|i| (i, k))).min();
        let (chunk, tail, toggle) = match next {
            Some((i, k)) => (&rest[..i], &rest[i + 2..], Some(k)),
            None => (rest, "", None),
        };
        let (colour, bold) = match mode {
            1 => (Color32::from_rgb(196, 78, 0), true),
            2 => (Color32::from_rgb(20, 80, 170), true),
            _ => (base, false),
        };
        let mut f = TextFormat { font_id: FontId::proportional(if bold { size + 1.0 } else { size }), color: colour, ..Default::default() };
        if bold {
            f.underline = egui::Stroke::new(1.5, colour.gamma_multiply(0.45));
        }
        job.append(chunk, 0.0, f);
        if let Some(k) = toggle {
            mode = if mode == k { 0 } else { k };
        }
        rest = tail;
    }
    job
}

/// A line said to the player by name, if they gave one:
/// «Il fuoco è sotto controllo» → «Giulia, il fuoco è sotto controllo».
/// The first letter is lowered: pass only lines opening with a common word.
pub fn to(player: &str, text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(f) if !player.is_empty() => format!("{player}, {}", f.to_lowercase().chain(c).collect::<String>()),
        _ => text.to_string(),
    }
}
