//! Graphics quality: what the renderer may spend on a machine weaker than the
//! one the scene was tuned on.
//!
//! The game is GPU-bound (the CPU idles while the render thread waits on the
//! frame), and the cost is the vegetation drawn once per pass — depth
//! prepass, main pass, every shadow cascade — plus full-screen effects
//! (depth of field, bloom, MSAA on an HDR target). Each knob here removes a
//! pass or shrinks one; none changes the simulation, the picking or what the
//! UI shows.
//!
//! `KIOSK_QUALITY=alta|media|bassa` picks a preset (default `alta`, the look
//! as authored). `KIOSK_GFX` then overrides single knobs, for tuning on the
//! kiosk machine: `KIOSK_GFX=-dof,-bloom,msaa=1,cascades=2,shadow=1500,veg=0.3,lod=800`.
//!
//! Extra switches for A/B tests of the GPU cost (defaults leave the look as it
//! is in every preset): `-terrain_shadows` stops the terrain casting shadows
//! (it still receives them; applies live), `terrain_stride=2|4` draws the native
//! terrain at every 2nd/4th sample (applies from the next launch), e.g.
//! `KIOSK_FPS=1 KIOSK_GFX=-terrain_shadows,terrain_stride=2`.
//!
//! Antialiasing: `alta` MSAA 4×, `media` SMAA, `bassa` none; the operator bar
//! chooses it apart too (saved in `~/.rocca_ventosa/antialiasing.txt`, dropped
//! when a level is picked), `KIOSK_GFX=smaa,msaa=1` from the environment.
//!
//! The operator bar (F2) also picks the preset, live and saved in
//! `~/.rocca_ventosa/grafica.txt` (`KIOSK_QUALITY` still wins). Everything
//! changes at once except the share of plants planted and the radius of the
//! near ground cover (`cover=<m>`, `-cover`), which are drawn when
//! the scene is built: it applies from the next launch.

use bevy::prelude::*;

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Graphics {
    /// Depth of field (and the depth prepass that only it needs).
    pub dof: bool,
    pub bloom: bool,
    /// MSAA samples: 4 or 1 (off).
    pub msaa: u32,
    /// SMAA, a post-process edge filter: the cheaper alternative to MSAA.
    pub smaa: bool,
    /// Sun shadows at all.
    pub shadows: bool,
    /// Shadow cascades and how far they reach, in metres.
    pub cascades: usize,
    pub shadow_distance: f32,
    /// Shadow map texels per cascade side.
    pub shadow_map: usize,
    /// Share of the authored plant densities that is planted.
    pub vegetation: f32,
    /// Distance from the camera (metres) beyond which plants are drawn as
    /// proxies of a couple of dozen faces (see `vegetation`).
    pub lod_m: f32,
    /// The terrain casts shadows (it always receives them).
    pub terrain_shadows: bool,
    /// Terrain samples per render-mesh vertex of the finest level (1 = every
    /// sample); the colour and normal textures have one texel per such vertex.
    /// Below 4, chunks beyond `terrain_mesh::LOD1_M`/`LOD2_M` use 2x and 4x that
    /// stride. Read when the scene is built: applies from the next launch.
    pub terrain_stride: usize,
    /// Radius (metres) of the grass layer drawn around the camera by the GPU;
    /// shrubs reach 4x as far (see `near_cover`); 0 = none. Read when the
    /// scene is built.
    pub cover_m: f32,
    /// Share of the plants built that are shown; changes while the game runs
    /// (`adaptive` thins the forest before it takes the shadows away).
    pub veg_keep: f32,
    /// The plants' stand-ins are their crown parts as lumps, painted by the
    /// shader (`vegetation::meshes::lobed_mesh`), rather than one six-sided
    /// crown: the browser's, which draws no leaf cards. Read when the scene
    /// is built.
    pub lobes: bool,
}

