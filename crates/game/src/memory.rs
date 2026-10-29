//! Native machines short of memory. The scene built at launch (terrain mesh,
//! road mask, plants, near grass) is most of the game's memory, and the
//! adaptive level cannot give it back: it only hides plants. So the launch
//! decides, from two signs, whether to build the lighter scene of
//! `quality::Graphics::RISPARMIO`:
//!
//! - the machine has at most `LOW_RAM_GIB` of RAM, and nobody chose a level;
//! - the previous launch did not end (`avvio.txt` is left behind by a crash,
//!   an out-of-memory kill or a frozen window closed by force). This is
//!   remembered in `memoria.txt` until a level is chosen in F2.
//!
//! `KIOSK_MEMORY=risparmio|normale` forces either way (tests, or by hand).
//! Never in the browser (the page chooses) nor with `KIOSK_SNAP`.

use std::path::PathBuf;

/// At most this much RAM: the lighter scene (2026-10-29: «alta» peaks at
/// ~2.3 GB resident at 1280x800, more with the GPU's share on unified memory).
const LOW_RAM_GIB: f64 = 8.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Short {
    /// Little RAM: build less, the live level stays the player's or the adaptive one.
    Ram,
    /// The last launch did not end: build less and start from «bassa».
    Crash,
}

fn dir() -> PathBuf {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".rocca_ventosa")
}

fn total_ram_gib() -> Option<f64> {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
    let bytes = sys.total_memory();
    (bytes > 0).then(|| bytes as f64 / (1u64 << 30) as f64)
}

/// Called once at launch, before the scene is built: why to build less, if
/// at all. Leaves the marker that `end` removes.
pub fn begin(level_chosen: bool) -> Option<Short> {
    if std::env::var_os("KIOSK_SNAP").is_some() {
        return None;
    }
    let short = decide(&dir(), level_chosen, std::env::var("KIOSK_MEMORY").ok().as_deref(), total_ram_gib);
    match short {
        Some(Short::Crash) => bevy::log::warn!("memoria: avvio precedente non terminato, scena leggera e grafica bassa"),
        Some(Short::Ram) => bevy::log::info!("memoria: poca RAM, scena leggera"),
        None => {}
    }
    short
}

fn decide(dir: &std::path::Path, level_chosen: bool, forced: Option<&str>, ram_gib: impl Fn() -> Option<f64>) -> Option<Short> {
    let marker = dir.join("avvio.txt");
    // a marker whose game is still running is another window, not a crash
    let crashed = std::fs::read_to_string(&marker).is_ok_and(|pid| !running(pid.trim().parse().unwrap_or(0)));
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&marker, std::process::id().to_string());
    match forced {
        Some("risparmio") => return Some(Short::Crash),
        Some("normale") => return None,
        _ => {}
    }
    let sticky = dir.join("memoria.txt");
    if crashed {
        let _ = std::fs::write(&sticky, "risparmio");
    }
    if sticky.exists() {
        return Some(Short::Crash);
    }
    let ram = ram_gib();
    bevy::log::info!("memoria: {:.1} GiB di RAM", ram.unwrap_or(0.0));
    (!level_chosen && ram.is_some_and(|g| g <= LOW_RAM_GIB)).then_some(Short::Ram)
}

#[cfg(unix)]
fn running(pid: i32) -> bool {
    pid > 0 && pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } == 0
}

#[cfg(not(unix))]
fn running(_pid: i32) -> bool {
    false
}

/// The game ended on its own: the next launch is not a recovery.
pub fn end() {
    end_in(&dir());
}

fn end_in(dir: &std::path::Path) {
    let _ = std::fs::remove_file(dir.join("avvio.txt"));
}

/// A level chosen in F2 replaces the recovery's.
pub fn forget() {
    let _ = std::fs::remove_file(dir().join("memoria.txt"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_that_did_not_end_makes_the_next_ones_light_until_a_level_is_chosen() {
        let dir = std::env::temp_dir().join(format!("rocca_memoria_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let big = || Some(48.0);
        assert_eq!(decide(&dir, false, None, big), None);
        end_in(&dir);
        assert_eq!(decide(&dir, false, None, big), None, "a clean exit is no crash");
        // no `end`: the marker stays, as after a kill
        assert_eq!(decide(&dir, false, None, big), Some(Short::Crash));
        end_in(&dir);
        assert_eq!(decide(&dir, true, None, big), Some(Short::Crash), "remembered");
        end_in(&dir);
        let _ = std::fs::remove_file(dir.join("memoria.txt")); // `forget`, from F2
        assert_eq!(decide(&dir, false, None, big), None);
        end_in(&dir);
        assert_eq!(decide(&dir, false, Some("normale"), || Some(4.0)), None);
        end_in(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn little_ram_lightens_only_a_level_left_to_the_machine() {
        let dir = std::env::temp_dir().join(format!("rocca_memoria_ram_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(decide(&dir, false, None, || Some(8.0)), Some(Short::Ram));
        end_in(&dir);
        assert_eq!(decide(&dir, true, None, || Some(8.0)), None);
        end_in(&dir);
        assert_eq!(decide(&dir, false, None, || Some(16.0)), None);
        end_in(&dir);
        assert_eq!(decide(&dir, false, None, || None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
