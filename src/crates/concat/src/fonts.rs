// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The base fonts, lent to the window, and the faces it falls back on for
//! the scripts they have no glyphs for.
//!
//! concat-text embeds the base fonts for the titles it paints; the window
//! draws its own text, and a text preset's card names the preset in the
//! preset's face. So the same bytes are handed to Slint's font collection
//! once, at start, before any window exists, rather than embedded a second
//! time.
//!
//! None of them has a Chinese, Korean, Arabic, Hebrew, Devanagari, Bengali
//! or Thai glyph, and Hanken Grotesk, the interface's face, has no
//! Cyrillic, so a word in those scripts is whatever the machine's own
//! fonts make of it. A desk has them; a phone from some makers does not,
//! or keeps them where the collection does not look, and a minimal Linux
//! install may have none: the interface in Chinese came up as rows of
//! boxes (#277). So a face for each rides along as the fallback for its
//! scripts, after whatever the machine has - Noto, cut to what the
//! interface needs - and Inter, which the title painter carries anyway,
//! stands behind the machine's own for Cyrillic and Greek.
//!
//! The Noto faces live in concat-text, which falls back on them for titles
//! too; their sources, the way they were cut and their licences are on
//! `concat_text::SCRIPT_FALLBACKS`, and THIRD_PARTY_NOTICES.md lists them.

use std::sync::Arc;

use slint::fontique_011::fontique::{self, FallbackKey, FamilyId, Language, Script};

const CJK_FALLBACK: &[u8] = concat_text::NOTO_SANS_CJK_SC;
const ARABIC_FALLBACK: &[u8] = concat_text::NOTO_SANS_ARABIC_UI;
const HEBREW_FALLBACK: &[u8] = concat_text::NOTO_SANS_HEBREW;
const DEVANAGARI_FALLBACK: &[u8] = concat_text::NOTO_SANS_DEVANAGARI;
const BENGALI_FALLBACK: &[u8] = concat_text::NOTO_SANS_BENGALI;
const THAI_FALLBACK: &[u8] = concat_text::NOTO_SANS_THAI;

/// A face the window falls back on, and the fallback lists it goes on: a
/// script, and the locales fontique keeps a list of its own for under it
/// ("" is the script's default list).
struct Fallback {
    face: &'static [u8],
    family: &'static str,
    scripts: &'static [(&'static str, &'static [&'static str])],
}

/// Every bundled fallback face. Han goes under every locale fontique keeps
/// a list of its own for - Simplified Chinese is the default, and Taiwan,
/// Hong Kong, Macau and Singapore, Japanese and Korean each have one - and
/// Arabic under Persian's and Urdu's as well as its default, Arabic's.
const FALLBACKS: [Fallback; 6] = [
    Fallback {
        face: CJK_FALLBACK,
        family: "Noto Sans CJK SC",
        scripts: &[
            (
                "Hani",
                &["", "zh-TW", "zh-HK", "zh-MO", "zh-SG", "ja", "ko"],
            ),
            ("Hang", &[""]),
            ("Hira", &[""]),
            ("Kana", &[""]),
            ("Bopo", &[""]),
        ],
    },
    Fallback {
        face: ARABIC_FALLBACK,
        family: "Noto Sans Arabic UI",
        scripts: &[("Arab", &["", "fa", "ur"])],
    },
    Fallback {
        face: HEBREW_FALLBACK,
        family: "Noto Sans Hebrew",
        scripts: &[("Hebr", &[""])],
    },
    Fallback {
        face: DEVANAGARI_FALLBACK,
        family: "Noto Sans Devanagari",
        scripts: &[("Deva", &[""])],
    },
    Fallback {
        face: BENGALI_FALLBACK,
        family: "Noto Sans Bengali",
        scripts: &[("Beng", &[""])],
    },
    Fallback {
        face: THAI_FALLBACK,
        family: "Noto Sans Thai",
        scripts: &[("Thai", &[""])],
    },
];

/// The scripts Inter stands behind the machine's fonts for: Hanken
/// Grotesk has Latin and nothing else.
const INTER_SCRIPTS: [&str; 2] = ["Cyrl", "Grek"];

