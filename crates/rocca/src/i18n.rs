//! Localization (user, 2026-10-21): every text the player reads is written in
//! Italian in the code, and that Italian text is the key. A catalog is a flat
//! JSON object `data/i18n/<code>.json` that maps an Italian text to its
//! translation; Italian has no catalog and reads the key itself, so the
//! default language costs nothing and is inert. A text with no entry in the
//! chosen catalog is shown in Italian.
//!
//! Texts with values use numbered holes, `{0}`, `{1}`, so a translation may
//! reorder them: `tr!("{0} è fuori servizio", name)`. Format specs are applied
//! to the value before it goes in (`format!("{:.0}", x)`).
//!
//! The language is read when a text is made: a log line or a chat message
//! keeps the language it was written in. It is chosen on the first screen,
//! before a game starts. Proper nouns (districts, teams) are not translated.

use std::collections::HashMap;
use std::sync::RwLock;

/// `(code, name in its own language)`: the languages of Europe the game speaks,
/// in the order the picker shows them. `tlh` (Klingon) is not here: it is an
/// easter egg, see [`EASTER_EGG`].
pub const LANGS: &[(&str, &str)] = &[
    ("it", "Italiano"),
    ("en", "English"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("pt", "Português"),
    ("ca", "Català"),
    ("gl", "Galego"),
    ("eu", "Euskara"),
    ("nl", "Nederlands"),
    ("lb", "Lëtzebuergesch"),
    ("da", "Dansk"),
    ("sv", "Svenska"),
    ("nb", "Norsk"),
    ("is", "Íslenska"),
    ("fi", "Suomi"),
    ("et", "Eesti"),
    ("lv", "Latviešu"),
    ("lt", "Lietuvių"),
    ("pl", "Polski"),
    ("cs", "Čeština"),
    ("sk", "Slovenčina"),
    ("hu", "Magyar"),
    ("ro", "Română"),
    ("bg", "Български"),
    ("el", "Ελληνικά"),
    ("hr", "Hrvatski"),
    ("sl", "Slovenščina"),
    ("sr", "Srpski"),
    ("bs", "Bosanski"),
    ("mk", "Македонски"),
    ("sq", "Shqip"),
    ("mt", "Malti"),
    ("ga", "Gaeilge"),
    ("cy", "Cymraeg"),
    ("uk", "Українська"),
    ("be", "Беларуская"),
    ("ru", "Русский"),
    ("tr", "Türkçe"),
];

/// The hidden language: Klingon, in the Latin transliteration (tlhIngan Hol).
pub const EASTER_EGG: (&str, &str) = ("tlh", "tlhIngan Hol");

type Catalog = HashMap<&'static str, &'static str>;

struct State {
    code: &'static str,
    catalog: &'static Catalog,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

/// The language in use (`"it"` until [`set`] is called).
pub fn code() -> &'static str {
    STATE.read().ok().and_then(|s| s.as_ref().map(|s| s.code)).unwrap_or("it")
}

/// The code of a supported language (the easter egg included) from a locale
/// tag such as `de-AT`, `pt_BR.UTF-8` or `NB`; `None` when it is not one.
pub fn parse(tag: &str) -> Option<&'static str> {
    let base = tag.split(['-', '_', '.', '@']).next()?.trim().to_ascii_lowercase();
    // «no», «nn» are Norwegian; «sh» is Serbo-Croatian; «ltz» is Luxembourgish.
    let base = match base.as_str() {
        "no" | "nn" => "nb",
        "ltz" => "lb",
        "ie" => "ga",
        b => b,
    };
    LANGS.iter().chain([&EASTER_EGG]).map(|(c, _)| *c).find(|c| *c == base)
}

/// The first language the game speaks in a list of locale tags in order of
/// preference (the browser's `navigator.languages`, the system's languages):
/// `["ja-JP", "de-AT", "en"]` gives `de`. The easter egg is never picked here.
pub fn first<S: AsRef<str>>(tags: impl IntoIterator<Item = S>) -> Option<&'static str> {
    tags.into_iter().filter_map(|t| parse(t.as_ref())).find(|c| *c != EASTER_EGG.0)
}

