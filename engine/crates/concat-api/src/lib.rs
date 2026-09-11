// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Concat API: the one dispatcher every way of driving the editor
//! without its window goes through.
//!
//! The command line, a daemon on a socket, an MCP server, a plugin: each is
//! a transport, and a transport is a loop that reads a [`Request`], hands
//! it to [`Api::dispatch`], and writes the [`Response`] and any [`Event`]s
//! back. Nothing about what a request *means* lives in a transport, so two
//! of them cannot disagree, and a method added here reaches all of them.
//!
//! The crate decides nothing about the edit either. Edits are
//! `concat_project` [`Command`]s carried as they are; projects, media,
//! templates, titles, cutouts and exports are `concat_host`'s. What this
//! crate owns is the choreography the window performs by hand - probe then
//! add, paint titles and find masks before rendering, save through the
//! session - stated once so a file exported here is the file the window
//! would have written.
//!
//! One [`Api`] holds one session per open project folder. It is not
//! thread-safe by design: a transport that serves several callers owns the
//! one `Api` and serialises through it, the way the window's event loop
//! does, and a long method blocks its caller. [`Api::exporter`] is the one
//! handle that crosses threads, so a cancel can reach a running export.

pub mod message;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_effects::Catalogue;
use concat_effects::manifest::Kind;
use concat_export::ExportClip;
use concat_host::cutout::{self, Cutouts};
use concat_host::export::{self, Exporter};
use concat_host::preview::{FrameSpec, Monitor};
use concat_host::session::EditorView;
use concat_host::templates::{self, SlotFill};
use concat_host::{AppDirs, ProjectInfo, Session, Titles, media, projects};
use concat_project::Command;
use concat_project::model::VideoSettings;

pub use message::{
    API_VERSION, Dirs, Done, Event, ExportSpec, Fill, PackageInfo, ParamInfo, Reply, Request,
    Response, VersionInfo, Written,
};

/// The export sheet's middle quality, and what an export gets when the
/// caller says nothing.
const DEFAULT_CRF: u8 = 20;
/// The x264 preset every export uses unless told otherwise.
const DEFAULT_PRESET: &str = "medium";

/// The dispatcher: the open sessions and the services behind them.
pub struct Api {
    dirs: AppDirs,
    /// Open projects by canonical folder path.
    sessions: BTreeMap<String, Session>,
    titles: Titles,
    cutouts: Cutouts,
    monitor: Monitor,
    exporter: Exporter,
}

impl Api {
    /// An API over this machine's app directories.
    pub fn new() -> Result<Api, String> {
        Ok(Api::with_dirs(AppDirs::locate()?))
    }

    /// An API over the given directories: a test's scratch, or an embedder
    /// with a home of its own.
    pub fn with_dirs(dirs: AppDirs) -> Api {
        Api {
            titles: Titles::new(&dirs),
            cutouts: Cutouts::new(&dirs.data),
            monitor: Monitor::new(),
            exporter: Exporter::new(),
            sessions: BTreeMap::new(),
            dirs,
        }
    }

    /// The directories this API works under.
    pub fn dirs(&self) -> &AppDirs {
        &self.dirs
    }

    /// The export slot, for a transport that needs to cancel from another
    /// thread while [`Api::dispatch`] blocks on [`Request::ExportRun`].
    pub fn exporter(&self) -> Exporter {
        self.exporter.clone()
    }

    /// Runs one request. Events fire through `notify` as the work goes;
    /// the response is what the request is worth.
    pub fn dispatch(&mut self, request: Request, notify: &mut dyn FnMut(Event)) -> Response {
        self.run(request, notify).into()
    }