impl Graphics {
    /// As authored. On the M4 Pro this was tuned on: about 45 FPS before the
    /// terrain, vegetation and smoke LODs of 2026-10-20, ~6–10 ms of GPU after.
    pub const ALTA: Graphics = Graphics {
        dof: true,
        bloom: true,
        msaa: 4,
        smaa: false,
        shadows: true,
        cascades: 4,
        shadow_distance: 4000.0,
        shadow_map: 2048,
        vegetation: if cfg!(target_arch = "wasm32") { 0.12 } else { 0.5 },
        lod_m: 1400.0,
        terrain_shadows: true,
        terrain_stride: 1,
        cover_m: if cfg!(target_arch = "wasm32") { 0.0 } else { 45.0 },
        veg_keep: 1.0,
        lobes: false,
    };
    pub const MEDIA: Graphics = Graphics {
        dof: false,
        // SMAA instead of MSAA: almost free, edges nearly as clean (2026-10-20)
        msaa: 1,
        smaa: true,
        cascades: 3,
        shadow_distance: 2500.0,
        vegetation: if cfg!(target_arch = "wasm32") { 0.12 } else { 0.35 },
        lod_m: 1000.0,
        cover_m: if cfg!(target_arch = "wasm32") { 0.0 } else { 35.0 },
        ..Graphics::ALTA
    };
    pub const BASSA: Graphics = Graphics {
        dof: false,
        bloom: false,
        msaa: 1,
        cascades: 2,
        shadow_distance: 1800.0,
        shadow_map: 1024,
        vegetation: if cfg!(target_arch = "wasm32") { 0.08 } else { 0.2 },
        lod_m: 600.0,
        cover_m: if cfg!(target_arch = "wasm32") { 0.0 } else { 22.0 },
        ..Graphics::ALTA
    };

    /// Native machines short of memory (`memory`): «bassa» on a scene built
    /// lighter, near the browser's: terrain at every 2nd sample (road mask
    /// 4 m), a tenth of the plants, no near grass. Measured 2026-10-29 at
    /// 1280x800, T+90: resident 2.5 GB «alta», 1.7 GB «bassa», 0.96 GB this.
    #[cfg(not(target_arch = "wasm32"))]
    pub const RISPARMIO: Graphics = Graphics {
        vegetation: 0.1, terrain_stride: 2, cover_m: 0.0,
        ..Graphics::BASSA
    };