/// Registers every base font with the window's text renderer, and each
/// bundled face as the fallback for its scripts.
pub fn register() {
    let mut collection = slint::fontique_011::shared_collection();
    register_base(&mut collection);
    let registered = register_fallbacks(&mut collection);
    log::debug!("fonts: {} fallback families registered", registered.len());
}

/// Registers the base fonts with `collection`, and puts Inter on the
/// Cyrillic and Greek fallback lists, behind whatever the machine has.
/// What was added: each family with its faces, once per file.
fn register_base(
    collection: &mut fontique::Collection,
) -> Vec<(FamilyId, Vec<fontique::FontInfo>)> {
    let mut registered = Vec::new();
    for face in concat_text::BASE_FONTS {
        let blob = fontique::Blob::new(Arc::new(face));
        registered.extend(collection.register_fonts(blob, None));
    }
    let inter = registered
        .iter()
        .map(|(family, _)| *family)
        .find(|family| collection.family_name(*family) == Some("Inter"));
    if let Some(inter) = inter {
        for script in INTER_SCRIPTS {
            if let Ok(script) = script.parse::<Script>() {
                collection.append_fallbacks(FallbackKey::from(script), std::iter::once(inter));
            }
        }
    }
    registered
}

/// Registers each bundled face with `collection` and appends it to the
/// fallback list of every script it is for, behind whatever the machine
/// has. What was added: each family with its faces, in [`FALLBACKS`]'
/// order.
fn register_fallbacks(
    collection: &mut fontique::Collection,
) -> Vec<(FamilyId, Vec<fontique::FontInfo>)> {
    let mut out = Vec::new();
    for fallback in &FALLBACKS {
        let blob = fontique::Blob::new(Arc::new(fallback.face));
        let registered = collection.register_fonts(blob, None);
        let families: Vec<FamilyId> = registered.iter().map(|(family, _)| *family).collect();
        for key in fallback_keys(fallback) {
            collection.append_fallbacks(key, families.iter().copied());
        }
        if registered.is_empty() {
            log::warn!(
                "fonts: the bundled {} face did not register",
                fallback.family
            );
        }
        out.extend(registered);
    }
    out
}