    fn run(&mut self, request: Request, notify: &mut dyn FnMut(Event)) -> Result<Reply, String> {
        let view = |view: EditorView| Ok(Reply::View(Box::new(view)));
        match request {
            Request::Version => Ok(Reply::Version(self.version())),
            Request::ProjectCreate {
                location,
                name,
                video,
            } => view(self.create(&location, &name, video.unwrap_or_default())?),
            Request::ProjectOpen { path } => view(self.open(&path)?),
            Request::ProjectClose { path, save } => {
                self.close(&path, save)?;
                Ok(Reply::Done(Done {}))
            }
            Request::ProjectList => Ok(Reply::Projects(self.recents())),
            Request::ProjectGet { path } => view(self.session(&path)?.view()),
            Request::ProjectDocument { path } => {
                Ok(Reply::Document(self.session(&path)?.document()))
            }
            Request::ProjectSave { path, name } => {
                self.save(&path, name.as_deref())?;
                Ok(Reply::Done(Done {}))
            }
            Request::ProjectSetVideo { path, video } => {
                view(self.session_mut(&path)?.set_video(video)?)
            }
            Request::EditApply { path, command } => view(self.apply(&path, *command)?),
            Request::EditUndo { path } => view(self.session_mut(&path)?.undo()),
            Request::EditRedo { path } => view(self.session_mut(&path)?.redo()),
            Request::MediaProbe { path } => Ok(Reply::Media(media::probe(&path)?)),
            Request::MediaImport { path, file } => view(self.import(&path, &file)?),
            Request::CatalogueList { kind } => Ok(Reply::Packages(catalogue(kind.as_deref())?)),
            Request::TemplateList => Ok(Reply::Templates(templates::list(&self.dirs.config))),
            Request::TemplateInstantiate {
                template,
                location,
                name,
                fills,
            } => view(self.instantiate(&template, &location, &name, fills)?),
            Request::TemplateSave { path, name } => {
                Ok(Reply::Template(self.save_template(&path, &name)?))
            }
            Request::ExportRun { path, spec } => {
                Ok(Reply::Written(self.export(&path, &spec, notify)?))
            }
            Request::ExportCancel => {
                self.exporter.cancel();
                Ok(Reply::Done(Done {}))
            }
            Request::PreviewFrame {
                path,
                time,
                output,
                width,
                height,
            } => Ok(Reply::Written(self.frame(
                &path,
                time,
                &output,
                width.zip(height),
            )?)),
        }
    }

    /// [`Request::Version`].
    pub fn version(&self) -> VersionInfo {
        VersionInfo {
            api_version: API_VERSION.to_owned(),
            concat: env!("CARGO_PKG_VERSION").to_owned(),
            dirs: Dirs::from(&self.dirs),
        }
    }

    /// [`Request::ProjectCreate`]: the folder, its manifest, and a session
    /// on it, remembered in recents like a project the window made.
    pub fn create(
        &mut self,
        location: &str,
        name: &str,
        video: VideoSettings,
    ) -> Result<EditorView, String> {
        let info = projects::create(
            location,
            name,
            video.width,
            video.height,
            video.rate_num,
            video.rate_den,
        )?;
        self.adopt(info)
    }

    /// [`Request::ProjectOpen`].
    pub fn open(&mut self, path: &str) -> Result<EditorView, String> {
        let key = key_of(path);
        if let Some(session) = self.sessions.get(&key) {
            return Ok(session.view());
        }
        let info = projects::open(path)?;
        self.adopt(info)
    }

    /// Opens a session on a project the host just described and puts it at
    /// the front of the recents list.
    fn adopt(&mut self, info: ProjectInfo) -> Result<EditorView, String> {
        let session = Session::open_info(&info)?;
        // Recents are a convenience for the launch screen; a machine whose
        // config folder cannot be written still edits.
        let _ = projects::remember(&self.dirs.config, &info);
        let view = session.view();
        self.sessions.insert(key_of(&info.path), session);
        Ok(view)
    }

    /// [`Request::ProjectClose`].
    pub fn close(&mut self, path: &str, save: bool) -> Result<(), String> {
        if save {
            self.save(path, None)?;
        }
        self.sessions
            .remove(&key_of(path))
            .map(drop)
            .ok_or_else(|| not_open(path))
    }

    /// [`Request::ProjectList`].
    pub fn recents(&self) -> Vec<ProjectInfo> {
        projects::list(&self.dirs.config)
    }

    /// [`Request::ProjectSave`].
    pub fn save(&mut self, path: &str, name: Option<&str>) -> Result<(), String> {
        self.session_mut(path)?.save(name)
    }

