// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The settings sheet: the preferences, the two model lists, and the API
//! server's switch.
//!
//! Everything on the sheet is a preference or follows from one, so most
//! messages do three things: change the pane, write the preference, and
//! apply it - to the downloaders, to the server, to the words on screen.
//! The model lists are the one part with a worker behind them: a download
//! reports as [`SettingsMsg::ModelProgress`] and ends as
//! [`SettingsMsg::ModelFinished`], and the lists are rebuilt from what is
//! on disk after anything that could have changed it.

use std::collections::HashMap;
use std::sync::Arc;

use concat_host::models::SourcePreference;
use slint::SharedString;

use crate::host::{on_ui, spawn};
use crate::i18n::{self, t, tf};
use crate::panes::Msg;
use crate::platform;
use crate::prefs;
use crate::studio::Studio;
use crate::ui::{ModelData, SettingsData};

/// Everything that can happen to the settings sheet.
#[derive(Clone, Debug)]
pub enum SettingsMsg {
    /// The preferences read back on launch, and applied.
    Restore,
    /// The sheet is asked for: the menu, the tray or the shortcut.
    Open,
    Close,
    PageChanged(i32),
    /// Show this run's log in the file manager.
    ShowLog,
    LanguageChanged(i32),
    PlayheadStopsChanged(bool),
    CustomContextActionsChanged(bool),
    HardwareDecodeChanged(bool),
    DownloadSourceChanged(i32),
    DownloadBaseEdited(String),
    ServerEnabledChanged(bool),
    ServerListenEdited(String),
    ServerTokenEdited(String),
    ServerTokenGenerated,
    /// Make this the model the engine uses.
    ModelActivated(String),
    ModelDownload(String),
    ModelCancel(String),
    ModelRemove(String),
    /// A download's worker reporting where it is.
    ModelProgress {
        id: String,
        /// Megabytes so far.
        fetched: f32,
        /// Megabytes in all, when the server said.
        total: Option<f32>,
        unpacking: bool,
    },
    /// A download's worker is done: the model on disk, or why not.
    ModelFinished {
        id: String,
        result: Result<(), String>,
    },
}

/// One downloadable model, as the settings sheet shows it.
#[derive(Clone, Debug)]
pub struct ModelState {
    pub id: String,
    pub name: String,
    pub note: String,
    pub megabytes: f32,
    pub accuracy: i32,
    pub installed: bool,
    pub active: bool,
    /// Megabytes fetched so far while a download runs.
    pub fetched: Option<f32>,
    pub unpacking: bool,
}

impl ModelState {
    /// The model as one row of the sheet's list.
    fn row(&self) -> ModelData {
        let total = self.megabytes;
        let fetched = self.fetched.unwrap_or(0.0);
        ModelData {
            id: self.id.as_str().into(),
            name: self.name.as_str().into(),
            note: self.note.as_str().into(),
            size: format!("{total:.0} MB").into(),
            accuracy: self.accuracy,
            installed: self.installed,
            active: self.active && self.installed,
            downloading: self.fetched.is_some(),
            progress: if total > 0.0 {
                (fetched / total).min(1.0)
            } else {
                0.0
            },
            transferred: if self.unpacking {
                t("Unpacking…").into()
            } else {
                tf(
                    "{0} MB of {1} MB",
                    &[&format!("{fetched:.0}"), &format!("{total:.0}")],
                )
                .into()
            },
            eta: SharedString::new(),
        }
    }
}

/// The models of a kind that are on disk, in the settings' order: the
/// rows of a sheet's model list.
pub fn installed(models: &[ModelState]) -> Vec<&ModelState> {
    models.iter().filter(|model| model.installed).collect()
}

/// The settings sheet's state.
#[derive(Default)]
pub struct SettingsPane {
    pub open: bool,
    pub tab: i32,
    pub language: usize,
    /// The switch that keeps the playhead inside the content.
    pub playhead_stops: bool,
    /// Show flip/reverse in clip context menu.
    pub custom_context_actions: bool,
    /// Video decodes on the platform's hardware.
    pub hardware_decode: bool,
    /// Index into `SourcePreference::ALL`: where model downloads look first.
    pub download_source: usize,
    /// The base URL of a custom download source.
    pub download_base: String,
    /// Why the server is not running when the switch is on: the bind that
    /// failed. Empty while it runs, or is off.
    pub server_error: String,
    /// The transcriber models, installed or not.
    pub transcribers: Vec<ModelState>,
    /// The voice models, installed or not.
    pub voices: Vec<ModelState>,
}

