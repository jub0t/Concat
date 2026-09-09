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
    /// The chosen transcriber model id, e.g. "base.en".
    pub transcriber_model: Option<String>,
    /// The chosen speech model id.
    pub tts_model: Option<String>,
    /// The chosen Kokoro speaker id.
    pub tts_voice: Option<i32>,
    /// Row in the transcriber's language list.
    pub transcribe_language: Option<i32>,
    /// The interface's locale code ("de", "pt-BR", ...); absent is English.
    pub locale: Option<String>,
    /// Package ids starred in the effect libraries, in no order. One list
    /// across all three shelves: a star is a fact about a package, and which
    /// library it happens to be filed in is not part of it.
    #[serde(default)]
    pub favourites: Vec<String>,
    /// What a clip plays when its file has several audio tracks.
    pub audio_tracks: AudioTracks,
    /// The playhead stops at the end of the content instead of going where
    /// it is put. Off by default: a click past the last clip lands there, so
    /// a clip can be dropped at the playhead beyond everything else.
    pub playhead_stops_at_end: bool,
}

/// What a clip of a file with several audio tracks plays when it is placed
/// from the bin - a screen recording that kept the desktop and the
/// microphone apart, say. A file with one track has nothing to choose and
/// is left alone whatever this says. The Audio panel changes a placed clip
/// afterwards, per clip; this only sets what a fresh one starts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioTracks {
    /// The first track in file order: what the engine plays unless told
    /// otherwise, and what every clip did before the choice existed.
    #[default]
    First,
    /// The last track in file order.
    Last,
    /// Every track, each as a sound clip of its own on its own lane, and
    /// the video muted - the same as detaching the audio by hand.
    Every,
}

impl AudioTracks {
    /// The choices, in the order the settings sheet lists them.
    pub const ALL: [AudioTracks; 3] = [AudioTracks::First, AudioTracks::Last, AudioTracks::Every];

    /// The row in the settings sheet.
    pub fn row(self) -> i32 {
        Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0) as i32
    }

    /// The choice a row names; a row off the list is the default.
    pub fn from_row(row: i32) -> Self {
        usize::try_from(row)
            .ok()
            .and_then(|row| Self::ALL.get(row).copied())
            .unwrap_or_default()
    }
}

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