    /// [`Request::EditApply`].
    pub fn apply(&mut self, path: &str, command: Command) -> Result<EditorView, String> {
        self.session_mut(path)?.apply(command)
    }

    /// [`Request::MediaImport`]: the probe and the add, as one.
    pub fn import(&mut self, path: &str, file: &str) -> Result<EditorView, String> {
        let item = media::probe(file)?.to_new_media();
        self.apply(path, Command::AddMedia { item })
    }

    /// [`Request::TemplateInstantiate`].
    pub fn instantiate(
        &mut self,
        template: &str,
        location: &str,
        name: &str,
        fills: Vec<Fill>,
    ) -> Result<EditorView, String> {
        // Every file is probed before anything is made, so a bad path
        // refuses the whole request rather than leaving a folder behind.
        let fills = fills
            .into_iter()
            .map(|fill| {
                Ok(SlotFill {
                    media_id: fill.media_id,
                    item: media::probe(&fill.file)?.to_new_media(),
                })
            })
            .collect::<Result<Vec<SlotFill>, String>>()?;
        let info = templates::instantiate(template, location, name, fills)?;
        self.adopt(info)
    }

    /// [`Request::TemplateSave`].
    pub fn save_template(
        &mut self,
        path: &str,
        name: &str,
    ) -> Result<concat_host::templates::TemplateInfo, String> {
        let session = self.session(path)?;
        templates::save(
            &self.dirs.config,
            &session.document(),
            &session.settings(),
            session.path(),
            name,
        )
    }

    /// [`Request::ExportRun`]: what the window's Export sheet does, in its
    /// order. Masks first, because the frame loop reads whatever is in the
    /// project's cache and draws the picture as shot where there is none;
    /// then titles, which rejoin the clip list as stills; then the render.
    pub fn export(
        &mut self,
        path: &str,
        spec: &ExportSpec,
        notify: &mut dyn FnMut(Event),
    ) -> Result<Written, String> {
        let session = self.session(path)?;
        if session.project().active().clips.is_empty() {
            return Err("There is nothing on the timeline to export".to_owned());
        }
        let settings = session.settings();
        let width = spec.width.unwrap_or(settings.width);
        let height = spec.height.unwrap_or(settings.height);

        let project_dir = PathBuf::from(session.path());
        let project = session.project().clone();
        self.find_masks(&project, &project_dir, notify)?;

        let session = self.session(path)?;
        let titles = self.title_clips(session, width, height);
        let host_spec = export::ExportSpec {
            output: spec.output.clone(),
            crf: spec.crf.unwrap_or(DEFAULT_CRF),
            preset: spec
                .preset
                .clone()
                .unwrap_or_else(|| DEFAULT_PRESET.to_owned()),
            encoder: "libx264".into(),
        };
        let mut request = export::request(session, &host_spec, titles);
        request.width = width;
        request.height = height;
        request.rate_num = spec.rate_num.unwrap_or(settings.rate_num);
        request.rate_den = spec.rate_den.unwrap_or(settings.rate_den);

        let job = self.exporter.begin()?;
        let written = export::run(&request, job.cancel_flag(), |progress| {
            notify(Event::from(progress));
        })?;
        Ok(Written {
            path: written,
            width,
            height,
        })
    }

    /// Runs every cutout analysis the timeline still needs, one after the
    /// other, so the export that follows cuts every clip it should.
    fn find_masks(
        &self,
        project: &concat_project::Project,
        project_dir: &Path,
        notify: &mut dyn FnMut(Event),
    ) -> Result<(), String> {
        for (media_id, request) in Cutouts::requests(project, project_dir) {
            if Cutouts::outstanding(&request) == 0 {
                continue;
            }
            self.cutouts.analyse(&request, &mut |progress| {
                let (fetching, fraction) = match progress {
                    cutout::Progress::Fetching { received, total } => {
                        (true, received as f32 / total.max(1) as f32)
                    }
                    cutout::Progress::Analysing(fraction) => (false, fraction),
                };
                notify(Event::CutoutProgress {
                    media_id: media_id.clone(),
                    fetching,
                    fraction,
                });
            })?;
        }
        Ok(())
    }

