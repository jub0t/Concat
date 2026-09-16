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
    /// Where model downloads look first: a `SourcePreference` by name.
    /// Absent is automatic.
    pub download_source: Option<String>,
    /// The base URL a custom download source appends a model's file to.
    pub download_base: Option<String>,
    /// The Concat API on a socket while the window is open.
    #[serde(default)]
    pub server: ServerPrefs,
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
