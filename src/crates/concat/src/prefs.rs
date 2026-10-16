// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the window remembers between runs, as one small JSON file in the
//! app's config directory. None of it is project state: the theme, which
//! models are chosen, which languages. A missing or unreadable file is the
//! defaults, never an error.

use concat_host::AppDirs;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

/// The interface scales Settings > Appearance offers, smallest first, as
/// percents of the display's own scale. Nothing on the list - and no value
/// at all - leaves the window at the scale the platform hands it.
/// https://github.com/jub0t/Concat/issues/320
pub const UI_SCALES: [u32; 5] = [100, 125, 150, 175, 200];

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
    /// The playhead's colour as "#rrggbb", when one was picked by hand in
    /// Settings > Appearance. `None` is the palette's own, which is lime.
    pub playhead: Option<String>,
    /// The interface scale, as a percent from [`UI_SCALES`]: the window and
    /// everything in it are drawn this much larger than the display's own
    /// scale says. `None` - or a value the list no longer has - follows the
    /// display. Settings > Appearance picks it, and it is remembered here
    /// for the window a launch makes; the window that is open is rescaled in
    /// place the moment the choice is made.
    /// https://github.com/jub0t/Concat/issues/320
    pub ui_scale: Option<u32>,
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
    /// Proxies: a file larger than HD gets a smaller copy written in the
    /// background, and playback at Half and Quarter reads that copy. Off
    /// by default, as in CapCut: a proxy is a full re-encode of every large
    /// file, on the CPU of a phone or through a hardware encoder a laptop
    /// may not take (an HDR one opened NVENC first), and the person who
    /// wants one for smoother playback turns it on in Settings.
    pub proxies: bool,
    /// The magnetic timeline: a delete and a trim close the gap they would
    /// leave, on the lane they happen on. Off by default, because a gap is
    /// sometimes the point; ⇧⌫ ripples either way.
    /// https://github.com/jub0t/Concat/issues/106
    pub magnetic: bool,
    /// Trim follow: while a clip's edge is dragged, the playhead rides the
    /// edge, so the monitor shows the exact frame the cut lands on. Off by
    /// default; the tray's button beside the hand turns it on.
    pub trim_follow: bool,
    /// The preview axis: the monitor shows the frame under the pointer as
    /// it crosses the lanes, the playhead staying where it is. Off by
    /// default: a monitor that jumps with the pointer startles whoever has
    /// not asked for it, and the tray's button is where they ask.
    pub preview_axis: bool,
    /// The preview axis plays the sound under the pointer too. Off by
    /// default: a burst of sound on every pass of the pointer is a lot to
    /// ask of a room.
    pub preview_axis_audio: bool,
    /// Where model downloads look first: a `SourcePreference` by name.
    /// Absent is automatic.
    pub download_source: Option<String>,
    /// The base URL a custom download source appends a model's file to.
    pub download_base: Option<String>,
    /// The Concat API on a socket while the window is open.
    #[serde(default)]
    pub server: ServerPrefs,
    /// The look new titles start from and generated captions are set in,
    /// kept from a title by the Text panel's "Use for new titles"; its
    /// words are empty. None starts titles in the bundled face.
    pub title_style: Option<concat_project::model::TextStyle>,
    /// What the log records, by the name concat_host::logs reads: `info`
    /// unless the Settings sheet was asked for more or less. CONCAT_LOG in
    /// the environment overrides it for one run.
    pub log_level: Option<String>,
}

impl Preferences {
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

    /// The scale index the Settings row shows: 0 follows the display, then
    /// one per [`UI_SCALES`]. A remembered percent the list no longer has
    /// reads as 0, the way a name the accent list no longer has falls back
    /// to the default rather than to whatever sits at its old position.
    pub fn scale_index(&self) -> usize {
        self.ui_scale
            .and_then(|percent| UI_SCALES.iter().position(|&scale| scale == percent))
            .map_or(0, |row| row + 1)
    }

    /// The percent a remembered row names: `None` at 0, which is the
    /// display's own, and `None` for a row past the list.
    pub fn scale_at(index: usize) -> Option<u32> {
        index
            .checked_sub(1)
            .and_then(|row| UI_SCALES.get(row).copied())
    }

    /// The multiplier the window backend takes for a percent: the percent
    /// over a hundred, which is what `SLINT_SCALE_FACTOR` parses.
    pub fn scale_factor(percent: u32) -> f64 {
        f64::from(percent) / 100.0
    }
}

/// The Settings sheet's Automation page: whether the API is served while the
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
    fn older_file_without_an_accent_is_the_default() {
        let back: Preferences = serde_json::from_str(r#"{"dark": false}"#).unwrap();
        assert_eq!(back.dark, Some(false));
        assert_eq!(back.accent, None);
        assert_eq!(back.accent_index(NAMES), 0);
    }

    /// A picked playhead colour is written as its hex and read back as
    /// one, and a file from before it existed reads as the palette's own.
    #[test]
    fn a_picked_playhead_round_trips_and_an_older_file_has_none() {
        let prefs = Preferences {
            playhead: Some("#ff453a".to_owned()),
            ..Preferences::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains("\"playhead\":\"#ff453a\""), "{text}");
        let back: Preferences = serde_json::from_str(&text).unwrap();
        assert_eq!(back.playhead.as_deref(), Some("#ff453a"));
        let older: Preferences = serde_json::from_str(r#"{"dark": true}"#).unwrap();
        assert_eq!(older.playhead, None);
    }

    /// The scale reaches its row by the percent the file holds, and a
    /// percent nobody offers falls back to the display's own - row 0 - as
    /// an unknown accent falls back to the default.
    #[test]
    fn a_remembered_scale_finds_its_row() {
        let at = |percent: Option<u32>| {
            Preferences {
                ui_scale: percent,
                ..Preferences::default()
            }
            .scale_index()
        };
        assert_eq!(at(None), 0);
        assert_eq!(at(Some(100)), 1);
        assert_eq!(at(Some(200)), 5);
        assert_eq!(at(Some(133)), 0);
    }

    /// The row finds the percent back, row 0 is the display's own, and a
    /// row past the list is none of the app's.
    #[test]
    fn a_row_finds_its_scale() {
        assert_eq!(Preferences::scale_at(0), None);
        assert_eq!(Preferences::scale_at(1), Some(100));
        assert_eq!(Preferences::scale_at(5), Some(200));
        assert_eq!(Preferences::scale_at(6), None);
    }

    /// A scale reads as the multiplier the backend's `SLINT_SCALE_FACTOR`
    /// wants: the percent over a hundred.
    #[test]
    fn a_scale_reads_as_a_multiplier() {
        assert_eq!(Preferences::scale_factor(100), 1.0);
        assert_eq!(Preferences::scale_factor(150), 1.5);
        assert_eq!(Preferences::scale_factor(200), 2.0);
    }

    /// A scale is written as its percent and read back, and a file from
    /// before the scale existed follows the display.
    #[test]
    fn a_scale_round_trips_and_an_older_file_follows_the_display() {
        let prefs = Preferences {
            ui_scale: Some(150),
            ..Preferences::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains("\"uiScale\":150"), "{text}");
        let back: Preferences = serde_json::from_str(&text).unwrap();
        assert_eq!(back.scale_index(), 3);
        let older: Preferences = serde_json::from_str(r#"{"dark": true}"#).unwrap();
        assert_eq!(older.ui_scale, None);
        assert_eq!(older.scale_index(), 0);
    }
}