/// The fallback lists `fallback` goes on, each once: a locale that shares
/// its script's default list, such as zh-Hans or ar, is not named, or
/// the face would be appended to that list twice.
fn fallback_keys(fallback: &Fallback) -> Vec<FallbackKey> {
    let mut keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (script, locales) in fallback.scripts {
        let Ok(script) = script.parse::<Script>() else {
            continue;
        };
        for locale in *locales {
            let language = if locale.is_empty() {
                None
            } else {
                Language::parse(locale).ok()
            };
            let key = FallbackKey::new(script, language.as_ref());
            if seen.insert(key) {
                keys.push(key);
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection() -> fontique::Collection {
        fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        })
    }

    /// Each bundled face registers as one family under its own name and
    /// goes on the fallback list of every script it is for, so a word in
    /// that script has somewhere to go on a machine with no font of its
    /// own for it.
    #[test]
    fn each_face_is_the_fallback_for_its_scripts() {
        let mut collection = collection();
        let registered = register_fallbacks(&mut collection);
        assert_eq!(registered.len(), FALLBACKS.len());
        for (fallback, (family, _)) in FALLBACKS.iter().zip(&registered) {
            assert_eq!(collection.family_name(*family), Some(fallback.family));
            for key in fallback_keys(fallback) {
                let listed: Vec<FamilyId> = collection.fallback_families(key).collect();
                assert_eq!(listed, vec![*family], "{} on {key:?}", fallback.family);
            }
        }
        // The lists a Chinese, Korean, Persian or Urdu word is looked up
        // by, named the way the shaper names them.
        for (script, locale, family) in [
            ("Hani", None, "Noto Sans CJK SC"),
            ("Hani", Some("zh-TW"), "Noto Sans CJK SC"),
            ("Hani", Some("ko"), "Noto Sans CJK SC"),
            ("Hang", None, "Noto Sans CJK SC"),
            ("Arab", None, "Noto Sans Arabic UI"),
            ("Arab", Some("ar"), "Noto Sans Arabic UI"),
            ("Arab", Some("fa"), "Noto Sans Arabic UI"),
            ("Arab", Some("ur"), "Noto Sans Arabic UI"),
            ("Hebr", Some("he"), "Noto Sans Hebrew"),
            ("Deva", Some("hi"), "Noto Sans Devanagari"),
            ("Beng", Some("bn"), "Noto Sans Bengali"),
            ("Thai", Some("th"), "Noto Sans Thai"),
        ] {
            let script: Script = script.parse().expect("a script");
            let language = locale.and_then(|locale| Language::parse(locale).ok());
            let key = FallbackKey::new(script, language.as_ref());
            let listed: Vec<FamilyId> = collection.fallback_families(key).collect();
            assert_eq!(listed.len(), 1, "{key:?}");
            assert_eq!(collection.family_name(listed[0]), Some(family), "{key:?}");
        }
    }

    /// Inter stands behind the machine's fonts for Cyrillic and Greek,
    /// which the interface's own face does not draw.
    #[test]
    fn inter_is_the_fallback_for_cyrillic() {
        let mut collection = collection();
        register_base(&mut collection);
        for script in INTER_SCRIPTS {
            let script: Script = script.parse().expect("a script");
            let listed: Vec<FamilyId> = collection.fallback_families(script).collect();
            assert_eq!(listed.len(), 1, "{script:?}");
            assert_eq!(collection.family_name(listed[0]), Some("Inter"));
        }
    }

    /// Every character every shipped locale uses has a glyph in the
    /// interface's face, Hanken Grotesk, or in a face this module puts on
    /// a fallback list, so no word of the interface is a box on a machine
    /// with no font of its own for its script. Read off the faces' own
    /// character maps.
    #[test]
    fn every_locale_has_a_glyph_for_every_character() {
        const INTERFACE: &[u8] =
            include_bytes!("../../concat-text/fonts/HankenGrotesk-Regular.ttf");
        let mut collection = collection();
        let mut faces: Vec<(&[u8], fontique::FontInfo)> = Vec::new();
        let interface = collection.register_fonts(fontique::Blob::new(Arc::new(INTERFACE)), None);
        faces.push((INTERFACE, interface[0].1[0].clone()));
        for (fallback, (_, infos)) in FALLBACKS.iter().zip(register_fallbacks(&mut collection)) {
            faces.push((fallback.face, infos[0].clone()));
        }
        let inter = concat_text::BASE_FONTS[0];
        let registered = collection.register_fonts(fontique::Blob::new(Arc::new(inter)), None);
        assert_eq!(collection.family_name(registered[0].0), Some("Inter"));
        faces.push((inter, registered[0].1[0].clone()));
        let charmaps: Vec<_> = faces
            .iter()
            .map(|(bytes, info)| {
                info.charmap_index()
                    .charmap(bytes)
                    .expect("the face has a character map")
            })
            .collect();
        let mut failures = Vec::new();
        for (code, text) in crate::i18n::BUILT_IN {
            let mut missing: Vec<char> = text
                .chars()
                .filter(|ch| !ch.is_control())
                .filter(|ch| {
                    !charmaps
                        .iter()
                        .any(|charmap| charmap.map(*ch).is_some_and(|glyph| glyph != 0))
                })
                .collect();
            missing.sort_unstable();
            missing.dedup();
            if !missing.is_empty() {
                failures.push(format!("{code} has no glyph for {missing:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn every_fallback_key_is_listed_once() {
        for fallback in &FALLBACKS {
            let keys = fallback_keys(fallback);
            let unique: std::collections::HashSet<_> = keys.iter().copied().collect();
            assert_eq!(keys.len(), unique.len(), "{}", fallback.family);
            let named: usize = fallback
                .scripts
                .iter()
                .map(|(_, locales)| locales.len())
                .sum();
            assert_eq!(keys.len(), named, "{}: {keys:?}", fallback.family);
        }
    }
}
