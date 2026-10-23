// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! What the window remembers between runs, as one small JSON file in the
//! app's config directory. None of it is project state: the theme, which
//! models are chosen, which languages. A missing or unreadable file is the
//! defaults, never an error.

use std::collections::BTreeMap;

use concat_host::AppDirs;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

/// The interface scales Settings > Appearance offers, smallest first, as
/// percents of the display's own scale. Nothing on the list - and no value
/// at all - leaves the window at the scale the platform hands it. 110 is
/// for a 1x desk where 100 reads small and 125 is a size too far.
/// https://github.com/jub0t/Concat/issues/320
pub const UI_SCALES: [u32; 6] = [100, 110, 125, 150, 175, 200];

/// Remembered preferences.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    /// The dark theme. `None` is the app's default, which is dark.
    pub dark: Option<bool>,
    /// The parent folder of the last project created successfully.
    pub last_project_location: Option<String>,
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
    /// Drawing in software: the window and the compositor on the
    /// platform's software rasteriser (WARP, lavapipe) instead of the GPU,
    /// as the person chose in Settings > General or with `concat
    /// --software`. Kept until they turn it back. Read at launch.
    ///
    /// Not the `softwareGpu` key 0.2.8 wrote: that one a safe start also
    /// set, for good, and nothing can tell the two apart. Left behind, it
    /// gives every machine a safe start trapped on the CPU one more go on
    /// its GPU - a hover drew a 14700K at "Very High" power (#476) - and
    /// a GPU that still dies starting puts it back in software by itself.
    #[serde(rename = "drawInSoftware")]
    pub software_gpu: bool,
    /// The build whose launch died while starting, which put the window in
    /// software by itself (see `safe_start`): until another build starts,
    /// or the person turns the GPU back on. A crash that an update fixed -
    /// the stray dxcompiler.dll of #418 - or a driver that a later one did
    /// would otherwise leave Concat on the CPU for good.
    pub software_after_crash: Option<String>,
    /// The magnetic timeline: a delete and a trim close the gap they would
    /// leave, on the lane they happen on. Off by default on a desk, because
    /// a gap is sometimes the point; ⇧⌫ ripples either way. Read it through
    /// [`Preferences::magnetic`], which knows the phone's default.
    /// https://github.com/jub0t/Concat/issues/106
    pub magnetic: bool,
    /// Whether the person has set the magnetic timeline, either way. Every
    /// phone has saved `magnetic: false` without anyone choosing it, so
    /// this is what tells a phone's chosen Off from the default it got.
    pub magnetic_chosen: bool,
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
    /// The VU meters at the timeline's right edge. `None` is shown: how
    /// loud the cut plays is worth seeing before anyone thinks to ask, and
    /// the tray's meter button puts them away.
    pub vu_meters: Option<bool>,
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
    /// How the voiceover panel was last set up.
    #[serde(default)]
    pub voiceover: VoiceoverPrefs,
    /// Rebound shortcuts, by action id: the chord each was given, or ""
    /// for an action a rebind left with none. Absent is the shipped
    /// chord — the file holds what moved, not the whole table. Written
    /// by Settings › Shortcuts; resolved in [`crate::keymap`].
    #[serde(default)]
    pub keybinds: BTreeMap<String, String>,
}

/// The voiceover panel's choices, kept from one take to the next and one
/// run to the next: a microphone picked once stays picked.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceoverPrefs {
    /// The microphone, by the name the system gives it. None, or one that
    /// is no longer plugged in, is the system's default.
    pub device: Option<String>,
    /// Three seconds counted in before a take starts. On by default: the
    /// breath before the first word is what a count-in is for.
    pub countdown: bool,
    /// The timeline is silent while a take records, so the speakers do not
    /// bleed into the microphone. On by default; off is for headphones.
    pub mute: bool,
    /// The input gain, in decibels.
    pub gain_db: f32,
    /// How fast the teleprompter scrolls: 0 slow, 1 steady, 2 fast.
    pub prompt_speed: u8,
}

impl Default for VoiceoverPrefs {
    fn default() -> Self {
        VoiceoverPrefs {
            device: None,
            countdown: true,
            mute: true,
            gain_db: 0.0,
            prompt_speed: 1,
        }
    }
}