impl SettingsPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SettingsMsg, studio: &mut Studio) {
        match msg {
            SettingsMsg::Restore => {
                self.language = studio
                    .languages
                    .iter()
                    .position(|language| {
                        Some(language.code.as_str()) == studio.prefs.locale.as_deref()
                    })
                    .unwrap_or(0);
                self.playhead_stops = studio.prefs.playhead_stops_at_end;
                self.custom_context_actions = studio.prefs.custom_context_actions;
                self.hardware_decode = studio.prefs.hardware_decode_on();
                concat_media::set_hardware_decode(self.hardware_decode);
                self.download_source = Self::download_source(studio).0;
                self.download_base = studio.prefs.download_base.clone().unwrap_or_default();
                Self::apply_download_source(studio);
                self.apply_server(studio);
                self.refresh(studio);
            }
            SettingsMsg::Open => {
                self.refresh(studio);
                self.open = true;
            }
            SettingsMsg::Close => self.open = false,
            SettingsMsg::PageChanged(index) => self.tab = index,
            SettingsMsg::ShowLog => {
                // The file this run is writing, when there is one, so the
                // manager opens with it selected; the folder when there is
                // not, which is still where the previous runs are.
                let target = concat_host::logs::current()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| concat_host::logs::folder(&studio.host.dirs));
                if let Err(error) = platform::reveal(&target.to_string_lossy()) {
                    studio.notify(&i18n::tf("Could not show the log: {0}", &[&error]), true);
                }
            }
            SettingsMsg::LanguageChanged(index) => {
                let index = index.max(0) as usize;
                if let Some(language) = studio.languages.get(index).cloned() {
                    self.language = index;
                    studio.prefs.locale = Some(language.code.clone());
                    // The words change on the publish that follows: Rust's
                    // on their way through `t`, the tree's through
                    // `I18n.lang`.
                    i18n::select(&language.code, &studio.host.dirs);
                }
                studio.prefs.save(&studio.host.dirs);
            }
            SettingsMsg::PlayheadStopsChanged(on) => {
                self.playhead_stops = on;
                studio.prefs.playhead_stops_at_end = on;
                studio.prefs.save(&studio.host.dirs);
                // A playhead already out past the end comes back in when
                // the switch goes on; seek does the clamp.
                let at = studio.playhead;
                studio.seek(at);
            }
            SettingsMsg::CustomContextActionsChanged(on) => {
                self.custom_context_actions = on;
                studio.prefs.custom_context_actions = on;
                studio.prefs.save(&studio.host.dirs);
            }
            SettingsMsg::HardwareDecodeChanged(on) => {
                self.hardware_decode = on;
                studio.prefs.hardware_decode = Some(on);
                studio.prefs.save(&studio.host.dirs);
                // Readers already open keep what they opened with; the
                // monitor's next frame opens fresh ones.
                concat_media::set_hardware_decode(on);
                studio.host.monitor.clear();
                studio.request_preview();
            }
            SettingsMsg::DownloadSourceChanged(index) => {
                let index = (index.max(0) as usize).min(SourcePreference::ALL.len() - 1);
                self.download_source = index;
                studio.prefs.download_source = Some(SourcePreference::ALL[index].name().to_owned());
                studio.prefs.save(&studio.host.dirs);
                Self::apply_download_source(studio);
            }
            SettingsMsg::DownloadBaseEdited(text) => {
                let base = text.trim().to_owned();
                self.download_base = base.clone();
                studio.prefs.download_base = (!base.is_empty()).then_some(base);
                studio.prefs.save(&studio.host.dirs);
                Self::apply_download_source(studio);
            }
            SettingsMsg::ServerEnabledChanged(on) => {
                studio.prefs.server.enabled = on;
                studio.prefs.save(&studio.host.dirs);
                self.apply_server(studio);
            }
            SettingsMsg::ServerListenEdited(text) => {
                let listen = text.trim().to_owned();
                studio.prefs.server.listen = if listen.is_empty() {
                    prefs::DEFAULT_LISTEN.to_owned()
                } else {
                    listen
                };
                studio.prefs.save(&studio.host.dirs);
                self.apply_server(studio);
            }
            SettingsMsg::ServerTokenEdited(text) => {
                studio.prefs.server.token = text.trim().to_owned();
                studio.prefs.save(&studio.host.dirs);
                self.apply_server(studio);
            }
            SettingsMsg::ServerTokenGenerated => {
                studio.prefs.server.token = new_token();
                studio.prefs.save(&studio.host.dirs);
                self.apply_server(studio);
            }
            SettingsMsg::ModelActivated(id) => {
                if self.is_transcriber(&id) {
                    studio.prefs.transcriber_model = Some(id);
                } else {
                    studio.prefs.tts_model = Some(id);
                }
                studio.prefs.save(&studio.host.dirs);
                self.refresh(studio);
            }
            SettingsMsg::ModelDownload(id) => self.download(&id, studio),
            SettingsMsg::ModelCancel(id) => {
                if self.is_transcriber(&id) {
                    studio.host.transcriber.cancel_download();
                } else {
                    studio.host.speech.cancel_download();
                }
            }
            SettingsMsg::ModelRemove(id) => {
                let result = if self.is_transcriber(&id) {
                    studio.host.transcriber.delete_model(&studio.host.dirs, &id)
                } else {
                    studio.host.speech.delete_model(&studio.host.dirs, &id)
                };
                if let Err(error) = result {
                    studio.notify(&error, true);
                }
                self.refresh(studio);
            }
            SettingsMsg::ModelProgress {
                id,
                fetched,
                total,
                unpacking,
            } => {
                if let Some(model) = self.model_mut(&id) {
                    model.fetched = Some(fetched);
                    model.unpacking = unpacking;
                    if let Some(total) = total {
                        model.megabytes = total;
                    }
                }
            }
            SettingsMsg::ModelFinished { id, result } => {
                if let Some(model) = self.model_mut(&id) {
                    model.fetched = None;
                    model.unpacking = false;
                }
                match result {
                    Ok(()) => {
                        studio.notify(&t("Model ready"), false);
                        if self.is_transcriber(&id) && studio.prefs.transcriber_model.is_none() {
                            studio.prefs.transcriber_model = Some(id.clone());
                        } else if !self.is_transcriber(&id) && studio.prefs.tts_model.is_none() {
                            studio.prefs.tts_model = Some(id.clone());
                        }
                        studio.prefs.save(&studio.host.dirs);
                    }
                    Err(error) => studio.notify(&error, true),
                }
                self.refresh(studio);
            }
        }
    }

    /// The remembered download source: its place in the menu, and itself.
    pub fn download_source(studio: &Studio) -> (usize, SourcePreference) {
        let preference =
            SourcePreference::parse(studio.prefs.download_source.as_deref().unwrap_or_default());
        let index = SourcePreference::ALL
            .iter()
            .position(|candidate| *candidate == preference)
            .unwrap_or(0);
        (index, preference)
    }

    /// Tells the downloaders where to look first, from the preferences.
    fn apply_download_source(studio: &Studio) {
        let (_, preference) = Self::download_source(studio);
        concat_host::models::set_preference(
            preference,
            studio.prefs.download_base.as_deref().unwrap_or_default(),
        );
    }

    /// Starts or stops the API's server to match the preferences. A server
    /// already running is stopped first, so an edited address or token
    /// takes effect; a bind that fails turns the switch back off and says
    /// why on the page.
    fn apply_server(&mut self, studio: &mut Studio) {
        if let Some(server) = studio.host.server.take() {
            server.stop();
        }
        self.server_error.clear();
        let prefs = &studio.prefs.server;
        if !prefs.enabled {
            return;
        }
        let started = prefs
            .listen
            .trim()
            .parse::<std::net::SocketAddr>()
            .map_err(|_| {
                tf(
                    "{0} is not an address like 127.0.0.1:7420",
                    &[&prefs.listen],
                )
            })
            .and_then(|address| {
                let config = concat_server::Config {
                    json: Some(address),
                    token: Some(prefs.token.clone()).filter(|token| !token.is_empty()),
                    ..concat_server::Config::default()
                };
                concat_server::Server::start(config, concat_api::Api::new)
            });
        match started {
            Ok(server) => studio.host.server = Some(server),
            Err(error) => {
                studio.prefs.server.enabled = false;
                studio.prefs.save(&studio.host.dirs);
                self.server_error = error.clone();
                studio.notify(&error, true);
            }
        }
    }

    /// What the Remote page says under the switch.
    fn server_status(&self, studio: &Studio) -> String {
        match &studio.host.server {
            Some(server) => {
                let address = server
                    .json_addr()
                    .map(|address| address.to_string())
                    .unwrap_or_default();
                tf(
                    "Listening on {0} · {1} connected",
                    &[&address, &server.connections()],
                )
            }
            None if !self.server_error.is_empty() => self.server_error.clone(),
            None => t("Off"),
        }
    }

    /// The two model lists, from what is on disk. A download in flight
    /// keeps its progress across the rebuild.
    fn refresh(&mut self, studio: &mut Studio) {
        let dirs = &studio.host.dirs;
        let downloading: HashMap<String, (Option<f32>, bool)> = self
            .transcribers
            .iter()
            .chain(self.voices.iter())
            .filter(|model| model.fetched.is_some())
            .map(|model| (model.id.clone(), (model.fetched, model.unpacking)))
            .collect();
        let chosen_transcriber = studio.prefs.transcriber_model.clone();
        let chosen_voice = studio.prefs.tts_model.clone();
        if let Ok(status) = concat_speech::Transcriber::status(dirs) {
            self.transcribers = status
                .models
                .iter()
                .map(|model| {
                    let (fetched, unpacking) =
                        downloading.get(&model.id).copied().unwrap_or((None, false));
                    ModelState {
                        id: model.id.clone(),
                        name: model.label.clone(),
                        note: model.blurb.clone(),
                        megabytes: model.size_bytes as f32 / 1_000_000.0,
                        accuracy: if model.id.starts_with("tiny") {
                            2
                        } else if model.id.starts_with("base") {
                            3
                        } else {
                            4
                        },
                        installed: model.downloaded,
                        active: chosen_transcriber.as_deref() == Some(model.id.as_str()),
                        fetched,
                        unpacking,
                    }
                })
                .collect();
        }
        if let Ok(status) = concat_speech::Speech::status(dirs) {
            studio.speech.speakers = status.voices.clone();
            self.voices = status
                .models
                .iter()
                .map(|model| {
                    let (fetched, unpacking) =
                        downloading.get(&model.id).copied().unwrap_or((None, false));
                    ModelState {
                        id: model.id.clone(),
                        name: model.label.clone(),
                        note: model.blurb.clone(),
                        megabytes: model.size_bytes as f32 / 1_000_000.0,
                        accuracy: if model.id.contains("int8") { 4 } else { 5 },
                        installed: model.downloaded,
                        active: chosen_voice.as_deref() == Some(model.id.as_str()),
                        fetched,
                        unpacking,
                    }
                })
                .collect();
        }
        // An engine with nothing chosen falls back to whatever is installed,
        // rather than silently having no model at all.
        for list in [&mut self.transcribers, &mut self.voices] {
            if !list.iter().any(|model| model.active && model.installed)
                && let Some(first) = list.iter_mut().find(|model| model.installed)
            {
                first.active = true;
            }
        }
    }

    fn is_transcriber(&self, id: &str) -> bool {
        self.transcribers.iter().any(|model| model.id == id)
    }

    fn model_mut(&mut self, id: &str) -> Option<&mut ModelState> {
        self.transcribers
            .iter_mut()
            .chain(self.voices.iter_mut())
            .find(|model| model.id == id)
    }

    /// Starts a download on a worker. Its reports come back as messages.
    fn download(&mut self, id: &str, studio: &Studio) {
        let transcriber = self.is_transcriber(id);
        let Some(model) = self.model_mut(id) else {
            return;
        };
        if model.installed || model.fetched.is_some() {
            return;
        }
        model.fetched = Some(0.0);
        let id = id.to_owned();
        let dirs = studio.host.dirs.clone();
        let whisper = Arc::clone(&studio.host.transcriber);
        let kokoro = Arc::clone(&studio.host.speech);
        spawn(
            move || {
                let report = |progress: concat_speech::DownloadProgress| {
                    let msg = SettingsMsg::ModelProgress {
                        id: progress.id,
                        fetched: progress.received as f32 / 1_000_000.0,
                        total: (progress.total > 0).then(|| progress.total as f32 / 1_000_000.0),
                        unpacking: progress.unpacking,
                    };
                    on_ui(move |studio, _, _| studio.handle(Msg::Settings(msg)));
                };
                let result = if transcriber {
                    whisper.download_model(&dirs, &id, report)
                } else {
                    kokoro.download_model(&dirs, &id, report)
                };
                (id, result)
            },
            |studio, _, _, (id, result)| {
                studio.handle(Msg::Settings(SettingsMsg::ModelFinished { id, result }));
            },
        );
    }

    /// The transcriber list's rows.
    pub fn transcriber_rows(&self) -> Vec<ModelData> {
        self.transcribers.iter().map(ModelState::row).collect()
    }

    /// The voice list's rows.
    pub fn voice_rows(&self) -> Vec<ModelData> {
        self.voices.iter().map(ModelState::row).collect()
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> SettingsData {
        SettingsData {
            open: self.open,
            tab: self.tab,
            language: self.language as i32,
            playhead_stops: self.playhead_stops,
            custom_context_actions: self.custom_context_actions,
            hardware_decode: self.hardware_decode,
            hardware_decode_offered: concat_media::HwDevice::platform_default()
                .is_some_and(concat_media::HwDevice::linked),
            download_source: self.download_source as i32,
            download_base: self.download_base.as_str().into(),
            server_enabled: studio.prefs.server.enabled,
            server_listen: studio.prefs.server.listen.as_str().into(),
            server_token: match &studio.host.server {
                // A token the server minted for itself is shown where a
                // chosen one would be typed: it is how a caller gets in.
                Some(server) if studio.prefs.server.token.is_empty() => server.token().into(),
                _ => studio.prefs.server.token.as_str().into(),
            },
            server_status: self.server_status(studio).into(),
            disk: {
                let on_disk: Vec<&ModelState> = installed(&self.transcribers)
                    .into_iter()
                    .chain(installed(&self.voices))
                    .collect();
                let megabytes: f32 = on_disk.iter().map(|model| model.megabytes).sum();
                tf(
                    "{0} installed · {1} MB on disk",
                    &[&on_disk.len(), &format!("{megabytes:.0}")],
                )
                .into()
            },
            version: env!("CARGO_PKG_VERSION").into(),
            engine: format!("concat-engine · FFmpeg {}", concat_media::linked_version()).into(),
        }
    }
}

/// A fresh token: 128 bits from the OS's randomness, as the standard
/// library hands it out through its hasher's seed, spelled in hex.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let word = |salt: u64| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.finish()
    };
    format!("{:016x}{:016x}", word(1), word(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_thirty_two_hex_digits_and_never_the_same_twice() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_row_shows_the_download_as_a_fraction_of_the_whole() {
        let model = ModelState {
            id: "base".into(),
            name: "Base".into(),
            note: String::new(),
            megabytes: 200.0,
            accuracy: 3,
            installed: false,
            active: true,
            fetched: Some(50.0),
            unpacking: false,
        };
        let row = model.row();
        assert!(row.downloading);
        assert!(!row.active, "a model that is not installed is not active");
        assert!((row.progress - 0.25).abs() < 1e-6);
    }

    #[test]
    fn installed_keeps_the_settings_order() {
        let model = |id: &str, installed: bool| ModelState {
            id: id.into(),
            name: id.into(),
            note: String::new(),
            megabytes: 1.0,
            accuracy: 1,
            installed,
            active: false,
            fetched: None,
            unpacking: false,
        };
        let list = [model("a", true), model("b", false), model("c", true)];
        let ids: Vec<&str> = installed(&list).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
    }
}
