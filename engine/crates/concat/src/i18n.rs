// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The interface's words in other languages.
//!
//! A locale is one JSON file: the English string as the key, the
//! translation as the value, and a `_` entry naming the language in its
//! own words. Every string a person reads passes through [`t`] here or
//! `I18n.t` in the `.slint` tree, with the English as the key, so a string
//! that has no translation yet reads in English rather than as a code, and
//! adding a language is adding a file.
//!
//! Two places hold locales. The ones the app ships live in `locales/` next
//! to this crate's `src/` and are compiled in. Anyone can add or correct a
//! language without a build by dropping `<code>.json` into the `locales`
//! folder of the app's config directory; a file there with a shipped
//! locale's code lays its entries over the shipped ones, so a correction
//! is a file of the lines that change.
//!
//! One lookup is a hash-map read behind a read lock, and the active
//! catalogue is swapped whole, so changing language is one write and every
//! string reads it on its next evaluation.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, OnceLock, RwLock};

use concat_host::AppDirs;

/// A language the interface can be in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    /// The locale's code, and the name of its file: "de", "pt-BR", ...
    pub code: String,
    /// The language's name for itself, as Settings lists it: whoever is
    /// stranded in a language they cannot read must still recognise their
    /// own.
    pub name: String,
}

/// The code the interface starts in, and the language of the keys.
pub const ENGLISH: &str = "en";

/// The locales the app ships, as `(code, file)`. `en.json` is the
/// catalogue's own inventory - every key, with itself as the value - which
/// is what a translator starts a new file from.
const BUILT_IN: [(&str, &str); 11] = [
    ("en", include_str!("../locales/en.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("it", include_str!("../locales/it.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ko", include_str!("../locales/ko.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("tr", include_str!("../locales/tr.json")),
    ("zh-Hans", include_str!("../locales/zh-Hans.json")),
];

/// One language's strings.
struct Catalog {
    code: String,
    strings: HashMap<String, String>,
}

fn active() -> &'static RwLock<Option<Arc<Catalog>>> {
    static ACTIVE: OnceLock<RwLock<Option<Arc<Catalog>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(None))
}

/// Where a machine's own locales live.
pub fn user_dir(dirs: &AppDirs) -> std::path::PathBuf {
    dirs.config.join("locales")
}

/// Every language on offer: English, then the shipped locales, then any
/// the machine adds, each once, by code. A machine's file for a shipped
/// code renames it if its `_` entry says so.
pub fn languages(dirs: &AppDirs) -> Vec<Language> {
    let mut out: Vec<Language> = vec![Language {
        code: ENGLISH.to_owned(),
        name: "English".to_owned(),
    }];
    for (code, text) in BUILT_IN {
        if code == ENGLISH {
            continue;
        }
        if let Some((name, _)) = parse(text) {
            out.push(Language {
                code: code.to_owned(),
                name,
            });
        }
    }
    let mut added: Vec<Language> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(user_dir(dirs)) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(code) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
                .filter(|code| !code.is_empty())
            else {
                continue;
            };
            let Some((name, _)) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| parse(&text))
            else {
                continue;
            };
            match out.iter_mut().find(|held| held.code == code) {
                Some(held) => {
                    if !name.is_empty() {
                        held.name = name;
                    }
                }
                None => added.push(Language {
                    code: code.to_owned(),
                    name: if name.is_empty() {
                        code.to_owned()
                    } else {
                        name
                    },
                }),
            }
        }
    }
    added.sort_by(|a, b| a.code.cmp(&b.code));
    out.extend(added);
    out
}