    /// What is built at launch from `RISPARMIO`, the rest kept.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn lighter(self) -> Graphics {
        let r = Graphics::RISPARMIO;
        Graphics {
            vegetation: self.vegetation.min(r.vegetation),
            terrain_stride: self.terrain_stride.max(r.terrain_stride),
            cover_m: self.cover_m.min(r.cover_m),
            ..self
        }
    }

    /// The same level with what was built at launch kept (it cannot change
    /// until the next launch, and a new town rebuilds the scene from it).
    pub fn built_as(self, now: &Graphics) -> Graphics {
        Graphics { vegetation: now.vegetation, terrain_stride: now.terrain_stride, cover_m: now.cover_m, lobes: now.lobes, ..self }
    }

    /// Browser budget, also on desktop: no post-processing or shadow passes.
    /// The plants as lobed stand-ins (`lobes`), twice as many as until
    /// 2026-11-02 (0.025).
    pub const WEB: Graphics = Graphics {
        shadows: false, shadow_map: 256, cascades: 1,
        vegetation: 0.05, lod_m: 0.0, terrain_stride: 4, cover_m: 0.0, lobes: true,
        ..Graphics::BASSA
    };

    /// Browser on a computer: the same as `WEB` plus sun shadows (two cascades)
    /// and a finer terrain and more plants. Still no HDR, DoF, bloom or MSAA.
    pub const WEB_MEDIA: Graphics = Graphics {
        shadows: true, shadow_map: 1024, cascades: 2, shadow_distance: 1800.0,
        vegetation: 0.08, terrain_stride: 2,
        ..Graphics::WEB
    };

    /// The top of the browser's adaptive ladder on a computer: `WEB_MEDIA` with
    /// the shadows of `MEDIA` (three cascades, 2048) and twice the plants;
    /// `adaptive` brings it down (the plants are hidden, not unbuilt, and not
    /// drawn: `vegetation::PlantScene::thin_by_count`).
    pub const WEB_TOP: Graphics = Graphics {
        cascades: 3, shadow_distance: 2500.0, shadow_map: 2048, vegetation: 0.14,
        ..Graphics::WEB_MEDIA
    };

    /// The preset from `KIOSK_QUALITY`, then the `KIOSK_GFX` overrides, then
    /// the old `KIOSK_VEG_DENSITY` (which still wins for the vegetation).
    /// In the browser the page decides (`window.roccaProfile`, see `web/index.html`).
    pub fn from_env() -> Graphics {
        if cfg!(target_arch = "wasm32") {
            let mut g = match web_profile().as_deref() {
                Some("media") if Graphics::auto_wanted() => Graphics::WEB_TOP,
                Some("media") => Graphics::WEB_MEDIA,
                _ => Graphics::WEB,
            };
            // `?gfx=` as `KIOSK_GFX`, e.g. `veg=0,terrain_stride=8`
            if let Some(spec) = web_param("gfx") {
                g.apply(&spec);
            }
            return g;
        }
        let name = std::env::var("KIOSK_QUALITY").ok().or_else(saved);
        let mut g = match name.as_deref() {
            // "auto" starts from the top; `adaptive` brings it down if it must
            Some("bassa" | "low") => Graphics::BASSA,
            Some("media" | "medium") => Graphics::MEDIA,
            _ => Graphics::ALTA,
        };
        // the antialiasing chosen apart in the operator bar, over the preset's
        if let Some(aa) = saved_aa() {
            g.set_aa(&aa);
        }
        if let Ok(spec) = std::env::var("KIOSK_GFX") {
            g.apply(&spec);
        }
        if let Some(v) = std::env::var("KIOSK_VEG_DENSITY").ok().and_then(|v| v.parse().ok()) {
            g.vegetation = v;
        }
        g
    }

    /// Whether the level follows the machine (`adaptive`): the default, unless a
    /// level was chosen by hand (`KIOSK_QUALITY`, `KIOSK_GFX`, a level saved from
    /// the operator bar, `?q=` in the browser), a snapshot is being taken, or
    /// `KIOSK_ADAPT=0`.
    /// A level was chosen by hand (the environment or F2), not left to `adaptive`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn chosen() -> bool {
        ["KIOSK_QUALITY", "KIOSK_GFX", "KIOSK_VEG_DENSITY"].iter().any(|k| std::env::var_os(k).is_some())
            || !matches!(saved().as_deref(), None | Some("auto"))
    }

    pub fn auto_wanted() -> bool {
        if cfg!(target_arch = "wasm32") {
            return !web_forced();
        }
        let off = |k: &str| std::env::var_os(k).is_some();
        if off("KIOSK_QUALITY") || off("KIOSK_GFX") || off("KIOSK_SNAP") || std::env::var("KIOSK_ADAPT").as_deref() == Ok("0") {
            return false;
        }
        matches!(saved().as_deref(), None | Some("auto"))
    }

    fn apply(&mut self, spec: &str) {
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (on, key) = match item.strip_prefix('-') {
                Some(k) => (false, k),
                None => (true, item.strip_prefix('+').unwrap_or(item)),
            };
            let (key, value) = key.split_once('=').unwrap_or((key, ""));
            let num = value.parse::<f32>().ok();
            match (key, num) {
                ("dof", _) => self.dof = on,
                ("bloom", _) => self.bloom = on,
                ("smaa", _) => self.smaa = on,
                ("shadows", _) => self.shadows = on,
                ("terrain_shadows", _) => self.terrain_shadows = on,
                ("terrain_stride", Some(n)) => self.terrain_stride = (n as usize).clamp(1, 8),
                ("lod", Some(m)) => self.lod_m = m.max(0.0),
                ("msaa", Some(n)) => self.msaa = if n >= 4.0 { 4 } else { 1 },
                ("msaa", None) => self.msaa = if on { 4 } else { 1 },
                ("cascades", Some(n)) => self.cascades = (n as usize).clamp(1, 4),
                ("shadow", Some(m)) => self.shadow_distance = m.max(100.0),
                ("shadow_map", Some(n)) => self.shadow_map = (n as usize).clamp(256, 4096),
                ("veg", Some(v)) => self.vegetation = v.max(0.0),
                ("cover", Some(m)) => self.cover_m = m.max(0.0),
                ("cover", None) => self.cover_m = if on { Graphics::ALTA.cover_m } else { 0.0 },
                ("lobes", _) => self.lobes = on,
                _ => warn!("KIOSK_GFX: «{item}» non riconosciuto"),
            }
        }
    }

    /// The preset this is, if it is one (the operator bar's selection).
    pub fn level(&self) -> Option<&'static str> {
        // the plant density and the terrain stride apply only at launch, and
        // the antialiasing is chosen apart, so they are not compared
        let same = |p: &Graphics| {
            Graphics { vegetation: self.vegetation, terrain_stride: self.terrain_stride, cover_m: self.cover_m, veg_keep: self.veg_keep, lobes: self.lobes, msaa: self.msaa, smaa: self.smaa, ..*p } == *self
        };
        [("alta", &Graphics::ALTA), ("media", &Graphics::MEDIA), ("bassa", &Graphics::BASSA)]
            .into_iter()
            .find(|(_, p)| same(p))
            .map(|(n, _)| n)
    }

    /// The antialiasing: `msaa` (4 samples), `smaa` or `nessuno`.
    pub fn aa(&self) -> &'static str {
        if self.msaa >= 4 { "msaa" } else if self.smaa { "smaa" } else { "nessuno" }
    }

    pub fn set_aa(&mut self, aa: &str) {
        match aa {
            "msaa" => (self.msaa, self.smaa) = (4, false),
            "smaa" => (self.msaa, self.smaa) = (1, true),
            "nessuno" => (self.msaa, self.smaa) = (1, false),
            _ => {}
        }
    }

    pub fn msaa(&self) -> Msaa {
        // WebGL2 has no multisampled HDR targets.
        if cfg!(target_arch = "wasm32") || self.msaa < 4 { Msaa::Off } else { Msaa::Sample4 }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".rocca_ventosa").join("grafica.txt")
}

