// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The editor's whole API, for the native phone app, through one call.
//!
//! The phone app edits through the same API an AI agent does over MCP and a
//! script does over the CLI: `concat_api`'s requests, as JSON, one call
//! each, and its events - an export's progress, a cutout's - as JSON on a
//! callback. So a phone feature is an API method first, and every one the
//! app gains an agent gains too. Nothing here knows what a request means.
//!
//! One thing differs from the API's own replies: where one carries an
//! editor view, its `project` is the phone's view of the edit
//! ([`crate::view`]), made from the open session's document, not the
//! document's file. The app reads the edit in that one shape and never
//! the file format.
//!
//! The picture and the sound stay where they were, on [`crate::Engine`]:
//! the Surface it draws on and the clock it plays to are not requests.

use std::sync::{Arc, Mutex};

use concat_api::{Api, ApiError, AppDirs, Reply, Request, Response};
use serde_json::Value;

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
    let refused = |error: ApiError| written(&Response::Error(error));
    let request: Value = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => {
            return refused(ApiError::invalid(format!(
                "the request did not read: {error}"
            )));
        }
    };
    // The project folder the request names, whose open session a view is
    // read from.
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let request: Request = match serde_json::from_value(request) {
        Ok(request) => request,
        Err(error) => {
            return refused(ApiError::invalid(format!(
                "the request did not read: {error}"
            )));
        }
    };
    let Ok(mut api) = api.lock() else {
        return refused(ApiError::failed(
            "the editor's API stopped after an earlier failure; restart the app",
        ));
    };
    match api.dispatch(request) {
        Response::Result(Reply::View(view)) => {
            let session = path.as_deref().and_then(|path| api.open_session(path).ok());
            // A request that names no folder (`project.create`) has its
            // document read back from the view.
            let document = match session {
                Some(session) => session.document(),
                None => match view.document() {
                    Ok(document) => document,
                    Err(why) => return written(&crate::view::unread(why)),
                },
            };
            crate::view::reply(&view, &document).to_string()
        }
        response => written(&response),
    }
}