    /// The timeline's titles painted for a `width` × `height` frame, as
    /// the stills that stand in for them.
    fn title_clips(&self, session: &Session, width: u32, height: u32) -> Vec<ExportClip> {
        self.titles
            .clips(session.project(), width, height)
            .into_iter()
            .map(|title| title.clip)
            .collect()
    }

    /// [`Request::PreviewFrame`]: the paused monitor's true frame, to a
    /// file.
    pub fn frame(
        &mut self,
        path: &str,
        time: f64,
        output: &str,
        size: Option<(u32, u32)>,
    ) -> Result<Written, String> {
        let session = self.session(path)?;
        let settings = session.settings();
        let (width, height) = size.unwrap_or((settings.width, settings.height));
        if width == 0 || height == 0 {
            return Err("A frame needs a width and a height".to_owned());
        }
        let mut clips = session.flattened_clips();
        clips.extend(self.title_clips(session, width, height));
        let pixels = self.monitor.frame(
            Arc::new(clips),
            &settings,
            FrameSpec {
                time,
                width,
                height,
            },
        )?;
        write_png(Path::new(output), width, height, &pixels)?;
        Ok(Written {
            path: output.to_owned(),
            width,
            height,
        })
    }

    fn session(&self, path: &str) -> Result<&Session, String> {
        self.sessions
            .get(&key_of(path))
            .ok_or_else(|| not_open(path))
    }

    fn session_mut(&mut self, path: &str) -> Result<&mut Session, String> {
        self.sessions
            .get_mut(&key_of(path))
            .ok_or_else(|| not_open(path))
    }
}

