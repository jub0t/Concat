// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the window remembers between runs, as one small JSON file in the
//! app's config directory. None of it is project state: the theme, which
//! models are chosen, which languages. A missing or unreadable file is the
//! defaults, never an error.

use concat_host::AppDirs;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

/// Remembered preferences.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    /// The dark theme. `None` is the app's default, which is dark.
    pub dark: Option<bool>,
    /// The accent, by the lower-cased name the theme lists it under —
    /// "lime", "sky", ... — or as "#rrggbb" for a colour picked by hand.
    /// `None`, or a name no longer on the list, is the first one, which is
    /// Lime.
    pub accent: Option<String>,
    /// The chosen transcriber model id, e.g. "base.en".
    pub transcriber_model: Option<String>,
    /// The chosen speech model id.
    pub tts_model: Option<String>,
    /// The chosen Kokoro speaker id.
    pub tts_voice: Option<i32>,
    /// The interface's locale code ("de", "pt-BR", ...); absent is English.
    pub locale: Option<String>,
    /// Package ids starred in the effect libraries, in no order. One list
    /// across all three shelves: a star is a fact about a package, and which
    /// library it happens to be filed in is not part of it.
    #[serde(default)]
    pub favourites: Vec<String>,
    /// The playhead stops at the end of the content instead of going where
    /// it is put. Off by default: a click past the last clip lands there, so
    /// a clip can be dropped at the playhead beyond everything else.
    pub playhead_stops_at_end: bool,
    /// Show flip horizontal, flip vertical, and reverse in the clip context
    /// menu. Keyboard shortcuts (H, J, R) are always available.
    pub custom_context_actions: bool,
    /// The magnetic timeline: a delete and a trim close the gap they would
    /// leave, on the lane they happen on. Off by default, because a gap is
    /// sometimes the point; ⇧⌫ ripples either way.
    /// https://github.com/jub0t/Concat/issues/106
    pub magnetic: bool,
    /// The preview axis: the monitor shows the frame under the pointer as
    /// it crosses the lanes, the playhead staying where it is. Off by
    /// default: a monitor that jumps with the pointer startles whoever has
    /// not asked for it, and the tray's button is where they ask.
    pub preview_axis: bool,
    /// The preview axis plays the sound under the pointer too. Off by
    /// default: a burst of sound on every pass of the pointer is a lot to
    /// ask of a room.
    pub preview_axis_audio: bool,
    /// Video decodes on the platform's own hardware where it has some:
    /// VideoToolbox on a Mac. `None` is the platform's default, which is on
    /// where the hardware path has been exercised (macOS and iOS) and off
    /// elsewhere; see `Preferences::hardware_decode_on`.
    pub hardware_decode: Option<bool>,
    /// The voices run on the machine's own accelerator - CoreML on a Mac -
    /// rather than the CPU. Off by default: what the accelerator takes of
    /// a network is the network's business, and the CPU is the answer that
    /// is always right.
    pub speech_accelerated: bool,
    /// Where model downloads look first: a `SourcePreference` by name.
    /// Absent is automatic.
    pub download_source: Option<String>,
    /// The base URL a custom download source appends a model's file to.
    pub download_base: Option<String>,
    /// The Concat API on a socket while the window is open.
    #[serde(default)]
    pub server: ServerPrefs,
}

impl Preferences {
    /// Whether video should decode on the hardware: the choice made, or
    /// the platform's default when none was.
    pub fn hardware_decode_on(&self) -> bool {
        self.hardware_decode
            .unwrap_or(cfg!(any(target_os = "macos", target_os = "ios")))
    }

    /// Which of `names` — the theme's accents, in its order — the remembered
    /// accent is. The first when nothing is remembered, and the first when
    /// the name is one the list no longer has: it is the default, and a
    /// choice that has gone away should fall back to the default rather
    /// than to whatever happens to sit at its old position.
    pub fn accent_index(&self, names: impl IntoIterator<Item = impl AsRef<str>>) -> usize {
        let Some(wanted) = self.accent.as_deref().map(str::trim) else {
            return 0;
        };
        names
            .into_iter()
            .position(|name| Self::accent_id(name.as_ref()) == wanted.to_lowercase())
            .unwrap_or(0)
    }

    /// The id an accent is remembered under: its name, lower-cased, so the
    /// file reads `"accent": "sky"` and a hand-typed capital still matches.
    pub fn accent_id(name: &str) -> String {
        name.trim().to_lowercase()
    }