/// Makes `code` the interface's language: the shipped locale of that code,
/// with the machine's own file of the same code laid over it. English, or
/// a code nothing answers to, is the keys themselves.
pub fn select(code: &str, dirs: &AppDirs) {
    let mut strings: HashMap<String, String> = HashMap::new();
    if code != ENGLISH
        && let Some((_, built_in)) = BUILT_IN
            .iter()
            .find(|(held, _)| *held == code)
            .and_then(|(_, text)| parse(text))
    {
        strings = built_in;
    }
    if let Some((_, own)) = std::fs::read_to_string(user_dir(dirs).join(format!("{code}.json")))
        .ok()
        .and_then(|text| parse(&text))
    {
        strings.extend(own);
    }
    let catalog = (!strings.is_empty() || code != ENGLISH).then(|| {
        Arc::new(Catalog {
            code: code.to_owned(),
            strings,
        })
    });
    if let Ok(mut slot) = active().write() {
        *slot = catalog;
    }
}

/// The code of the language the interface is in.
pub fn current() -> String {
    active()
        .read()
        .ok()
        .and_then(|slot| slot.as_ref().map(|catalog| catalog.code.clone()))
        .unwrap_or_else(|| ENGLISH.to_owned())
}

/// `key` in the interface's language, or `key` when there is no
/// translation for it.
pub fn t(key: &str) -> String {
    active()
        .read()
        .ok()
        .and_then(|slot| {
            slot.as_ref()
                .and_then(|catalog| catalog.strings.get(key).cloned())
        })
        .unwrap_or_else(|| key.to_owned())
}

/// [`t`], with `{0}`, `{1}`, ... replaced by `args` in order.
pub fn tf(key: &str, args: &[&dyn Display]) -> String {
    fill(&t(key), args)
}

/// `{0}`, `{1}`, ... in `text` replaced by `args`.
fn fill(text: &str, args: &[&dyn Display]) -> String {
    let mut out = text.to_owned();
    for (index, arg) in args.iter().enumerate() {
        out = out.replace(&format!("{{{index}}}"), &arg.to_string());
    }
    out
}

/// A locale file's name for its language and its strings. `None` for a
/// file that is not a JSON object.
fn parse(text: &str) -> Option<(String, HashMap<String, String>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let name = object
        .get("_")
        .and_then(|meta| meta.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_default()
        .to_owned();
    let strings = object
        .iter()
        .filter(|(key, _)| key.as_str() != "_")
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect();
    Some((name, strings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inventory: every key the app can ask for, as `en.json` lists
    /// them. `scripts/locales.py` regenerates that file from the source.
    fn inventory() -> HashMap<String, String> {
        parse(BUILT_IN[0].1).expect("en.json parses").1
    }

    #[test]
    fn every_shipped_locale_parses_names_itself_and_keys_off_the_inventory() {
        let inventory = inventory();
        assert!(inventory.len() > 100, "the inventory is missing");
        for (code, text) in BUILT_IN {
            let (name, strings) = parse(text).unwrap_or_else(|| panic!("{code}.json parses"));
            assert!(!name.is_empty(), "{code}.json names its language");
            for key in strings.keys() {
                assert!(
                    inventory.contains_key(key),
                    "{code}.json translates {key:?}, which nothing asks for"
                );
            }
            if code != ENGLISH {
                // A shipped translation covers the inventory: a missing
                // line would read in English in the middle of a page.
                let missing: Vec<&String> = inventory
                    .keys()
                    .filter(|key| !strings.contains_key(*key))
                    .collect();
                assert!(missing.is_empty(), "{code}.json lacks {missing:?}");
            }
        }
    }

    #[test]
    fn placeholders_fill_in_order() {
        assert_eq!(fill("{1} of {0}", &[&3, &"a"]), "a of 3");
        assert_eq!(fill("plain", &[&1]), "plain");
    }

    #[test]
    fn a_missing_translation_reads_as_the_key() {
        let root = std::env::temp_dir().join(format!("concat-i18n-{}", std::process::id()));
        let dirs = AppDirs::under(&root);
        select("de", &dirs);
        assert_eq!(current(), "de");
        assert_eq!(t("a key nobody translated"), "a key nobody translated");
        assert_ne!(t("Settings"), "Settings");
        select(ENGLISH, &dirs);
        assert_eq!(t("Settings"), "Settings");
    }
}