/// [`Request::CatalogueList`]: the built-in packages plus whatever
/// [`Catalogue::install`] added, in id order.
fn catalogue(kind: Option<&str>) -> Result<Vec<PackageInfo>, String> {
    let kind = match kind {
        None => None,
        Some("effect") => Some(Kind::Effect),
        Some("filter") => Some(Kind::Filter),
        Some("audio") => Some(Kind::Audio),
        Some("transition") => Some(Kind::Transition),
        Some("generator") => Some(Kind::Generator),
        Some(other) => {
            return Err(format!(
                "{other:?} is not a kind: effect, filter, audio, transition or generator"
            ));
        }
    };
    let mut packages: Vec<PackageInfo> = Catalogue::builtin()
        .packages()
        .filter(|package| kind.is_none_or(|kind| package.kind() == kind))
        .map(|package| {
            let meta = &package.manifest.effect;
            PackageInfo {
                id: meta.id.clone(),
                name: meta.name.clone(),
                kind: format!("{:?}", meta.kind).to_ascii_lowercase(),
                category: meta.category.clone(),
                description: meta.description.clone(),
                intensity: meta.intensity.clone(),
                params: package
                    .manifest
                    .params
                    .iter()
                    .map(|param| ParamInfo {
                        key: param.key.clone(),
                        label: param.label.clone(),
                        kind: format!("{:?}", param.kind).to_ascii_lowercase(),
                        min: param.min,
                        max: param.max,
                        default: param.default,
                        step: param.step,
                        unit: param.unit.clone(),
                        animate: param.animate,
                        values: param.values.clone(),
                        labels: param.labels.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    packages.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(packages)
}

/// The key a project folder is held under: its canonical path where the
/// folder exists, so `.` and an absolute spelling of it are one session,
/// and the path as given where it does not yet.
fn key_of(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|canonical| canonical.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned())
}

fn not_open(path: &str) -> String {
    format!("{path} is not open - open it first")
}

/// Writes RGBA pixels as a PNG, creating the folder above the file.
fn write_png(output: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let file = std::fs::File::create(output)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    writer
        .write_image_data(pixels)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    writer
        .finish()
        .map_err(|error| format!("could not write {}: {error}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::commands::NewMedia;
    use concat_project::model::MediaKind;
    use serde_json::json;

    /// An API whose config and data live in a scratch folder that goes
    /// away with the test.
    fn api() -> (Api, tempfile::TempDir) {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        (Api::with_dirs(dirs), scratch)
    }

    fn ok(response: Response) -> Reply {
        match response {
            Response::Result(reply) => reply,
            Response::Error(error) => panic!("refused: {error}"),
        }
    }

    fn err(response: Response) -> String {
        match response {
            Response::Result(_) => panic!("worked, unexpectedly"),
            Response::Error(error) => error,
        }
    }

    fn view(reply: Reply) -> EditorView {
        match reply {
            Reply::View(view) => *view,
            _ => panic!("not a view"),
        }
    }

    fn quiet() -> impl FnMut(Event) {
        |_| {}
    }

    fn still(path: &str) -> NewMedia {
        NewMedia {
            path: path.to_owned(),
            name: "still.png".to_owned(),
            duration: None,
            kind: MediaKind::Image,
            width: Some(640),
            height: Some(360),
            frame_rate: None,
            frame_rate_fraction: None,
            video_codec: None,
            audio_codec: None,
            has_audio: false,
            audio_tracks: Vec::new(),
        }
    }

    #[test]
    fn a_request_parses_by_method_and_carries_a_command_verbatim() {
        let request: Request = serde_json::from_value(json!({
            "method": "edit.apply",
            "path": "/p",
            "command": { "op": "addTextClip", "start": 1.5 }
        }))
        .expect("parses");
        assert_eq!(
            request,
            Request::EditApply {
                path: "/p".to_owned(),
                command: Box::new(Command::AddTextClip {
                    track_id: None,
                    start: 1.5,
                    style: None,
                    duration: None,
                    offset_y: None,
                }),
            }
        );
    }

    #[test]
    fn a_response_is_a_result_or_an_error_object() {
        let worked = serde_json::to_value(Response::Result(Reply::Done(Done {}))).expect("json");
        assert_eq!(worked, json!({ "result": {} }));
        let refused = serde_json::to_value(Response::Error("no".to_owned())).expect("json");
        assert_eq!(refused, json!({ "error": "no" }));
    }

    #[test]
    fn create_edit_save_and_reopen_round_trip() {
        let (mut api, scratch) = api();
        let location = scratch.path().join("projects");
        std::fs::create_dir_all(&location).expect("location");
        let location = location.to_string_lossy().into_owned();

        let created = view(ok(api.dispatch(
            Request::ProjectCreate {
                location: location.clone(),
                name: "Round trip".to_owned(),
                video: Some(VideoSettings {
                    width: 1080,
                    height: 1920,
                    rate_num: 60,
                    rate_den: 1,
                }),
            },
            &mut quiet(),
        )));
        assert_eq!(created.settings.width, 1080);
        assert_eq!(created.settings.height, 1920);
        assert_eq!(created.settings.rate_num, 60);
        let path = format!("{location}/Round trip");
        assert!(projects::is_project(Path::new(&path)));

        let added = view(ok(api.dispatch(
            Request::EditApply {
                path: path.clone(),
                command: Box::new(Command::AddMedia {
                    item: still("/nowhere/still.png"),
                }),
            },
            &mut quiet(),
        )));
        let media_id = added.created_id.expect("minted");
        let placed = view(ok(api.dispatch(
            Request::EditApply {
                path: path.clone(),
                command: Box::new(Command::AddClipAtFirstFree {
                    media_id,
                    start: 2.0,
                }),
            },
            &mut quiet(),
        )));
        assert!(placed.can_undo);
        assert_eq!(placed.project.active().clips.len(), 1);

        ok(api.dispatch(
            Request::ProjectClose {
                path: path.clone(),
                save: true,
            },
            &mut quiet(),
        ));
        assert_eq!(
            err(api.dispatch(Request::ProjectGet { path: path.clone() }, &mut quiet())),
            not_open(&path)
        );

        let reopened = view(ok(
            api.dispatch(Request::ProjectOpen { path: path.clone() }, &mut quiet())
        ));
        assert_eq!(reopened.project.active().clips.len(), 1);
        assert_eq!(reopened.project.active().clips[0].start, 2.0);
        assert!(!reopened.can_undo, "history does not survive a save");

        let recents = match ok(api.dispatch(Request::ProjectList, &mut quiet())) {
            Reply::Projects(list) => list,
            _ => panic!("not a list"),
        };
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "Round trip");
    }

    #[test]
    fn a_refusal_is_the_command_layers_sentence() {
        let (mut api, scratch) = api();
        let location = scratch.path().to_string_lossy().into_owned();
        let created = view(ok(api.dispatch(
            Request::ProjectCreate {
                location: location.clone(),
                name: "Refused".to_owned(),
                video: None,
            },
            &mut quiet(),
        )));
        let track_id = created.project.active().tracks[0].id.clone();
        let refused = err(api.dispatch(
            Request::EditApply {
                path: format!("{location}/Refused"),
                command: Box::new(Command::AddClip {
                    media_id: "m999".to_owned(),
                    track_id,
                    start: 0.0,
                }),
            },
            &mut quiet(),
        ));
        assert_eq!(refused, "That media is no longer in the bin.");
    }

    #[test]
    fn creating_over_a_project_is_refused() {
        let (mut api, scratch) = api();
        let location = scratch.path().to_string_lossy().into_owned();
        let create = || Request::ProjectCreate {
            location: location.clone(),
            name: "Twice".to_owned(),
            video: None,
        };
        ok(api.dispatch(create(), &mut quiet()));
        assert!(err(api.dispatch(create(), &mut quiet())).contains("already exists"));
    }

    #[test]
    fn the_catalogue_lists_packages_with_their_knobs() {
        let packages = catalogue(Some("filter")).expect("kind");
        assert!(!packages.is_empty());
        assert!(packages.iter().all(|package| package.kind == "filter"));
        assert!(packages.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(catalogue(Some("look")).is_err());
        let all = catalogue(None).expect("all");
        assert!(all.len() > packages.len());
        assert!(all.iter().any(|package| !package.params.is_empty()));
    }

    #[test]
    fn a_method_on_a_closed_project_says_so() {
        let (mut api, _scratch) = api();
        assert_eq!(
            err(api.dispatch(
                Request::EditUndo {
                    path: "/never".to_owned()
                },
                &mut quiet()
            )),
            not_open("/never")
        );
    }

    #[test]
    fn undo_and_redo_step_the_history() {
        let (mut api, scratch) = api();
        let location = scratch.path().to_string_lossy().into_owned();
        ok(api.dispatch(
            Request::ProjectCreate {
                location: location.clone(),
                name: "History".to_owned(),
                video: None,
            },
            &mut quiet(),
        ));
        let path = format!("{location}/History");
        ok(api.dispatch(
            Request::EditApply {
                path: path.clone(),
                command: Box::new(Command::AddTextClip {
                    track_id: None,
                    start: 0.0,
                    style: None,
                    duration: Some(3.0),
                    offset_y: None,
                }),
            },
            &mut quiet(),
        ));
        let undone = view(ok(
            api.dispatch(Request::EditUndo { path: path.clone() }, &mut quiet())
        ));
        assert!(undone.project.active().clips.is_empty());
        assert!(undone.can_redo);
        let redone = view(ok(api.dispatch(Request::EditRedo { path }, &mut quiet())));
        assert_eq!(redone.project.active().clips.len(), 1);
    }

    #[test]
    fn the_document_is_what_a_save_writes() {
        let (mut api, scratch) = api();
        let location = scratch.path().to_string_lossy().into_owned();
        ok(api.dispatch(
            Request::ProjectCreate {
                location: location.clone(),
                name: "Doc".to_owned(),
                video: None,
            },
            &mut quiet(),
        ));
        let path = format!("{location}/Doc");
        let document = match ok(api.dispatch(
            Request::ProjectDocument { path: path.clone() },
            &mut quiet(),
        )) {
            Reply::Document(document) => document,
            _ => panic!("not a document"),
        };
        ok(api.dispatch(
            Request::ProjectSave {
                path: path.clone(),
                name: None,
            },
            &mut quiet(),
        ));
        assert_eq!(projects::read_document(&path).expect("saved"), document);
    }
}
