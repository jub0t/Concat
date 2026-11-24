// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The editor's whole API, for the native phone app, through one call.
//!
//! The phone app edits through the same API an AI agent does over MCP and a
//! script does over the CLI: `concat_api`'s requests, as JSON, one call
//! each, and its events - an export's progress, a cutout's - as JSON on a
//! callback. So a phone feature is an API method first, and every one the
//! app gains an agent gains too. Kotlin reads and writes the JSON with types
//! generated from `concat --schema`; nothing here knows what a request
//! means.
//!
//! The picture and the sound stay where they were, on [`crate::Engine`]:
//! the Surface it draws on and the clock it plays to are not requests.

use std::sync::{Arc, Mutex};

use concat_api::{Api, ApiError, AppDirs, Request, Response};

/// Where the API's events go: each one as a JSON object, on whichever
/// thread the job that raised it runs.
#[uniffi::export(callback_interface)]
pub trait ApiEvents: Send + Sync {
    /// One event: `{"event": "export.progress", ...}` and the like.
    fn event(&self, json: String);
}

/// The phone's own text-to-speech, as the app hands it over: the read-aloud
/// "Phone voice". The engine calls [`PhoneVoice::say`] on a worker thread,
/// once per piece of the text, and joins the files.
#[uniffi::export(callback_interface)]
pub trait PhoneVoice: Send + Sync {
    /// Says `text` into a WAV at `path`, at `rate` times the voice's own
    /// speed, and returns once the file is written: `None`, or why not.
    fn say(&self, text: String, path: String, rate: f32) -> Option<String>;
}

/// Offers `voice` to read aloud with, from now on. The app calls it once its
/// text-to-speech has started, so a phone with none never lists a voice
/// that cannot read; the first call wins.
#[uniffi::export]
pub fn set_phone_voice(voice: Box<dyn PhoneVoice>) {
    concat_speech::tts::set_system_voice(Box::new(
        move |text: &str, path: &std::path::Path, rate: f32| match voice.say(
            text.to_owned(),
            path.to_string_lossy().into_owned(),
            rate,
        ) {
            None => Ok(()),
            Some(why) => Err(why),
        },
    ));
}

/// How the phone stays awake while a job runs: an export, a transcription
/// or a download takes minutes, and a screen that timed out half a minute
/// in took the work down with it (#695).
#[uniffi::export(callback_interface)]
pub trait KeepAwake: Send + Sync {
    /// `true` when the first job starts, `false` when the last one ends.
    fn stay_awake(&self, on: bool);
}

/// Hands the engine the phone's way of staying awake, for every job from
/// now on. The first call wins.
#[uniffi::export]
pub fn set_keep_awake(hook: Box<dyn KeepAwake>) {
    concat_host::awake::install(Box::new(move |on| hook.stay_awake(on)));
}

/// The editor's API, as the phone app holds it: one per process.
#[derive(uniffi::Object)]
pub struct EditorApi {
    api: Arc<Mutex<Api>>,
}

#[uniffi::export]
impl EditorApi {
    /// The API over the app's own folders: `config_dir` for settings,
    /// recents and templates, `data_dir` for models and caches. Events go
    /// to `events`.
    #[uniffi::constructor]
    pub fn new(config_dir: String, data_dir: String, events: Box<dyn ApiEvents>) -> Arc<Self> {
        let events: Arc<dyn ApiEvents> = Arc::from(events);
        let sink: concat_api::EventSink =
            Arc::new(move |event| match serde_json::to_string(&event) {
                Ok(json) => events.event(json),
                Err(error) => log::warn!("api: an event did not write as JSON: {error}"),
            });
        let dirs = AppDirs {
            config: config_dir.into(),
            data: data_dir.into(),
        };
        // As the Slint app's settings pane does on open: decode on the
        // phone's MediaCodec, which takes files its CPU would not.
        #[cfg(target_os = "android")]
        concat_media::set_hardware_decode(true);
        Arc::new(EditorApi {
            api: Arc::new(Mutex::new(Api::with_dirs(dirs, sink))),
        })
    }

    /// One request, `{"method": "project.open", ...}`, and its reply:
    /// `{"result": ...}` or `{"error": {"code": ..., "message": ...}}`.
    /// Never throws: a request that does not read is an error reply.
    /// Blocking; call it off the main thread.
    pub fn call(&self, request: String) -> String {
        call_with(&self.api, &request)
    }

    /// The picture and sound of `project_path`, which must be open here
    /// (`project.open` or `project.create` first). One at a time is what a
    /// phone wants: each holds a GPU device and an audio stream.
    pub fn preview(&self, project_path: String) -> Result<Arc<crate::Preview>, crate::MobileError> {
        crate::Preview::open(Arc::clone(&self.api), project_path)
    }
}

/// [`EditorApi::call`] on an API given.
fn call_with(api: &Mutex<Api>, request: &str) -> String {
    let response = match serde_json::from_str::<Request>(request) {
        Ok(request) => match api.lock() {
            Ok(mut api) => api.dispatch(request),
            Err(_) => Response::Error(ApiError::failed(
                "the editor's API stopped after an earlier failure; restart the app",
            )),
        },
        Err(error) => Response::Error(ApiError::invalid(format!(
            "the request did not read: {error}"
        ))),
    };
    serde_json::to_string(&response).unwrap_or_else(|error| {
        serde_json::json!({ "error": { "code": "failed", "message": format!("the reply did not write: {error}") } })
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> (Mutex<Api>, tempfile_dir::Dir) {
        let dir = tempfile_dir::Dir::new();
        let dirs = AppDirs {
            config: dir.path().join("config"),
            data: dir.path().join("data"),
        };
        (Mutex::new(Api::with_dirs(dirs, Arc::new(|_| {}))), dir)
    }

    /// A scratch folder that goes when the test does, without a crate for it.
    mod tempfile_dir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicU32, Ordering};
                static NEXT: AtomicU32 = AtomicU32::new(0);
                let path = std::env::temp_dir().join(format!(
                    "concat-mobile-api-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).expect("a scratch folder");
                Dir(path)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn a_request_gets_its_reply_as_json() {
        let (api, _dir) = api();
        let reply: serde_json::Value =
            serde_json::from_str(&call_with(&api, r#"{"method":"version"}"#)).expect("JSON");
        assert!(reply.get("result").is_some(), "{reply}");
    }

    #[test]
    fn a_request_that_does_not_read_is_an_error_reply_not_a_throw() {
        let (api, _dir) = api();
        for bad in ["", "{", r#"{"method":"no.such.method"}"#, "[1,2]"] {
            let reply: serde_json::Value =
                serde_json::from_str(&call_with(&api, bad)).expect("JSON");
            assert!(reply.get("error").is_some(), "{bad:?} gave {reply}");
        }
    }
}
