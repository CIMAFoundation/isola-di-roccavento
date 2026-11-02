//! Native machines short of memory. The scene built at launch (terrain mesh,
//! road mask, plants, near grass) is most of the game's memory, and the
//! adaptive level cannot give it back: it only hides plants. So the launch
//! builds the lighter scene of `quality::Graphics::RISPARMIO` when the machine
//! has at most `LOW_RAM_GIB` of RAM and nobody chose a level.
//!
//! `KIOSK_MEMORY=risparmio|normale` forces either way (tests, or by hand).
//! Never in the browser (the page chooses) nor with `KIOSK_SNAP`.
//!
//! Until 2026-11-02 a launch that did not end (a crash, a kill) also made the
//! next ones light, until a level was chosen in F2: removed at the user's
//! request, since any panic set it off, memory or not.

/// At most this much RAM: the lighter scene (2026-10-29: «alta» peaks at
/// ~2.3 GB resident at 1280x800, more with the GPU's share on unified memory).
const LOW_RAM_GIB: f64 = 8.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Short {
    /// Little RAM: build less, the live level stays the player's or the adaptive one.
    Ram,
    /// `KIOSK_MEMORY=risparmio`: build less and start from «bassa».
    Forced,
}

fn total_ram_gib() -> Option<f64> {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
    let bytes = sys.total_memory();
    (bytes > 0).then(|| bytes as f64 / (1u64 << 30) as f64)
}

/// Called once at launch, before the scene is built: why to build less, if at all.
pub fn begin(level_chosen: bool) -> Option<Short> {
    if std::env::var_os("KIOSK_SNAP").is_some() {
        return None;
    }
    let short = decide(level_chosen, std::env::var("KIOSK_MEMORY").ok().as_deref(), total_ram_gib);
    match short {
        Some(Short::Forced) => bevy::log::info!("memoria: KIOSK_MEMORY=risparmio, scena leggera e grafica bassa"),
        Some(Short::Ram) => bevy::log::info!("memoria: poca RAM, scena leggera"),
        None => {}
    }
    short
}

fn decide(level_chosen: bool, forced: Option<&str>, ram_gib: impl Fn() -> Option<f64>) -> Option<Short> {
    match forced {
        Some("risparmio") => return Some(Short::Forced),
        Some("normale") => return None,
        _ => {}
    }
    let ram = ram_gib();
    bevy::log::info!("memoria: {:.1} GiB di RAM", ram.unwrap_or(0.0));
    (!level_chosen && ram.is_some_and(|g| g <= LOW_RAM_GIB)).then_some(Short::Ram)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_ram_lightens_only_a_level_left_to_the_machine() {
        assert_eq!(decide(false, None, || Some(8.0)), Some(Short::Ram));
        assert_eq!(decide(true, None, || Some(8.0)), None);
        assert_eq!(decide(false, None, || Some(16.0)), None);
        assert_eq!(decide(false, None, || None), None);
    }

    #[test]
    fn the_variable_forces_either_way() {
        assert_eq!(decide(true, Some("risparmio"), || Some(48.0)), Some(Short::Forced));
        assert_eq!(decide(false, Some("normale"), || Some(4.0)), None);
    }
}