    /// The remembered accent when it is a colour rather than a name: the
    /// hex, for the caller to parse. A hash is what tells the two apart,
    /// and no name on the list starts with one.
    pub fn custom_accent(&self) -> Option<&str> {
        self.accent
            .as_deref()
            .map(str::trim)
            .filter(|text| text.starts_with('#'))
    }
}

/// The Settings sheet's Remote page: whether the API is served while the
/// window is open, where, and behind which token.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerPrefs {
    /// Serve while the window is open.
    pub enabled: bool,
    /// The TCP address JSON-RPC lines are served on.
    pub listen: String,
    /// What a connection presents first. Empty means none, which the
    /// server only allows on loopback.
    pub token: String,
}

impl Default for ServerPrefs {
    fn default() -> Self {
        ServerPrefs {
            enabled: false,
            listen: DEFAULT_LISTEN.to_owned(),
            token: String::new(),
        }
    }
}

/// Where the window's server listens unless told otherwise: loopback, on
/// the port `concat-cli serve` uses too.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7420";

impl Preferences {
    /// Reads the file, or the defaults when there is none.
    pub fn load(dirs: &AppDirs) -> Self {
        std::fs::read(dirs.config.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes the file. Best effort: a preference that did not stick is
    /// not worth interrupting anyone over.
    pub fn save(&self, dirs: &AppDirs) {
        let _ = std::fs::create_dir_all(&dirs.config);
        if let Ok(encoded) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(dirs.config.join(FILE), encoded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 3] = ["Lime", "Sky", "Violet"];

    /// Nothing remembered is the first accent, which is the default.
    #[test]
    fn no_accent_is_the_first() {
        assert_eq!(Preferences::default().accent_index(NAMES), 0);
    }

    /// A remembered name finds its row whatever its case or spacing, since
    /// the file is plain JSON somebody may have edited.
    #[test]
    fn accent_matches_by_name_not_by_case() {
        for spelled in ["violet", "Violet", "VIOLET", "  violet\n"] {
            let prefs = Preferences {
                accent: Some(spelled.to_owned()),
                ..Preferences::default()
            };
            assert_eq!(prefs.accent_index(NAMES), 2, "{spelled:?}");
        }
    }

    /// A name the list no longer has falls back to the default rather than
    /// to a position, and an empty list cannot be indexed into at all.
    #[test]
    fn unknown_or_empty_accent_falls_back() {
        let prefs = Preferences {
            accent: Some("chartreuse".to_owned()),
            ..Preferences::default()
        };
        assert_eq!(prefs.accent_index(NAMES), 0);
        assert_eq!(prefs.accent_index(Vec::<String>::new()), 0);
        let blank = Preferences {
            accent: Some(String::new()),
            ..Preferences::default()
        };
        assert_eq!(blank.accent_index(NAMES), 0);
    }

    /// The id is what gets written and what gets matched, so the two have
    /// to agree on one spelling.
    #[test]
    fn accent_id_round_trips_through_the_file() {
        let prefs = Preferences {
            accent: Some(Preferences::accent_id("Sky")),
            ..Preferences::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains("\"accent\":\"sky\""), "{text}");
        let back: Preferences = serde_json::from_str(&text).unwrap();
        assert_eq!(back.accent_index(NAMES), 1);
    }

    /// A hex is a picked colour and a word is not, whatever else is around
    /// it; and a picked colour is no name, so the index falls back to the
    /// default until the caller has parsed it.
    #[test]
    fn a_hash_means_a_picked_colour() {
        let picked = Preferences {
            accent: Some("  #FF00AA ".to_owned()),
            ..Preferences::default()
        };
        assert_eq!(picked.custom_accent(), Some("#FF00AA"));
        assert_eq!(picked.accent_index(NAMES), 0);
        let named = Preferences {
            accent: Some("sky".to_owned()),
            ..Preferences::default()
        };
        assert_eq!(named.custom_accent(), None);
        assert_eq!(Preferences::default().custom_accent(), None);
        let blank = Preferences {
            accent: Some(String::new()),
            ..Preferences::default()
        };
        assert_eq!(blank.custom_accent(), None);
    }

    /// A file from before the accent existed still loads, and reads as the
    /// default.
    #[test]
    fn older_file_without_an_accent_is_lime() {
        let back: Preferences = serde_json::from_str(r#"{"dark": false}"#).unwrap();
        assert_eq!(back.dark, Some(false));
        assert_eq!(back.accent, None);
        assert_eq!(back.accent_index(NAMES), 0);
    }
}