#[cfg(not(target_arch = "wasm32"))]
fn saved() -> Option<String> {
    std::fs::read_to_string(saved_path()).ok().map(|s| s.trim().to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn aa_path() -> std::path::PathBuf {
    saved_path().with_file_name("antialiasing.txt")
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_aa() -> Option<String> {
    std::fs::read_to_string(aa_path()).ok().map(|s| s.trim().to_string())
}

/// Remember the antialiasing for the next launch.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_aa(aa: &str) -> Result<(), String> {
    let p = aa_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("grafica: {e}"))?;
    }
    std::fs::write(&p, aa).map_err(|e| format!("grafica: {e}"))
}

/// Back to the level's own antialiasing.
#[cfg(not(target_arch = "wasm32"))]
pub fn forget_aa() {
    let _ = std::fs::remove_file(aa_path());
}

// WebGL2 has neither multisampled HDR targets nor a choice in the page.
#[cfg(target_arch = "wasm32")]
fn saved_aa() -> Option<String> {
    None
}

/// `bassa` or `media`, as the page decided from the device (or from `?q=`).
#[cfg(target_arch = "wasm32")]
fn web_profile() -> Option<String> {
    js_sys::Reflect::get(&js_sys::global(), &"roccaProfile".into()).ok()?.as_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn web_profile() -> Option<String> {
    None
}

/// A parameter of the page's address (`?gfx=veg=0`, see [`Graphics::from_env`]).
/// Always `None` on the desktop.
#[cfg(target_arch = "wasm32")]
fn web_param(name: &str) -> Option<String> {
    let location = js_sys::Reflect::get(&js_sys::global(), &"location".into()).ok()?;
    let search = js_sys::Reflect::get(&location, &"search".into()).ok()?.as_string()?;
    let search = js_sys::decode_uri_component(&search).ok()?.as_string()?;
    search.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn web_param(_: &str) -> Option<String> {
    None
}

/// The page forced a profile with `?q=`.
#[cfg(target_arch = "wasm32")]
fn web_forced() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &"roccaProfileForced".into()).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn web_forced() -> bool {
    false
}

/// The adaptive rung the last run settled on (0 = the top).
#[cfg(not(target_arch = "wasm32"))]
pub fn saved_step() -> Option<usize> {
    std::fs::read_to_string(saved_path().with_file_name("grafica_auto.txt")).ok()?.trim().parse().ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_step(step: usize) {
    let p = saved_path().with_file_name("grafica_auto.txt");
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, step.to_string());
}

#[cfg(target_arch = "wasm32")]
pub fn saved_step() -> Option<usize> {
    web_sys::window()?.local_storage().ok()??.get_item("rocca_ventosa_grafica_auto").ok()??.parse().ok()
}

#[cfg(target_arch = "wasm32")]
pub fn save_step(step: usize) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item("rocca_ventosa_grafica_auto", &step.to_string());
    }
}

