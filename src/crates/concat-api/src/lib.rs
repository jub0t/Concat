// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Concat API: the one dispatcher every way of driving the editor
//! without its window goes through.
//!
//! The command line, a daemon on a socket, an MCP server, a plugin: each is
//! a transport, and a transport is a loop that reads a [`Request`], hands
//! it to [`Api::dispatch`], and writes the [`Response`] back, forwarding
//! the [`Event`]s the API's jobs raise as they come. Nothing about what a
//! request *means* lives in a transport, so two of them cannot disagree,
//! and a method added here reaches all of them. What a line transport puts
//! around the three is the JSON-RPC 2.0 envelope in [`rpc`].
//!
//! The crate decides nothing about the edit either. Edits are
//! `concat_project` [`Command`]s carried as they are; projects, media,
//! templates, titles, cutouts and exports are `concat_host`'s. What this
//! crate owns is the choreography the window performs by hand - probe then
//! add, find masks and paint titles before rendering, save through the
//! session - stated once so a file exported here is the file the window
//! would have written.
//!
//! One [`Api`] holds one session per open project folder. It is not
//! thread-safe by design: a transport that serves several callers owns the
//! one `Api` and serialises through it, the way the window's event loop
//! does. A method that would keep everyone waiting - an export - runs as a
//! job on its own thread instead: [`Api::dispatch`] returns its name at
//! once and the job reports through the [`EventSink`] the API was made
//! with, from that thread, which is why the sink must be `Send + Sync`.

pub mod message;
pub mod rpc;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use base64::Engine as _;
use concat_effects::Catalogue;
use concat_effects::manifest::Kind;
use concat_export::{ExportClip, ExportRequest};
pub use concat_host::AppDirs;
use concat_host::cutout::{self, AnalyseRequest, Cutouts};
use concat_host::export::{self, Exporter};
use concat_host::preview::{FrameSpec, Monitor};
use concat_host::session::EditorView;
use concat_host::templates::{self, SlotFill};
use concat_host::{ProjectInfo, Session, Titles, media, projects};
use concat_project::Command;
use concat_project::model::VideoSettings;

pub use message::{
    API_VERSION, ApiError, Dirs, Done, ErrorCode, Event, ExportSpec, Fill, PackageInfo, ParamInfo,
    Picture, Reply, Request, Response, Started, VersionInfo, Written,
};

/// The export sheet's middle quality, and what an export gets when the
/// caller says nothing.
const DEFAULT_CRF: u8 = 20;
/// The x264 preset every export uses unless told otherwise.
const DEFAULT_PRESET: &str = "medium";

/// What every build of the API serves, before what is decided by the
/// build or the embedder is added; see [`VersionInfo::capabilities`].
const CAPABILITIES: &[&str] = &["events"];

/// Where a job's events go. Called from the job's thread, so a transport
/// that writes them to a caller locks its writer inside.
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// The dispatcher: the open sessions and the services behind them.
pub struct Api {
    dirs: AppDirs,
    /// Open projects by canonical folder path.
    sessions: BTreeMap<String, Session>,
    titles: Titles,
    cutouts: Arc<Cutouts>,
    monitor: Monitor,
    exporter: Exporter,
    events: EventSink,
    jobs: Jobs,
    /// [`VersionInfo::capabilities`], in the order they were added.
    capabilities: Vec<String>,
}

impl Api {
    /// An API over this machine's app directories, reporting its jobs
    /// through `events`.
    pub fn new(events: EventSink) -> Result<Api, String> {
        Ok(Api::with_dirs(AppDirs::locate()?, events))
    }

