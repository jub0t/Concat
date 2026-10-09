// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The base fonts, lent to the window, and the face it falls back on for
//! the scripts they have no glyphs for.
//!
//! concat-text embeds the base fonts for the titles it paints; the window
//! draws its own text, and a text preset's card names the preset in the
//! preset's face. So the same bytes are handed to Slint's font collection
//! once, at start, before any window exists, rather than embedded a second
//! time.
//!
//! None of them has a Chinese or Korean glyph, so a word in those scripts
//! is whatever the machine's own fonts make of it. A desk has one; a phone
//! from some makers does not, or keeps it where the collection does not
//! look, and the interface in Chinese came up as rows of boxes (#277). A
//! subset of Noto Sans CJK - the Simplified and the common Traditional
//! Chinese characters, the Hangul syllables, the kana and the punctuation
//! that goes with them - rides along as the fallback for those scripts,
//! after whatever the machine has.

use std::sync::Arc;

use slint::fontique_011::fontique::{self, FallbackKey, FamilyId, Language, Script};

/// Noto Sans CJK SC, cut to the characters the interface falls back on:
/// GB 2312, the common half of Big5, KS X 1001's Hangul, the kana, and
/// the CJK punctuation and fullwidth forms. Two megabytes rather than
/// sixteen.
const CJK_FALLBACK: &[u8] = include_bytes!("../fonts/NotoSansCJKsc-Concat.otf");

/// Registers every base font with the window's text renderer, and the CJK
/// face as the fallback for its scripts.
pub fn register() {
    let mut collection = slint::fontique_011::shared_collection();
    for face in concat_text::BASE_FONTS {
        let blob = fontique::Blob::new(Arc::new(face));
        collection.register_fonts(blob, None);
    }
    let families = register_cjk_fallback(&mut collection);
    log::debug!("fonts: {} CJK fallback family registered", families.len());
}

/// Registers the CJK face with `collection` and appends it to the fallback
/// list of every script it is for, behind whatever the machine has. The
/// families it added.
fn register_cjk_fallback(collection: &mut fontique::Collection) -> Vec<FamilyId> {
    let blob = fontique::Blob::new(Arc::new(CJK_FALLBACK));
    let families: Vec<FamilyId> = collection
        .register_fonts(blob, None)
        .into_iter()
        .map(|(family, _)| family)
        .collect();
    for key in cjk_fallback_keys() {
        collection.append_fallbacks(key, families.iter().copied());
    }
    families
}

/// The fallback lists the face goes on: Han under every locale fontique
/// keeps a list of its own for - Simplified Chinese is the default, and
/// Taiwan, Hong Kong, Macau and Singapore, Japanese and Korean each have
/// one - and the Hangul, kana and bopomofo scripts. Each list once: a
/// locale that shares the default's list, such as zh-Hans, is not named,
/// or the face would be appended to that list twice.
fn cjk_fallback_keys() -> Vec<FallbackKey> {
    const SCRIPTS: [(&str, &[&str]); 5] = [
        (
            "Hani",
            &["", "zh-TW", "zh-HK", "zh-MO", "zh-SG", "ja", "ko"],
        ),
        ("Hang", &[""]),
        ("Hira", &[""]),
        ("Kana", &[""]),
        ("Bopo", &[""]),
    ];
    let mut keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (script, locales) in SCRIPTS {
        let Ok(script) = script.parse::<Script>() else {
            continue;
        };
        for locale in locales {
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

    /// The CJK face registers as one family and goes on the Han fallback
    /// lists, so a Chinese or Korean word has somewhere to go on a machine
    /// with no font of its own for it.
    #[test]
    fn the_cjk_face_is_the_fallback_for_han_and_hangul() {
        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let families = register_cjk_fallback(&mut collection);
        assert_eq!(families.len(), 1);
        assert_eq!(
            collection.family_name(families[0]),
            Some("Noto Sans CJK SC")
        );
        let han: Script = "Hani".parse().expect("a script");
        for key in [
            FallbackKey::from(han),
            FallbackKey::new(han, Language::parse("zh-TW").ok().as_ref()),
            FallbackKey::new(han, Language::parse("ko").ok().as_ref()),
        ] {
            let listed: Vec<FamilyId> = collection.fallback_families(key).collect();
            assert_eq!(listed, families, "{key:?}");
        }
        let hangul: Script = "Hang".parse().expect("a script");
        let listed: Vec<FamilyId> = collection.fallback_families(hangul).collect();
        assert_eq!(listed, families);
    }

    #[test]
    fn every_fallback_key_is_listed_once() {
        let keys = cjk_fallback_keys();
        let unique: std::collections::HashSet<_> = keys.iter().copied().collect();
        assert_eq!(keys.len(), unique.len());
        assert!(keys.len() >= 8, "{keys:?}");
    }
}