/// Where the data folder is (`ROCCA_DATA`, default `data`): the catalogs are in `<root>/i18n/`.
static ROOT: RwLock<Option<std::path::PathBuf>> = RwLock::new(None);

/// Tells where the data folder is. Call before [`set`] when it is not `data`.
pub fn set_root(root: impl Into<std::path::PathBuf>) {
    if let Ok(mut r) = ROOT.write() {
        *r = Some(root.into());
    }
}

fn catalog_path(code: &str) -> std::path::PathBuf {
    let root = ROOT.read().ok().and_then(|r| r.clone()).unwrap_or_else(|| "data".into());
    root.join("i18n").join(format!("{code}.json"))
}

/// Chooses the language. An unknown code or a missing or broken catalog leaves
/// Italian, never a crash. Returns the language now in use.
pub fn set(code: &str) -> &'static str {
    let Some(code) = parse(code) else { return self::code() };
    if code == "it" {
        if let Ok(mut s) = STATE.write() {
            *s = None;
        }
        return "it";
    }
    let loaded: Option<Catalog> = datafs::read(catalog_path(code))
        .ok()
        .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
        .map(|m| m.into_iter().map(|(k, v)| (&*Box::leak(k.into_boxed_str()), &*Box::leak(v.into_boxed_str()))).collect());
    match loaded {
        Some(c) => {
            if let Ok(mut s) = STATE.write() {
                *s = Some(State { code, catalog: Box::leak(Box::new(c)) });
            }
            code
        }
        None => self::code(),
    }
}

/// The text in the language in use; the key itself (Italian) when there is no
/// translation.
pub fn tr(key: &'static str) -> &'static str {
    let Ok(s) = STATE.read() else { return key };
    match s.as_ref() {
        Some(s) => s.catalog.get(key).copied().unwrap_or(key),
        None => key,
    }
}

/// A text that comes from the data, not from the code (the scenarios' names
/// and descriptions in `game.json` → `levels`): translated like [`tr`], the
/// Italian itself when there is no translation. `tools/i18n/keys.py` takes
/// these keys from the data too.
pub fn data(key: &str) -> &str {
    let Ok(s) = STATE.read() else { return key };
    match s.as_ref() {
        Some(s) => s.catalog.get(key).copied().unwrap_or(key),
        None => key,
    }
}

/// [`tr`] with the holes `{0}`, `{1}`… filled.
pub fn fmt(key: &'static str, args: &[String]) -> String {
    let mut out = tr(key).to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// `tr!("testo")` is the translated text (`&'static str`);
/// `tr!("{0} verso {1}", a, b)` is a `String` with the values in the holes.
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::tr($key)
    };
    ($key:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::fmt($key, &[$(($arg).to_string()),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags() {
        assert_eq!(parse("de-AT"), Some("de"));
        assert_eq!(parse("pt_BR.UTF-8"), Some("pt"));
        assert_eq!(parse("NO"), Some("nb"));
        assert_eq!(parse("tlh"), Some("tlh"));
        assert_eq!(parse("ja"), None);
    }

    #[test]
    fn first_supported_of_the_preferred() {
        assert_eq!(first(["ja-JP", "de-AT", "en"]), Some("de"));
        assert_eq!(first(["en-GB", "it"]), Some("en"));
        assert_eq!(first(["zh-CN", "nn-NO"]), Some("nb"));
        assert_eq!(first(["tlh", "fr-CA"]), Some("fr"));
        assert_eq!(first(["ja", "ko"]), None);
        assert_eq!(first(Vec::<String>::new()), None);
    }

    #[test]
    fn holes_are_filled_in_any_order() {
        // No catalog is loaded in this test, so the key is the text.
        assert_eq!(fmt("{1} verso {0}", &["A".into(), "B".into()]), "B verso A");
    }
}