impl Preferences {
    /// Whether the timeline shows its VU meters; see `vu_meters`.
    pub fn meters_shown(&self) -> bool {
        self.vu_meters.unwrap_or(true)
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
    // A phone sets its size the system's way and never scales by this.
    #[cfg_attr(target_os = "android", allow(dead_code))]
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
    /// The remembered parent folder, if it is still a directory.
    pub fn project_location(&self) -> Option<&str> {
        self.last_project_location
            .as_deref()
            .filter(|location| std::path::Path::new(location).is_dir())
    }

    /// Whether the magnet shows as on. On a phone, until the person says
    /// otherwise: the phone editors close the gap a delete leaves on the
    /// main picture's lane, a Split and Delete there left a black hole in
    /// the picture and the export (#497), and the phone shell has no magnet
    /// in sight to turn on. What that default closes is
    /// `Studio::closes_gap`'s to say; the field alone is the desk's magnet,
    /// which closes every lane.
    pub fn magnetic(&self) -> bool {
        self.magnetic || (crate::platform::phone() && !self.magnetic_chosen)
    }

    /// Sets the magnetic timeline by the person's choice.
    pub fn set_magnetic(&mut self, on: bool) {
        self.magnetic = on;
        self.magnetic_chosen = true;
    }

    /// Whether this launch draws in software, by choice or after a crash.
    pub fn draws_in_software(&self) -> bool {
        self.software_gpu || self.software_after_crash.is_some()
    }

    /// Settles how this launch draws, from whether the last one died while
    /// starting and which build this is; whether anything changed and
    /// wants saving. A death puts the window in software for this build. A
    /// different build tries the GPU again, and a death on it puts the
    /// window back.
    pub fn settle_start(&mut self, died_starting: bool, build: &str) -> bool {
        if self.software_gpu {
            return false;
        }
        match &self.software_after_crash {
            Some(crashed) if crashed != build => {
                log::info!("start: {crashed} closed while starting; {build} tries the GPU again");
                self.software_after_crash = None;
                true
            }
            None if died_starting => {
                log::warn!(
                    "start: the last launch closed while starting; drawing in software until another build"
                );
                self.software_after_crash = Some(build.to_owned());
                true
            }
            _ => false,
        }
    }

    /// Reads the file, or the defaults when there is none.
    pub fn load(dirs: &AppDirs) -> Self {
        let path = dirs.config.join(FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        if let Ok(preferences) = serde_json::from_slice(&bytes) {
            return preferences;
        }
        // One value of the wrong shape - a field an older or newer build
        // wrote differently, a hand edit - failed the whole file, and every
        // setting went back to its default at once. The ones that still
        // read are kept, and the file as it was is set aside rather than
        // overwritten by the next save.
        let _ = std::fs::copy(&path, path.with_extension("json.unreadable"));
        let preferences = salvage(&bytes);
        log::warn!(
            "settings: {} did not read whole; kept what did",
            path.display()
        );
        preferences
    }

    /// Writes the file. Best effort: a preference that did not stick is
    /// not worth interrupting anyone over. Written beside the file and
    /// moved over it, so a crash or a second window saving at the same
    /// moment leaves one whole file, never half of one; and readable by
    /// this account alone where the platform has the notion, since it
    /// holds the remote API's token.
    pub fn save(&self, dirs: &AppDirs) {
        let _ = std::fs::create_dir_all(&dirs.config);
        let Ok(encoded) = serde_json::to_vec_pretty(self) else {
            return;
        };
        let path = dirs.config.join(FILE);
        let staging = dirs
            .config
            .join(format!("{FILE}.{}.part", std::process::id()));
        if write_private(&staging, &encoded).is_err() || std::fs::rename(&staging, &path).is_err() {
            let _ = std::fs::remove_file(&staging);
        }
    }
}

/// The fields of a settings file that read, on the defaults: each top-level
/// value is laid over them in turn and kept only if the whole still reads.
fn salvage(bytes: &[u8]) -> Preferences {
    let Ok(serde_json::Value::Object(found)) = serde_json::from_slice(bytes) else {
        return Preferences::default();
    };
    let Ok(serde_json::Value::Object(mut kept)) = serde_json::to_value(Preferences::default())
    else {
        return Preferences::default();
    };
    for (key, value) in found {
        let before = kept.insert(key.clone(), value);
        if serde_json::from_value::<Preferences>(serde_json::Value::Object(kept.clone())).is_err() {
            match before {
                Some(before) => kept.insert(key, before),
                None => kept.remove(&key),
            };
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

/// Writes a new file only this account can read, on Unix; elsewhere as
/// the platform makes files, which on Windows is the profile's own folder.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A choice made is kept, whatever the platform's default: a phone
    /// that turned the magnet off stays off, and a desk that turned it on
    /// stays on.
    #[test]
    fn a_chosen_magnetic_timeline_outranks_the_default() {
        let mut prefs = Preferences::default();
        assert!(!prefs.magnetic_chosen);
        prefs.set_magnetic(false);
        assert!(!prefs.magnetic());
        prefs.set_magnetic(true);
        assert!(prefs.magnetic());
        // A file saved before the choice was recorded reads as unchosen.
        let old: Preferences = serde_json::from_str(r#"{"magnetic":false}"#).expect("old prefs");
        assert!(!old.magnetic_chosen);
        assert_eq!(old.magnetic(), crate::platform::phone());
    }

    /// One value of the wrong shape costs that value, not every setting,
    /// and the file is set aside before anything writes over it.
    #[test]
    fn a_settings_file_that_does_not_read_whole_keeps_what_does() {
        let root =
            std::env::temp_dir().join(format!("concat-settings-salvage-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            data: root.join("data"),
        };
        std::fs::create_dir_all(&dirs.config).expect("a config folder");
        std::fs::write(
            dirs.config.join(FILE),
            r#"{"magnetic": true, "dark": "very", "lastProjectLocation": "/films"}"#,
        )
        .expect("a settings file");
        let prefs = Preferences::load(&dirs);
        assert!(prefs.magnetic);
        assert_eq!(prefs.last_project_location.as_deref(), Some("/films"));
        assert_eq!(
            prefs.dark, None,
            "the value that did not read is the default"
        );
        assert!(dirs.config.join("settings.json.unreadable").is_file());

        prefs.save(&dirs);
        assert!(Preferences::load(&dirs).magnetic);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dirs.config.join(FILE))
                .expect("saved")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "nobody else reads it");
        }
        let leftovers = std::fs::read_dir(&dirs.config)
            .expect("listed")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_last_project_location_survives_loading_and_saving() {
        let root =
            std::env::temp_dir().join(format!("concat-project-location-{}", std::process::id()));
        let location = root.join("my projects");
        std::fs::create_dir_all(&location).unwrap();
        let dirs = AppDirs::under(&root);
        let location = location.to_string_lossy().into_owned();
        std::fs::write(
            dirs.config.join(FILE),
            serde_json::to_vec(&serde_json::json!({
                "dark": false,
                "lastProjectLocation": location,
            }))
            .unwrap(),
        )
        .unwrap();
        let prefs = Preferences::load(&dirs);
        assert_eq!(prefs.dark, Some(false));
        assert_eq!(prefs.project_location(), Some(location.as_str()));
        prefs.save(&dirs);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dirs.config.join(FILE)).unwrap()).unwrap();
        assert_eq!(saved["lastProjectLocation"], location);
        assert_eq!(
            Preferences::load(&dirs).project_location(),
            Some(location.as_str())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn a_missing_or_non_directory_project_location_uses_the_default() {
        let root = std::env::temp_dir().join(format!(
            "concat-project-location-fallback-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let location = root.join("projects");
        let prefs = Preferences {
            last_project_location: Some(location.to_string_lossy().into_owned()),
            ..Preferences::default()
        };
        assert_eq!(prefs.project_location(), None);
        std::fs::create_dir(&location).unwrap();
        assert_eq!(prefs.project_location(), location.to_str());
        std::fs::remove_dir(&location).unwrap();
        assert_eq!(prefs.project_location(), None);
        std::fs::write(&location, "a file, not a folder").unwrap();
        assert_eq!(prefs.project_location(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_settings_have_no_remembered_project_location() {
        let prefs: Preferences = serde_json::from_str(r#"{"dark": false}"#).unwrap();
        assert_eq!(prefs.dark, Some(false));
        assert_eq!(prefs.last_project_location, None);
        assert_eq!(prefs.project_location(), None);
    }
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
        assert_eq!(at(Some(110)), 2);
        assert_eq!(at(Some(200)), 6);
        assert_eq!(at(Some(133)), 0);
    }

    /// The row finds the percent back, row 0 is the display's own, and a
    /// row past the list is none of the app's.
    #[test]
    fn a_row_finds_its_scale() {
        assert_eq!(Preferences::scale_at(0), None);
        assert_eq!(Preferences::scale_at(1), Some(100));
        assert_eq!(Preferences::scale_at(2), Some(110));
        assert_eq!(Preferences::scale_at(6), Some(200));
        assert_eq!(Preferences::scale_at(7), None);
    }

    /// A scale reads as the multiplier the backend's `SLINT_SCALE_FACTOR`
    /// wants: the percent over a hundred.
    #[test]
    fn a_scale_reads_as_a_multiplier() {
        assert_eq!(Preferences::scale_factor(100), 1.0);
        assert_eq!(Preferences::scale_factor(110), 1.1);
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
        assert_eq!(back.scale_index(), 4);
        let older: Preferences = serde_json::from_str(r#"{"dark": true}"#).unwrap();
        assert_eq!(older.ui_scale, None);
        assert_eq!(older.scale_index(), 0);
    }

    /// The shortcuts a person rebound ride along, one chord per action,
    /// and a file from before the shortcuts page existed loads with the
    /// table as it ships.
    #[test]
    fn rebound_shortcuts_round_trip_and_an_older_file_has_none() {
        let mut prefs = Preferences {
            dark: Some(true),
            ..Preferences::default()
        };
        prefs.keybinds.insert("delete-left".into(), "j".into());
        prefs.keybinds.insert("mute".into(), String::new());
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains("\"keybinds\""), "{text}");
        let back: Preferences = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.keybinds.get("delete-left").map(String::as_str),
            Some("j")
        );
        assert_eq!(back.keybinds.get("mute").map(String::as_str), Some(""));
        let older: Preferences = serde_json::from_str(r#"{"dark": false}"#).unwrap();
        assert!(older.keybinds.is_empty());
    }

    /// A launch that died while starting puts the window in software for
    /// its build only: the next build tries the GPU again, and a death on
    /// that one puts it back. The person's own choice is never touched.
    #[test]
    fn a_crash_draws_in_software_until_another_build() {
        let mut prefs = Preferences::default();
        assert!(!prefs.settle_start(false, "a"));
        assert!(!prefs.draws_in_software());

        assert!(prefs.settle_start(true, "a"));
        assert!(prefs.draws_in_software());
        assert!(
            !prefs.settle_start(false, "a"),
            "same build stays in software"
        );
        assert!(
            !prefs.settle_start(true, "a"),
            "a death in software changes nothing"
        );
        assert!(prefs.draws_in_software());

        assert!(prefs.settle_start(false, "b"));
        assert!(!prefs.draws_in_software(), "a new build tries the GPU");
        assert!(prefs.settle_start(true, "b"));
        assert_eq!(prefs.software_after_crash.as_deref(), Some("b"));

        let mut chosen = Preferences {
            software_gpu: true,
            ..Preferences::default()
        };
        assert!(!chosen.settle_start(true, "a"));
        assert!(!chosen.settle_start(false, "b"));
        assert!(chosen.draws_in_software());
        assert_eq!(chosen.software_after_crash, None);
    }

    /// The `softwareGpu` key 0.2.8 wrote is let go, safe start's and the
    /// person's alike, so the GPU gets one more go; the new key round-trips.
    #[test]
    fn the_old_software_key_is_let_go() {
        let older: Preferences = serde_json::from_str(r#"{"softwareGpu": true}"#).unwrap();
        assert!(!older.draws_in_software());
        let prefs = Preferences {
            software_gpu: true,
            ..Preferences::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains("\"drawInSoftware\":true"), "{text}");
        let back: Preferences = serde_json::from_str(&text).unwrap();
        assert!(back.software_gpu);
    }
}