/// A reply as the JSON the app reads.
fn written(response: &Response) -> String {
    serde_json::to_string(response).unwrap_or_else(|error| {
        serde_json::json!({ "error": { "code": "failed", "message": format!("the reply did not write: {error}") } })
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn api() -> (Mutex<Api>, tempfile_dir::Dir) {
        api_with(Arc::new(|_| {}))
    }

    fn api_with(events: concat_api::EventSink) -> (Mutex<Api>, tempfile_dir::Dir) {
        let dir = tempfile_dir::Dir::new();
        let dirs = AppDirs {
            config: dir.path().join("config"),
            data: dir.path().join("data"),
        };
        (Mutex::new(Api::with_dirs(dirs, events)), dir)
    }

    /// One request as the app sends it, and the result it gets; a refusal
    /// fails the test.
    fn result(api: &Mutex<Api>, request: Value) -> Value {
        let reply: Value =
            serde_json::from_str(&call_with(api, &request.to_string())).expect("JSON");
        assert!(reply.get("error").is_none(), "{request} gave {reply}");
        reply["result"].clone()
    }

    fn edit(api: &Mutex<Api>, path: &str, command: Value) -> Value {
        result(
            api,
            json!({ "method": "edit.apply", "path": path, "command": command }),
        )
    }

    /// Makes a project as the app does, and says where it is: the reply
    /// names no folder, and recents put the new one first.
    fn create(api: &Mutex<Api>, dir: &tempfile_dir::Dir) -> (Value, String) {
        let created = result(
            api,
            json!({
                "method": "project.create",
                "location": dir.path(),
                "name": "Phone",
                "video": { "width": 1080, "height": 1920, "rateNum": 30, "rateDen": 1 },
            }),
        );
        let recents = result(api, json!({ "method": "project.list" }));
        let path = recents[0]["path"].as_str().expect("in recents").to_owned();
        (created, path)
    }

    /// The timeline the app shows: the one `activeTimelineId` names.
    fn showing(view: &Value) -> &Value {
        let active = &view["project"]["activeTimelineId"];
        view["project"]["timelines"]
            .as_array()
            .and_then(|all| all.iter().find(|timeline| &timeline["id"] == active))
            .expect("the timeline showing is listed")
    }

    fn clips<'a>(view: &'a Value, kind: &str) -> Vec<&'a Value> {
        let mut found: Vec<&Value> = showing(view)["clips"]
            .as_array()
            .expect("clips")
            .iter()
            .filter(|clip| clip["kind"] == kind)
            .collect();
        let start = |clip: &Value| clip["start"].as_f64().unwrap_or(f64::NAN);
        found.sort_by(|a, b| start(a).total_cmp(&start(b)));
        found
    }

    fn id(value: &Value) -> String {
        value.as_str().expect("an id is a string").to_owned()
    }

    fn near(value: &Value, expected: f64) -> bool {
        value.as_f64().is_some_and(|v| (v - expected).abs() < 1e-6)
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

    /// An edit with one of every kind of clip the phone shows, made
    /// through the requests the app sends: a video cut at twice the speed
    /// and trimmed, a second cut after it with a transition over the cut,
    /// the first one's sound detached, a title with words styled apart, a
    /// shape, a layer, a caption lane, effects and filters with keys, a
    /// mask, a crop, an animation, a cover, and a second timeline.
    fn everything(api: &Mutex<Api>, path: &str) -> Value {
        let media = id(&edit(
            api,
            path,
            json!({ "op": "addMedia", "item": {
                "path": "/footage/a.mp4", "name": "a.mp4", "duration": 10.0, "kind": "video",
                "width": 1920, "height": 1080, "frameRateFraction": "30/1", "hasAudio": true,
            }}),
        )["createdId"]);
        let view = edit(
            api,
            path,
            json!({ "op": "addClipsInSequence", "mediaIds": [media, media], "start": 0.0 }),
        );
        let cuts = clips(&view, "video");
        let (first, second) = (id(&cuts[0]["id"]), id(&cuts[1]["id"]));
        edit(
            api,
            path,
            json!({ "op": "setClipSpeed", "clipId": first, "speed": 2.0, "ripple": true }),
        );
        edit(
            api,
            path,
            json!({ "op": "trimClip", "clipId": first, "edge": "start", "delta": 0.5, "ripple": false }),
        );
        edit(
            api,
            path,
            json!({ "op": "setClipKey", "clipId": first, "property": "scale", "at": 0.5, "value": 1.5 }),
        );
        edit(
            api,
            path,
            json!({ "op": "updateClip", "clipId": first, "patch": {
                "videoEffects": [{ "id": "concat.warm", "params": { "amount": 0.5 },
                    "keys": { "amount": [{ "at": 0.0, "value": 0.0 }, { "at": 1.0, "value": 1.0 }] } }],
                "filters": [{ "id": "concat.echo", "params": { "mix": 0.3 }, "enabled": false }],
                "masks": [{ "name": "Heart", "source": { "kind": "shape", "shape": "heart", "width": 0.5, "height": 0.4 },
                    "feather": 0.2, "invert": true,
                    "keys": { "x": [{ "at": 0.0, "value": 0.0 }, { "at": 0.5, "value": 0.1 }] } }],
                "crop": { "left": 0.1, "top": 0.0, "right": 0.2, "bottom": 0.05 },
                "motion": { "in": { "kind": "fade", "seconds": 0.4 }, "loop": { "kind": "pulse", "seconds": 1.0, "amount": 0.5 } },
                "volume": 0.5, "fadeIn": 0.25, "flipH": true, "opacity": 0.8,
            }}),
        );
        edit(api, path, json!({ "op": "detachAudio", "clipId": first }));
        let now = result(api, json!({ "method": "project.get", "path": path }));
        let cut = clips(&now, "video")[1]["start"].as_f64().expect("a start");
        edit(
            api,
            path,
            json!({ "op": "addTransitionClip", "start": cut - 0.5, "transitionId": "concat.dissolve",
                "name": "Dissolve", "duration": 1.0 }),
        );
        edit(
            api,
            path,
            json!({ "op": "addTextClip", "start": 0.0, "duration": 2.0, "above": true, "style": {
                "content": "Hello world", "fontSize": 0.08, "color": "#ff0000",
                "runs": [{ "start": 6, "end": 11, "color": "#00ff00", "italic": true }],
            }}),
        );
        edit(
            api,
            path,
            json!({ "op": "addShapeClip", "start": 6.0, "duration": 1.0, "name": "Star", "above": true,
                "style": { "kind": "star" } }),
        );
        edit(
            api,
            path,
            json!({ "op": "addLayerClip", "start": 6.0, "duration": 1.0, "effectId": "concat.glow", "name": "Glow" }),
        );
        edit(
            api,
            path,
            json!({ "op": "placeCaptions", "look": { "style": { "fontSize": 0.06, "color": "#ffff00" }, "offsetY": 0.3 },
                "cues": [{ "start": 8.0, "duration": 1.0, "text": "Hi" }] }),
        );
        let view = edit(
            api,
            path,
            json!({ "op": "setCover", "cover": { "kind": "frame", "time": 1.5 } }),
        );
        // A second timeline, then the first shown again.
        let first_timeline = id(&view["project"]["activeTimelineId"]);
        edit(api, path, json!({ "op": "addTimeline" }));
        let view = edit(
            api,
            path,
            json!({ "op": "selectTimeline", "timelineId": first_timeline }),
        );
        assert_eq!(view["project"]["activeTimelineId"], json!(first_timeline));
        assert_eq!(id(&cuts[0]["id"]), first);
        assert_eq!(id(&cuts[1]["id"]), second);
        view
    }

    #[test]
    fn the_phone_reads_every_kind_of_clip_in_its_own_names_and_units() {
        let (api, dir) = api();
        let (_, path) = create(&api, &dir);
        let view = everything(&api, &path);
        let project = &view["project"];
        assert!(
            view.get("settings").is_some() && view["canUndo"] == true,
            "{view}"
        );

        // The timelines: both listed, only the one showing with its lanes.
        let timelines = project["timelines"].as_array().expect("timelines");
        assert_eq!(timelines.len(), 2);
        let other = timelines
            .iter()
            .find(|timeline| timeline["id"] != project["activeTimelineId"])
            .expect("the other timeline");
        assert!(
            other.get("clips").is_none() && other.get("tracks").is_none(),
            "{other}"
        );
        let timeline = showing(&view);
        assert_eq!(
            (&timeline["video"]["width"], &timeline["video"]["height"]),
            (&json!(1080), &json!(1920)),
            "the frame the project was made with"
        );
        assert_eq!(
            (&timeline["video"]["rateNum"], &timeline["video"]["rateDen"]),
            (&json!(30), &json!(1))
        );

        // The bin.
        let media = &project["media"][0];
        assert_eq!(media["path"], "/footage/a.mp4");
        assert_eq!(media["kind"], "video");
        assert!(near(&media["duration"], 10.0));
        assert_eq!(
            (&media["width"], &media["height"]),
            (&json!(1920), &json!(1080))
        );
        assert_eq!(project["cover"], json!({ "kind": "frame", "time": 1.5 }));

        // The video cut: twice the speed, its head trimmed half a second.
        let cut = clips(&view, "video")[0];
        assert_eq!(cut["mediaId"], media["id"]);
        assert_eq!(cut["name"], "a.mp4");
        assert!(near(&cut["speed"], 2.0), "{cut}");
        assert!(near(&cut["start"], 0.5), "{cut}");
        assert!(near(&cut["duration"], 4.5), "{cut}");
        assert!(near(&cut["sourceStart"], 1.0), "{cut}");
        assert!(
            near(&cut["volume"], 0.5) && near(&cut["opacity"], 0.8),
            "{cut}"
        );
        assert!(
            near(&cut["fadeIn"], 0.25) && near(&cut["fadeOut"], 0.0),
            "{cut}"
        );
        // Its sound plays from the clip detached from it.
        assert_eq!((&cut["flipH"], &cut["muted"]), (&json!(true), &json!(true)));
        assert!(
            near(&cut["scale"], 1.0) && near(&cut["offsetX"], 0.0),
            "{cut}"
        );
        assert_eq!(
            cut["keys"],
            json!([{ "property": "scale", "at": 0.5, "value": 1.5, "ease": [0.0, 0.0, 1.0, 1.0] }])
        );
        assert_eq!(
            cut["videoEffects"],
            json!([{ "id": "concat.warm", "params": { "amount": 0.5 }, "enabled": true,
                "keys": { "amount": [
                    { "at": 0.0, "value": 0.0, "ease": [0.0, 0.0, 1.0, 1.0] },
                    { "at": 1.0, "value": 1.0, "ease": [0.0, 0.0, 1.0, 1.0] } ] } }])
        );
        // Its sound chain went with its sound.
        assert_eq!(cut["filters"], json!([]));
        let mask = &cut["masks"][0];
        assert!(
            mask["id"].as_str().is_some_and(|id| !id.is_empty()),
            "{mask}"
        );
        assert_eq!(mask["source"]["kind"], "shape");
        assert_eq!(mask["source"]["shape"], "heart");
        assert_eq!(
            (&mask["invert"], &mask["name"]),
            (&json!(true), &json!("Heart"))
        );
        assert!(near(&mask["feather"], 0.2) && near(&mask["source"]["width"], 0.5));
        assert!(near(&mask["keys"]["x"][1]["at"], 0.5), "{mask}");
        assert_eq!(
            cut["crop"],
            json!({ "left": 0.1, "top": 0.0, "right": 0.2, "bottom": 0.05 })
        );
        assert_eq!(
            cut["motion"]["in"],
            json!({ "kind": "fade", "seconds": 0.4 })
        );
        assert_eq!(
            cut["motion"]["loop"],
            json!({ "kind": "pulse", "seconds": 1.0, "amount": 0.5 })
        );
        assert!(cut["motion"]["out"].is_null(), "{cut}");
        let second = clips(&view, "video")[1];
        assert!(
            second.get("crop").is_none() && second.get("motion").is_none(),
            "{second}"
        );
        assert_eq!(second["masks"], json!([]));

        // The sound detached from it, on a lane of its own.
        let sound = clips(&view, "audio")[0];
        assert_eq!(sound["mediaId"], media["id"]);
        assert_ne!(sound["trackId"], cut["trackId"]);
        assert!(
            near(&sound["start"], 0.5) && near(&sound["sourceStart"], 1.0),
            "{sound}"
        );
        assert!(
            near(&sound["speed"], 2.0) && near(&sound["volume"], 0.5),
            "{sound}"
        );
        assert_eq!(sound["muted"], false);
        assert_eq!(
            sound["filters"],
            json!([{ "id": "concat.echo", "params": { "mix": 0.3 }, "enabled": false, "keys": {} }])
        );

        // The transition over the cut between the two.
        let transition = clips(&view, "transition")[0];
        assert_eq!(transition["name"], "Dissolve");
        assert!(near(&transition["duration"], 1.0), "{transition}");
        assert_eq!(transition["videoEffects"][0]["id"], "concat.dissolve");
        assert!(transition.get("mediaId").is_none(), "{transition}");

        // The title, its last word styled apart.
        let title = clips(&view, "text")
            .into_iter()
            .find(|clip| clip["text"]["content"] == "Hello world")
            .expect("the title");
        assert_eq!(title["text"]["color"], "#ff0000");
        assert!(near(&title["text"]["fontSize"], 0.08));
        assert_eq!(title["text"]["runs"][0]["start"], 6);
        assert_eq!(title["text"]["runs"][0]["color"], "#00ff00");
        assert_eq!(title["text"]["runs"][0]["italic"], true);
        assert_eq!(title["name"], "Hello world");

        // The shape, the layer and its effect.
        let shape = clips(&view, "shape")[0];
        assert_eq!(shape["name"], "Star");
        assert!(shape.get("text").is_none(), "{shape}");
        let layer = clips(&view, "layer")[0];
        assert_eq!(layer["videoEffects"][0]["id"], "concat.glow");

        // The caption lane's look, and its caption.
        let tracks = timeline["tracks"].as_array().expect("tracks");
        let captions = tracks
            .iter()
            .find(|track| track.get("captions").is_some())
            .expect("a caption lane");
        assert_eq!(captions["captions"]["style"]["color"], "#ffff00");
        assert!(near(&captions["captions"]["offsetY"], 0.3));
        assert_eq!(captions["captions"]["style"]["content"], "");
        assert!(
            clips(&view, "text")
                .iter()
                .any(|clip| clip["trackId"] == captions["id"] && clip["text"]["content"] == "Hi")
        );

        // Lanes bottom first: the cuts are on the lowest, the caption lane,
        // made last on top, is the last.
        assert_eq!(tracks[0]["id"], cut["trackId"]);
        assert_eq!(
            tracks.last().map(|track| &track["id"]),
            Some(&captions["id"])
        );
        assert!(
            tracks
                .iter()
                .all(|track| track["visible"] == true && track["muted"] == false)
        );

        // Every clip on a lane listed, with every field the app reads.
        let lanes: Vec<&Value> = tracks.iter().map(|track| &track["id"]).collect();
        for clip in showing(&view)["clips"].as_array().expect("clips") {
            assert!(lanes.contains(&&clip["trackId"]), "{clip}");
            for field in [
                "id",
                "kind",
                "name",
                "start",
                "duration",
                "sourceStart",
                "volume",
                "speed",
                "videoEffects",
                "filters",
                "scale",
                "offsetX",
                "offsetY",
                "rotation",
                "opacity",
                "muted",
                "flipH",
                "fadeIn",
                "fadeOut",
                "keys",
                "masks",
            ] {
                assert!(
                    clip.get(field).is_some_and(|v| !v.is_null()),
                    "{field} of {clip}"
                );
            }
        }
    }

    #[test]
    fn what_the_phone_reads_sent_back_whole_changes_nothing() {
        let (api, dir) = api();
        let (_, path) = create(&api, &dir);
        let before = everything(&api, &path);
        let cut = clips(&before, "video")[0];
        let title = clips(&before, "text")
            .into_iter()
            .find(|clip| clip["text"]["content"] == "Hello world")
            .expect("the title");
        let lane = showing(&before)["tracks"]
            .as_array()
            .and_then(|tracks| tracks.iter().find(|track| track.get("captions").is_some()))
            .expect("a caption lane");
        let after = edit(
            &api,
            &path,
            json!({ "op": "batch", "commands": [
                { "op": "updateClip", "clipId": cut["id"], "patch": {
                    "videoEffects": cut["videoEffects"], "filters": cut["filters"],
                    "masks": cut["masks"], "crop": cut["crop"], "motion": cut["motion"],
                }},
                { "op": "updateClip", "clipId": title["id"], "patch": { "text": title["text"] } },
                { "op": "setCaptionLook", "trackId": lane["id"], "look": lane["captions"] },
                { "op": "setCover", "cover": before["project"]["cover"] },
            ]}),
        );
        assert_eq!(after["project"], before["project"]);
    }

    /// What `Smoke.kt` asks of the engine on CI's emulators, in its order,
    /// with the view read after each as the app reads it.
    #[test]
    fn the_smoke_export_runs_on_what_the_phone_reads() {
        let (events, heard) = std::sync::mpsc::channel::<Value>();
        let events = Mutex::new(events);
        let (api, dir) = api_with(Arc::new(move |event| {
            if let (Ok(event), Ok(events)) = (serde_json::to_value(&event), events.lock()) {
                let _ = events.send(event);
            }
        }));
        let (created, path) = create(&api, &dir);
        assert_eq!(created["settings"]["name"], "Phone");
        assert_eq!(showing(&created)["clips"], json!([]));
        assert!(
            !showing(&created)["tracks"]
                .as_array()
                .expect("tracks")
                .is_empty()
        );

        let clip = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../concat-media/fixtures/picture-starts-late.mov")
            .canonicalize()
            .expect("the fixture");
        let clip = clip.to_string_lossy();
        let imported = result(
            &api,
            json!({ "method": "media.import", "path": path, "file": clip }),
        );
        let media = imported["project"]["media"]
            .as_array()
            .and_then(|bin| bin.iter().find(|media| media["path"] == clip.as_ref()))
            .expect("the bin has the clip, at the path imported");
        assert_eq!(media["kind"], "video");

        let placed = edit(
            &api,
            &path,
            json!({ "op": "addClipsInSequence", "mediaIds": [media["id"]], "start": 0.0,
                "trackId": null, "ripple": false }),
        );
        let on = clips(&placed, "video");
        assert_eq!(on.len(), 1, "{placed}");
        assert_eq!(on[0]["mediaId"], media["id"]);
        assert!(
            on[0]["duration"].as_f64().is_some_and(|d| d > 0.0),
            "{placed}"
        );

        result(&api, json!({ "method": "project.save", "path": path }));
        let output = dir.path().join("smoke.mov");
        let started = result(
            &api,
            json!({ "method": "export.run", "path": path, "output": output, "width": 1080,
                "height": 1920, "crf": 23, "preset": "veryfast", "codec": "h264" }),
        );
        let job = started["job"].clone();
        assert!(job.is_string(), "{started}");
        let ended = loop {
            let event = heard
                .recv_timeout(std::time::Duration::from_secs(300))
                .expect("the export ends");
            if event["job"] == job
                && (event["event"] == "export.done" || event["event"] == "export.failed")
            {
                break event;
            }
        };
        assert_eq!(ended["event"], "export.done", "{ended}");
        assert!(std::fs::metadata(&output).is_ok_and(|file| file.len() > 0));
    }
}