    /// An API over the given directories: a test's scratch, or an embedder
    /// with a home of its own.
    pub fn with_dirs(dirs: AppDirs, events: EventSink) -> Api {
        Api {
            titles: Titles::new(&dirs),
            cutouts: Arc::new(Cutouts::new(&dirs.data)),
            monitor: Monitor::new(),
            exporter: Exporter::new(),
            sessions: BTreeMap::new(),
            events,
            jobs: Jobs::default(),
            dirs,
            capabilities: CAPABILITIES.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    /// The directories this API works under.
    pub fn dirs(&self) -> &AppDirs {
        &self.dirs
    }

    /// Adds a name to [`VersionInfo::capabilities`]: what the transport
    /// or embedder around this API serves that the API cannot know of
    /// itself, such as the sockets it is listening on. A name already
    /// there is not repeated.
    pub fn add_capability(&mut self, name: &str) {
        if !self.capabilities.iter().any(|known| known == name) {
            self.capabilities.push(name.to_owned());
        }
    }

    /// The export slot, for an embedder that shares it with a window.
    pub fn exporter(&self) -> Exporter {
        self.exporter.clone()
    }

    /// How many jobs are still running.
    pub fn running(&mut self) -> usize {
        self.jobs.reap();
        self.jobs.threads.len()
    }

    /// Waits for every running job to end. A transport calls this before
    /// it exits, so an export begun on its last line still finishes.
    pub fn finish(&mut self) {
        self.jobs.finish();
    }

    /// Runs one request. The response is what the request is worth; a job
    /// it begins reports through the sink from then on.
    pub fn dispatch(&mut self, request: Request) -> Response {
        self.jobs.reap();
        self.run(request).into()
    }

    fn run(&mut self, request: Request) -> Result<Reply, ApiError> {
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
                view(self.session_mut(&path)?.set_video(video).map_err(refused)?)
            }
            Request::EditApply { path, command } => view(self.apply(&path, *command)?),
            Request::EditUndo { path } => view(self.session_mut(&path)?.undo()),
            Request::EditRedo { path } => view(self.session_mut(&path)?.redo()),
            Request::MediaProbe { path } => {
                Ok(Reply::Media(media::probe(&path).map_err(ApiError::failed)?))
            }
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
            Request::ExportRun { path, spec } => Ok(Reply::Started(self.export(&path, &spec)?)),
            Request::ExportCancel { job } => {
                self.cancel(&job)?;
                Ok(Reply::Done(Done {}))
            }
            Request::PreviewFrame {
                path,
                time,
                output,
                width,
                height,
            } => {
                let size = width.zip(height);
                match output {
                    Some(output) => Ok(Reply::Written(self.frame(&path, time, &output, size)?)),
                    None => Ok(Reply::Picture(self.picture(&path, time, size)?)),
                }
            }
        }
    }

    /// [`Request::Version`].
    pub fn version(&self) -> VersionInfo {
        VersionInfo {
            api_version: API_VERSION.to_owned(),
            concat: env!("CARGO_PKG_VERSION").to_owned(),
            dirs: Dirs::from(&self.dirs),
            capabilities: {
                let mut capabilities = self.capabilities.clone();
                if self.monitor.has_gpu() {
                    capabilities.push("gpu".to_owned());
                }
                capabilities
            },
        }
    }

    /// [`Request::ProjectCreate`]: the folder, its manifest, and a session
    /// on it, remembered in recents like a project the window made.
    pub fn create(
        &mut self,
        location: &str,
        name: &str,
        video: VideoSettings,
    ) -> Result<EditorView, ApiError> {
        let info = projects::create(
            location,
            name,
            video.width,
            video.height,
            video.rate_num,
            video.rate_den,
        )
        .map_err(ApiError::failed)?;
        self.adopt(info)
    }

    /// [`Request::ProjectOpen`].
    pub fn open(&mut self, path: &str) -> Result<EditorView, ApiError> {
        let key = key_of(path);
        if let Some(session) = self.sessions.get(&key) {
            return Ok(session.view());
        }
        let info = projects::open(path).map_err(ApiError::failed)?;
        self.adopt(info)
    }

    /// Opens a session on a project the host just described and puts it at
    /// the front of the recents list.
    fn adopt(&mut self, info: ProjectInfo) -> Result<EditorView, ApiError> {
        let session = Session::open_info(&info).map_err(ApiError::failed)?;
        // Recents are a convenience for the launch screen; a machine whose
        // config folder cannot be written still edits.
        let _ = projects::remember(&self.dirs.config, &info);
        let view = session.view();
        self.sessions.insert(key_of(&info.path), session);
        Ok(view)
    }