#[cfg(target_arch = "wasm32")]
fn saved() -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item("rocca_ventosa_grafica").ok()?
}

/// Remember the level for the next launch. A failure is reported, never fatal.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(level: &str) -> Result<(), String> {
    crate::memory::forget();
    let p = saved_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("grafica: {e}"))?;
    }
    std::fs::write(&p, level).map_err(|e| format!("grafica: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn save(level: &str) -> Result<(), String> {
    let s = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or("grafica: il browser non salva dati")?;
    s.set_item("rocca_ventosa_grafica", level).map_err(|_| "grafica: salvataggio non riuscito".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_switches_parse_and_presets_keep_the_look() {
        for p in [Graphics::ALTA, Graphics::MEDIA, Graphics::BASSA] {
            assert!(p.terrain_shadows);
            assert_eq!(p.terrain_stride, 1);
        }
        assert_eq!(Graphics::WEB.terrain_stride, 4);
        assert!(Graphics::WEB_MEDIA.shadows && !Graphics::WEB.shadows);
        assert!(Graphics::WEB_MEDIA.terrain_stride < Graphics::WEB.terrain_stride);
        let mut g = Graphics::ALTA;
        g.apply("-terrain_shadows,terrain_stride=2");
        assert!(!g.terrain_shadows);
        assert_eq!(g.terrain_stride, 2);
        assert_eq!(g.level(), None);
        g.terrain_shadows = true;
        assert_eq!(g.level(), Some("alta")); // stride is launch-only
        g.apply("terrain_stride=0");
        assert_eq!(g.terrain_stride, 1);
        g.apply("+terrain_shadows");
        assert!(g.terrain_shadows);
    }

    #[test]
    fn the_lighter_scene_changes_only_what_is_built_at_launch() {
        assert_eq!(Graphics::RISPARMIO.level(), Some("bassa"));
        let l = Graphics::ALTA.lighter();
        assert_eq!(l.level(), Some("alta"));
        assert_eq!((l.terrain_stride, l.vegetation, l.cover_m), (2, 0.1, 0.0));
        assert_eq!(Graphics::MEDIA.built_as(&l).level(), Some("media"));
        assert_eq!(Graphics::MEDIA.built_as(&l).terrain_stride, 2);
    }
}
