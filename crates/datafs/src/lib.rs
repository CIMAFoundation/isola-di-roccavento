//! Reading the game's data files: from disk natively, and in the browser from
//! `globalThis.roccaData`, an object that maps `data/...` paths to `Uint8Array`s
//! which the page downloads (and caches) before the wasm starts. The browser
//! inflates them itself when it can; when `globalThis.roccaDataGzip` is true the
//! bytes are still gzipped and are inflated here, on each read. The data is not
//! compiled into the binary, so a new build does not make anyone download it
//! again. Paths are the same in both: `data/...`.

use std::io;
use std::path::{Path, PathBuf};

/// `./data/x/../y` as `data/y`, with forward slashes: the key in `roccaData`.
#[cfg(any(target_arch = "wasm32", test))]
fn key(p: &Path) -> String {
    let mut parts: Vec<String> = vec![];
    for c in p.components() {
        match c {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    parts.join("/")
}

#[cfg(target_arch = "wasm32")]
fn not_found(p: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} not found", p.display()))
}

#[cfg(target_arch = "wasm32")]
mod store {
    use js_sys::{Object, Reflect, Uint8Array};
    use std::io::Read;
    use wasm_bindgen::JsCast;

    fn files() -> Option<Object> {
        Reflect::get(&js_sys::global(), &"roccaData".into()).ok()?.dyn_into().ok()
    }

    pub fn get(key: &str) -> Option<Vec<u8>> {
        let value = Reflect::get(&files()?.into(), &key.into()).ok()?;
        let bytes = value.dyn_into::<Uint8Array>().ok()?.to_vec();
        let gzipped = Reflect::get(&js_sys::global(), &"roccaDataGzip".into()).ok()?.is_truthy();
        if !gzipped {
            return Some(bytes);
        }
        // gzip ends with the original size (modulo 2^32). Reserving it avoids
        // repeatedly growing and copying a 10 MB terrain buffer on restarts.
        // Bound the hint: the decoder still validates the stream and can grow.
        let size = bytes.get(bytes.len().checked_sub(4)?..)?;
        let hint = u32::from_le_bytes(size.try_into().ok()?) as usize;
        let mut plain = Vec::with_capacity(hint.min(64 * 1024 * 1024));
        flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut plain).ok()?;
        Some(plain)
    }

    pub fn forget(key: &str) {
        if let Some(f) = files() {
            let _ = Reflect::delete_property(&f, &key.into());
        }
    }

    pub fn keys() -> Vec<String> {
        files().map(|f| Object::keys(&f).iter().filter_map(|k| k.as_string()).collect()).unwrap_or_default()
    }
}

/// Whether the data comes from the page instead of the disk (the browser build).
pub fn is_embedded() -> bool {
    cfg!(target_arch = "wasm32")
}

pub fn read(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let p = p.as_ref();
    #[cfg(target_arch = "wasm32")]
    {
        store::get(&key(p)).ok_or_else(|| not_found(p))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(p)
    }
}

/// [`read`], for a file needed once: in the browser the page's copy is dropped
/// too, so its memory can go.
pub fn take(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let bytes = read(&p)?;
    #[cfg(target_arch = "wasm32")]
    store::forget(&key(p.as_ref()));
    Ok(bytes)
}

pub fn read_to_string(p: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn is_dir(p: impl AsRef<Path>) -> bool {
    let p = p.as_ref();
    #[cfg(target_arch = "wasm32")]
    {
        let k = key(p) + "/";
        store::keys().iter().any(|f| f.starts_with(&k))
    }
    #[cfg(not(target_arch = "wasm32"))]
    p.is_dir()
}

/// The files directly inside `dir`, sorted.
pub fn files_in(dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    #[cfg(target_arch = "wasm32")]
    let mut out: Vec<PathBuf> = {
        let k = key(dir) + "/";
        store::keys()
            .iter()
            .filter_map(|f| f.strip_prefix(&k).filter(|rest| !rest.contains('/')).map(|rest| dir.join(rest)))
            .collect()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let mut out: Vec<PathBuf> =
        std::fs::read_dir(dir)?.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).map(|e| e.path()).collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_normalised() {
        assert_eq!(key(Path::new("./data/scenarios/../behaviours/graphs/a.json")), "data/behaviours/graphs/a.json");
    }
}