    /// [`Request::ProjectClose`].
    pub fn close(&mut self, path: &str, save: bool) -> Result<(), ApiError> {
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
    pub fn save(&mut self, path: &str, name: Option<&str>) -> Result<(), ApiError> {
        self.session_mut(path)?.save(name).map_err(ApiError::failed)
    }

    /// [`Request::EditApply`].
    pub fn apply(&mut self, path: &str, command: Command) -> Result<EditorView, ApiError> {
        self.session_mut(path)?.apply(command).map_err(refused)
    }

    /// [`Request::MediaImport`]: the probe and the add, as one.
    pub fn import(&mut self, path: &str, file: &str) -> Result<EditorView, ApiError> {
        let item = media::probe(file).map_err(ApiError::failed)?.to_new_media();
        self.apply(path, Command::AddMedia { item })
    }

    /// [`Request::TemplateInstantiate`].
    pub fn instantiate(
        &mut self,
        template: &str,
        location: &str,
        name: &str,
        fills: Vec<Fill>,
    ) -> Result<EditorView, ApiError> {
        // Every file is probed before anything is made, so a bad path
        // refuses the whole request rather than leaving a folder behind.
        let fills = fills
            .into_iter()
            .map(|fill| {
                Ok(SlotFill {
                    media_id: fill.media_id,
                    item: media::probe(&fill.file)
                        .map_err(ApiError::failed)?
                        .to_new_media(),
                })
            })
            .collect::<Result<Vec<SlotFill>, ApiError>>()?;
        let info =
            templates::instantiate(template, location, name, fills).map_err(ApiError::failed)?;
        self.adopt(info)
    }

    /// [`Request::TemplateSave`].
    pub fn save_template(
        &mut self,
        path: &str,
        name: &str,
    ) -> Result<concat_host::templates::TemplateInfo, ApiError> {
        let session = self.session(path)?;
        templates::save(
            &self.dirs.config,
            &session.document(),
            &session.settings(),
            session.path(),
            name,
        )
        .map_err(ApiError::failed)
    }

    /// [`Request::ExportRun`]: what the window's Export sheet does, in its
    /// order, on a thread of its own. Masks first, because the frame loop
    /// reads whatever is in the project's cache and draws the picture as
    /// shot where there is none; then the render, with the titles already
    /// painted here and rejoining the clip list as stills.
    pub fn export(&mut self, path: &str, spec: &ExportSpec) -> Result<Started, ApiError> {
        let session = self.session(path)?;
        if session.project().active().clips.is_empty() {
            return Err(ApiError::new(
                ErrorCode::Refused,
                "There is nothing on the timeline to export",
            ));
        }
        let settings = session.settings();
        let width = spec.width.unwrap_or(settings.width);
        let height = spec.height.unwrap_or(settings.height);
        let codec = match spec.codec.as_deref() {
            None => export::VideoCodec::H264,
            Some(name) => export::VideoCodec::parse(name).ok_or_else(|| {
                ApiError::invalid(format!("unknown codec {name:?}: h264, hevc or av1"))
            })?,
        };
        let host_spec = export::ExportSpec {
            output: spec.output.clone(),
            crf: spec.crf.unwrap_or(DEFAULT_CRF),
            preset: spec
                .preset
                .clone()
                .unwrap_or_else(|| DEFAULT_PRESET.to_owned()),
            codec,
            ten_bit: spec.ten_bit.unwrap_or(false),
        };

        let project_path = session.path().to_owned();
        let project_dir = PathBuf::from(&project_path);
        let masks: Vec<(String, AnalyseRequest)> =
            Cutouts::requests(session.project(), &project_dir)
                .into_iter()
                .filter(|(_, request)| Cutouts::outstanding(request) != 0)
                .collect();
        let titles = self.title_clips(session, width, height);
        let mut request = export::request(session, &host_spec, titles);
        request.width = width;
        request.height = height;
        request.rate_num = spec.rate_num.unwrap_or(settings.rate_num);
        request.rate_den = spec.rate_den.unwrap_or(settings.rate_den);

        let slot = self
            .exporter
            .begin()
            .map_err(|message| ApiError::new(ErrorCode::Busy, message))?;
        let started = Started {
            job: self.jobs.mint(),
            path: project_path,
            output: spec.output.clone(),
        };
        let job = ExportJob {
            name: started.job.clone(),
            path: started.path.clone(),
            request,
            masks,
            cutouts: Arc::clone(&self.cutouts),
            events: Arc::clone(&self.events),
        };
        let thread = std::thread::Builder::new()
            .name(format!("export {}", started.job))
            .spawn(move || job.run(slot))
            .map_err(|error| ApiError::failed(format!("could not start the export: {error}")))?;
        self.jobs.threads.push((started.job.clone(), thread));
        Ok(started)
    }

    /// [`Request::ExportCancel`].
    pub fn cancel(&mut self, job: &str) -> Result<(), ApiError> {
        self.jobs.reap();
        if !self.jobs.threads.iter().any(|(name, _)| name == job) {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job} is running"),
            ));
        }
        // Whichever phase it is in: the analysis ahead of the render, or
        // the render itself.
        self.cutouts.cancel();
        self.exporter.cancel();
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

    /// [`Request::PreviewFrame`] with an output: the paused monitor's true
    /// frame, to a file.
    pub fn frame(
        &mut self,
        path: &str,
        time: f64,
        output: &str,
        size: Option<(u32, u32)>,
    ) -> Result<Written, ApiError> {
        let (width, height, pixels) = self.pixels(path, time, size)?;
        write_png(Path::new(output), width, height, &pixels)?;
        Ok(Written {
            path: output.to_owned(),
            width,
            height,
        })
    }

    /// [`Request::PreviewFrame`] without one: the same frame, inline.
    pub fn picture(
        &mut self,
        path: &str,
        time: f64,
        size: Option<(u32, u32)>,
    ) -> Result<Picture, ApiError> {
        let (width, height, pixels) = self.pixels(path, time, size)?;
        let mut png = Vec::new();
        encode_png(&mut png, width, height, &pixels)
            .map_err(|error| ApiError::failed(format!("could not encode the frame: {error}")))?;
        Ok(Picture {
            width,
            height,
            png: base64::engine::general_purpose::STANDARD.encode(png),
        })
    }

    /// The composited RGBA frame at `time`.
    fn pixels(
        &mut self,
        path: &str,
        time: f64,
        size: Option<(u32, u32)>,
    ) -> Result<(u32, u32, Vec<u8>), ApiError> {
        let session = self.session(path)?;
        let settings = session.settings();
        let (width, height) = size.unwrap_or((settings.width, settings.height));
        if width == 0 || height == 0 {
            return Err(ApiError::invalid("A frame needs a width and a height"));
        }
        let mut clips = session.flattened_clips();
        clips.extend(self.title_clips(session, width, height));
        let pixels = self
            .monitor
            .frame(
                Arc::new(clips),
                &settings,
                FrameSpec {
                    time,
                    width,
                    height,
                },
            )
            .map_err(ApiError::failed)?;
        Ok((width, height, pixels))
    }

    fn session(&self, path: &str) -> Result<&Session, ApiError> {
        self.sessions
            .get(&key_of(path))
            .ok_or_else(|| not_open(path))
    }

    fn session_mut(&mut self, path: &str) -> Result<&mut Session, ApiError> {
        self.sessions
            .get_mut(&key_of(path))
            .ok_or_else(|| not_open(path))
    }
}

/// The jobs the API has begun, by name, with the thread each runs on.
#[derive(Default)]
struct Jobs {
    minted: u64,
    threads: Vec<(String, JoinHandle<()>)>,
}

impl Jobs {
    /// The next name.
    fn mint(&mut self) -> String {
        self.minted += 1;
        format!("j{}", self.minted)
    }

    /// Forgets the threads that have ended.
    fn reap(&mut self) {
        self.threads.retain(|(_, thread)| !thread.is_finished());
    }

    /// Waits for the rest.
    fn finish(&mut self) {
        for (_, thread) in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// An export in flight: everything its thread needs, owned.
struct ExportJob {
    name: String,
    path: String,
    request: ExportRequest,
    masks: Vec<(String, AnalyseRequest)>,
    cutouts: Arc<Cutouts>,
    events: EventSink,
}

impl ExportJob {
    /// The whole job, start to end, on the thread it is called from. The
    /// slot is held until this returns, so the next export waits its turn.
    fn run(self, slot: concat_host::jobs::Job) {
        let outcome = self.masks(&slot).and_then(|()| {
            export::run(&self.request, slot.cancel_flag(), |progress| {
                (self.events)(Event::progress(&self.name, &self.path, progress));
            })
        });
        let event = match outcome {
            Ok(output) => Event::ExportDone {
                job: self.name.clone(),
                path: self.path.clone(),
                output,
                width: self.request.width,
                height: self.request.height,
            },
            Err(message) => Event::ExportFailed {
                job: self.name.clone(),
                path: self.path.clone(),
                error: if slot.cancelled() {
                    ApiError::new(ErrorCode::Cancelled, "The export was cancelled")
                } else {
                    ApiError::failed(message)
                },
            },
        };
        (self.events)(event);
    }

    /// Runs every cutout analysis the timeline still needs, one after the
    /// other, so the render that follows cuts every clip it should.
    fn masks(&self, slot: &concat_host::jobs::Job) -> Result<(), String> {
        for (media_id, request) in &self.masks {
            if slot.cancelled() {
                return Err("cancelled".to_owned());
            }
            self.cutouts.analyse(request, &mut |progress| {
                let (fetching, fraction) = match progress {
                    cutout::Progress::Fetching { received, total } => {
                        (true, received as f32 / total.max(1) as f32)
                    }
                    cutout::Progress::Analysing(fraction) => (false, fraction),
                };
                (self.events)(Event::CutoutProgress {
                    job: self.name.clone(),
                    path: self.path.clone(),
                    media_id: media_id.clone(),
                    fetching,
                    fraction,
                });
            })?;
        }
        Ok(())
    }
}

/// [`Request::CatalogueList`]: the built-in packages plus whatever
/// [`Catalogue::install`] added, in id order.
fn catalogue(kind: Option<&str>) -> Result<Vec<PackageInfo>, ApiError> {
    let kind = match kind {
        None => None,
        Some("effect") => Some(Kind::Effect),
        Some("filter") => Some(Kind::Filter),
        Some("audio") => Some(Kind::Audio),
        Some("transition") => Some(Kind::Transition),
        Some("generator") => Some(Kind::Generator),
        Some(other) => {
            return Err(ApiError::invalid(format!(
                "{other:?} is not a kind: effect, filter, audio, transition or generator"
            )));
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

fn not_open(path: &str) -> ApiError {
    ApiError::new(
        ErrorCode::NotOpen,
        format!("{path} is not open - open it first"),
    )
}

/// The edit layer's no, as the code that says so.
fn refused(message: String) -> ApiError {
    ApiError::new(ErrorCode::Refused, message)
}

/// Writes RGBA pixels as a PNG, creating the folder above the file.
fn write_png(output: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), ApiError> {
    let could_not = |error: &dyn std::fmt::Display| {
        ApiError::failed(format!("could not write {}: {error}", output.display()))
    };
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ApiError::failed(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    let file = std::fs::File::create(output).map_err(|error| could_not(&error))?;
    encode_png(std::io::BufWriter::new(file), width, height, pixels)
        .map_err(|error| could_not(&error))
}

/// Encodes RGBA pixels as a PNG into `sink`.
fn encode_png(
    sink: impl std::io::Write,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), png::EncodingError> {
    let mut encoder = png::Encoder::new(sink, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::commands::NewMedia;
    use concat_project::model::MediaKind;
    use serde_json::json;
    use std::sync::Mutex;

    /// An API whose config and data live in a scratch folder that goes
    /// away with the test, and whose events pile up where a test can read
    /// them.
    fn api() -> (Api, tempfile::TempDir, Arc<Mutex<Vec<Event>>>) {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let api = Api::with_dirs(
            dirs,
            Arc::new(move |event| sink.lock().expect("events").push(event)),
        );
        (api, scratch, events)
    }

    fn ok(response: Response) -> Reply {
        match response {
            Response::Result(reply) => reply,
            Response::Error(error) => panic!("refused: {error}"),
        }
    }

    fn err(response: Response) -> ApiError {
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

    /// A project named `name` under the scratch folder, open.
    fn project(api: &mut Api, scratch: &tempfile::TempDir, name: &str) -> String {
        let location = scratch.path().to_string_lossy().into_owned();
        ok(api.dispatch(Request::ProjectCreate {
            location: location.clone(),
            name: name.to_owned(),
            video: None,
        }));
        format!("{location}/{name}")
    }

    #[test]
    fn version_names_what_the_build_serves() {
        let (mut api, _scratch, _events) = api();
        let base = api.version();
        assert_eq!(base.api_version, API_VERSION);
        assert_eq!(base.capabilities, vec!["events".to_owned()]);
        assert!(!base.capabilities.iter().any(|name| name == "gpu"));

        api.add_capability("json-rpc");
        api.add_capability("json-rpc");
        api.add_capability("grpc");
        let served = serde_json::to_value(ok(api.dispatch(Request::Version))).expect("JSON");
        assert_eq!(
            served["capabilities"],
            json!(["events", "json-rpc", "grpc"]),
            "each name once, in the order added"
        );
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
                    above: false,
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
        let refused =
            serde_json::to_value(Response::Error(ApiError::new(ErrorCode::Refused, "no")))
                .expect("json");
        assert_eq!(
            refused,
            json!({ "error": { "code": "refused", "message": "no" } })
        );
    }

    #[test]
    fn create_edit_save_and_reopen_round_trip() {
        let (mut api, scratch, _) = api();
        let location = scratch.path().join("projects");
        std::fs::create_dir_all(&location).expect("location");
        let location = location.to_string_lossy().into_owned();

        let created = view(ok(api.dispatch(Request::ProjectCreate {
            location: location.clone(),
            name: "Round trip".to_owned(),
            video: Some(VideoSettings {
                width: 1080,
                height: 1920,
                rate_num: 60,
                rate_den: 1,
            }),
        })));
        assert_eq!(created.settings.width, 1080);
        assert_eq!(created.settings.height, 1920);
        assert_eq!(created.settings.rate_num, 60);
        let path = format!("{location}/Round trip");
        assert!(projects::is_project(Path::new(&path)));

        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/still.png"),
            }),
        })));
        let media_id = added.created_id.expect("minted");
        let placed = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id,
                start: 2.0,
            }),
        })));
        assert!(placed.can_undo);
        assert_eq!(placed.project.active().clips.len(), 1);

        ok(api.dispatch(Request::ProjectClose {
            path: path.clone(),
            save: true,
        }));
        assert_eq!(
            err(api.dispatch(Request::ProjectGet { path: path.clone() })),
            not_open(&path)
        );

        let reopened = view(ok(api.dispatch(Request::ProjectOpen { path: path.clone() })));
        assert_eq!(reopened.project.active().clips.len(), 1);
        assert_eq!(reopened.project.active().clips[0].start, 2.0);
        assert!(!reopened.can_undo, "history does not survive a save");

        let recents = match ok(api.dispatch(Request::ProjectList)) {
            Reply::Projects(list) => list,
            _ => panic!("not a list"),
        };
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "Round trip");
    }

    #[test]
    fn a_refusal_is_the_command_layers_sentence_with_its_code() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Refused");
        let created = view(ok(api.dispatch(Request::ProjectGet { path: path.clone() })));
        let track_id = created.project.active().tracks[0].id.clone();
        let refused = err(api.dispatch(Request::EditApply {
            path,
            command: Box::new(Command::AddClip {
                media_id: "m999".to_owned(),
                track_id,
                start: 0.0,
                ripple: false,
            }),
        }));
        assert_eq!(refused.code, ErrorCode::Refused);
        assert_eq!(refused.message, "That media is no longer in the bin.");
    }

    #[test]
    fn creating_over_a_project_is_refused() {
        let (mut api, scratch, _) = api();
        let location = scratch.path().to_string_lossy().into_owned();
        let create = || Request::ProjectCreate {
            location: location.clone(),
            name: "Twice".to_owned(),
            video: None,
        };
        ok(api.dispatch(create()));
        let again = err(api.dispatch(create()));
        assert_eq!(again.code, ErrorCode::Failed);
        assert!(again.message.contains("already exists"));
    }

    #[test]
    fn the_catalogue_lists_packages_with_their_knobs() {
        let packages = catalogue(Some("filter")).expect("kind");
        assert!(!packages.is_empty());
        assert!(packages.iter().all(|package| package.kind == "filter"));
        assert!(packages.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(
            catalogue(Some("look")).expect_err("not a kind").code,
            ErrorCode::Invalid
        );
        let all = catalogue(None).expect("all");
        assert!(all.len() > packages.len());
        assert!(all.iter().any(|package| !package.params.is_empty()));
    }

    #[test]
    fn a_method_on_a_closed_project_says_so() {
        let (mut api, _scratch, _) = api();
        let refused = err(api.dispatch(Request::EditUndo {
            path: "/never".to_owned(),
        }));
        assert_eq!(refused, not_open("/never"));
        assert_eq!(refused.code, ErrorCode::NotOpen);
    }

    #[test]
    fn undo_and_redo_step_the_history() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "History");
        ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(3.0),
                offset_y: None,
            }),
        }));
        let undone = view(ok(api.dispatch(Request::EditUndo { path: path.clone() })));
        assert!(undone.project.active().clips.is_empty());
        assert!(undone.can_redo);
        let redone = view(ok(api.dispatch(Request::EditRedo { path })));
        assert_eq!(redone.project.active().clips.len(), 1);
    }

    #[test]
    fn the_document_is_what_a_save_writes() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Doc");
        let document = match ok(api.dispatch(Request::ProjectDocument { path: path.clone() })) {
            Reply::Document(document) => document,
            _ => panic!("not a document"),
        };
        ok(api.dispatch(Request::ProjectSave {
            path: path.clone(),
            name: None,
        }));
        assert_eq!(projects::read_document(&path).expect("saved"), document);
    }

    #[test]
    fn an_empty_timeline_has_nothing_to_export_and_no_job_to_cancel() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Empty");
        let refused = err(api.dispatch(Request::ExportRun {
            path,
            spec: ExportSpec {
                output: scratch
                    .path()
                    .join("out.mp4")
                    .to_string_lossy()
                    .into_owned(),
                ..ExportSpec::default()
            },
        }));
        assert_eq!(refused.code, ErrorCode::Refused);
        let missing = err(api.dispatch(Request::ExportCancel {
            job: "j1".to_owned(),
        }));
        assert_eq!(missing.code, ErrorCode::NotFound);
        assert_eq!(api.running(), 0);
    }

    #[test]
    fn an_export_is_a_job_that_reports_how_it_ended() {
        let (mut api, scratch, events) = api();
        let path = project(&mut api, &scratch, "Job");
        // A still that does not exist: the job starts, since the timeline
        // has a clip, and fails in the render, which is an event.
        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/still.png"),
            }),
        })));
        ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id: added.created_id.expect("minted"),
                start: 0.0,
            }),
        }));
        let started = match ok(api.dispatch(Request::ExportRun {
            path: path.clone(),
            spec: ExportSpec {
                output: scratch
                    .path()
                    .join("out.mp4")
                    .to_string_lossy()
                    .into_owned(),
                ..ExportSpec::default()
            },
        })) {
            Reply::Started(started) => started,
            _ => panic!("not a job"),
        };
        assert_eq!(started.job, "j1");
        api.finish();
        let events = events.lock().expect("events");
        let last = events.last().expect("the job said how it ended");
        assert_eq!(last.job(), "j1");
        match last {
            Event::ExportFailed {
                path: at, error, ..
            } => {
                assert_eq!(at, &path);
                assert_eq!(error.code, ErrorCode::Failed);
            }
            other => panic!("ended with {other:?}"),
        }
        assert_eq!(api.running(), 0);
    }

    #[test]
    fn a_frame_comes_back_inline_when_no_output_is_named() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Frame");
        let picture = match ok(api.dispatch(Request::PreviewFrame {
            path: path.clone(),
            time: 0.0,
            output: None,
            width: Some(16),
            height: Some(9),
        })) {
            Reply::Picture(picture) => picture,
            _ => panic!("not a picture"),
        };
        assert_eq!((picture.width, picture.height), (16, 9));
        let png = base64::engine::general_purpose::STANDARD
            .decode(picture.png)
            .expect("base64");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let reader = decoder.read_info().expect("a PNG");
        assert_eq!((reader.info().width, reader.info().height), (16, 9));

        let refused = err(api.dispatch(Request::PreviewFrame {
            path,
            time: 0.0,
            output: None,
            width: Some(0),
            height: Some(9),
        }));
        assert_eq!(refused.code, ErrorCode::Invalid);
    }
}
