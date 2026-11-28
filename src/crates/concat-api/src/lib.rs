// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

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
//! `concat_doc` commands, as [`Command`] carries them; projects, media,
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

#[cfg(feature = "schema")]
mod contract;
mod job;
mod lend;
#[cfg(test)]
mod lend_tests;
pub mod message;
#[cfg(test)]
mod project_close_tests;
pub mod rpc;
pub mod wire;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use concat_core::Halt;
use concat_doc::commands::Command as DocCommand;
use concat_doc::model::{self as dm, ItemId, Model};
use concat_doc::{AnyId, Document};
use concat_effects::Catalogue;
use concat_effects::manifest::Kind;
use concat_export::{ExportClip, ExportRequest};
pub use concat_host::AppDirs;
use concat_host::cutout::{self, AnalyseRequest, Cutouts};
use concat_host::export::{self, Exporter};
use concat_host::preview::{FrameSpec, Monitor};
use concat_host::stabilize::{StabilizeRequest, Stabilizers};
use concat_host::templates::{self, SlotFill};
use concat_host::{ProjectInfo, Session, Titles, lock, media, projects};
pub use wire::{Command, VideoSettings};

pub use message::{
    API_VERSION, ApiError, CaptionSegment, ClipBounds, Dirs, Done, EditorView, ErrorCode, Event,
    ExportSpec, Fill, JobState, JobStatus, ModelInfo, ModelsInfo, PackageInfo, ParamInfo,
    PeaksInfo, Picture, Reply, Request, Response, SettingsView, ShapeCard, Started, VersionInfo,
    VoiceItem, Written,
};

/// The export sheet's middle quality, and what an export gets when the
/// caller says nothing.
const DEFAULT_CRF: u8 = 20;
/// The x264 preset every export uses unless told otherwise.
const DEFAULT_PRESET: &str = "medium";

/// What every build of the API serves, before what is decided by the
/// build or the embedder is added; see [`VersionInfo::capabilities`].
const CAPABILITIES: &[&str] = &["events", "captions", "models", "speech"];

/// The JSON Schema of [`Request`]: one alternative per method, each the
/// object its params are with `method` naming it, and every type a request
/// carries - an edit [`Command`] and all it holds - under `$defs`. The
/// descriptions are this crate's and `concat_doc`'s doc comments, so
/// what a caller is told about a field is what its maintainer wrote next
/// to it. Generated, never written by hand: a method or a command field
/// added to the types is in the schema on the next build.
#[cfg(feature = "schema")]
pub use contract::contract_schema;
pub use lend::Lender;

/// The JSON Schema of [`Request`] alone; see [`contract_schema`] for the
/// whole contract.
#[cfg(feature = "schema")]
pub fn request_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_default()
}

/// Where a job's events go. Called from the job's thread, so a transport
/// that writes them to a caller locks its writer inside.
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// The largest frame a preview or an export may ask for, a side: 8K, the
/// engine's own ceiling. A caller with the token is trusted to edit, not
/// to ask the machine for seventeen gigabytes of pixels (audit 2026-09-23,
/// #4).
pub const MAX_SIDE: u32 = concat_core::frame::MAX_SIDE;
/// The highest constant rate factor any codec here takes.
const MAX_CRF: u8 = 63;
/// The fastest frame rate an export may ask for.
const MAX_RATE: f64 = 240.0;

/// Who holds a project folder open on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    /// The window, on screen.
    Window,
    /// The API, for a caller on a socket.
    Api,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Holder::Window => "the window",
            Holder::Api => "the API",
        })
    }
}

/// Why a project folder could not be claimed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The other side of this process has it: the window, or the API it
    /// serves.
    Here(Holder),
    /// Another process has it - a second window, `concat --mcp`, the CLI -
    /// as its lock file says.
    Elsewhere(lock::Owner),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Here(holder) => write!(f, "open in {holder}"),
            Refusal::Elsewhere(owner) => {
                let what = if owner.role == "window" {
                    "a Concat window"
                } else {
                    "another Concat process"
                };
                match owner.pid {
                    Some(pid) => write!(f, "open in {what} (process {pid})"),
                    None => write!(f, "open in {what}"),
                }
            }
        }
    }
}

/// One claimed folder: who has it in this process, and the lock that
/// keeps every other process off it.
struct Claim {
    holder: Holder,
    /// `None` where the folder would not take a lock file - one that
    /// cannot be written - which costs the guard, not the open.
    _lock: Option<lock::ProjectLock>,
}

/// The project folders open on this machine and who has each, shared
/// between a window and the API it embeds, so neither opens a folder the
/// other is editing and saves over its work (audit 2026-09-23, #5). Cheap
/// to clone; every clone is the same register.
///
/// Each claimed folder is also locked against every other process
/// ([`lock::ProjectLock`]): a second window, `concat --mcp` working on its
/// own, the CLI. The lock goes with the claim, and with the process if it
/// dies, so a crash leaves a folder the next opener takes over.
#[derive(Clone, Default)]
pub struct OpenProjects(Arc<Mutex<BTreeMap<String, Claim>>>);

impl OpenProjects {
    /// Claims `path` for `holder`. Refused, naming who has it, when the
    /// other side of this process does or another process does; claiming
    /// again what one already holds is fine.
    pub fn claim(&self, path: &str, holder: Holder) -> Result<(), Refusal> {
        let key = key_of(path);
        let mut open = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match open.get(&key) {
            Some(claim) if claim.holder != holder => Err(Refusal::Here(claim.holder)),
            Some(_) => Ok(()),
            None => {
                let role = match holder {
                    Holder::Window => "window",
                    Holder::Api => "api",
                };
                let lock = match lock::ProjectLock::acquire(Path::new(path), role) {
                    Ok(lock) => Some(lock),
                    Err(lock::LockError::Held(owner)) => return Err(Refusal::Elsewhere(owner)),
                    // A folder that will not take the file: a read-only
                    // one, which nobody can save over anyway.
                    Err(lock::LockError::Io(_)) => None,
                };
                open.insert(
                    key,
                    Claim {
                        holder,
                        _lock: lock,
                    },
                );
                Ok(())
            }
        }
    }

    /// Gives `path` back, whoever had it.
    pub fn release(&self, path: &str) {
        let key = key_of(path);
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key);
    }

    /// Who has `path` open, if anyone.
    pub fn holder(&self, path: &str) -> Option<Holder> {
        let key = key_of(path);
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .map(|claim| claim.holder)
    }
}

/// The dispatcher: the open sessions and the services behind them.
pub struct Api {
    dirs: AppDirs,
    /// Open projects by canonical folder path.
    sessions: BTreeMap<String, Session>,
    /// Where this API may write: a created project, an instantiated
    /// template, an export or a preview file lands under one of these.
    /// Empty means anywhere, for the process's own owner at a terminal.
    roots: Vec<PathBuf>,
    /// Which project folders are open on this machine, across this API
    /// and a window it is embedded in.
    open: OpenProjects,
    /// What the window has lent this API of the projects it had open.
    lending: lend::Lending,
    titles: Titles,
    cutouts: Arc<Cutouts>,
    stabilizers: Arc<Stabilizers>,
    monitor: Monitor,
    exporter: Exporter,
    events: EventSink,
    jobs: Jobs,
    /// Every job's last known state and each running one's stop flag, fed
    /// by the events on their way to `events`.
    book: Arc<job::Book>,
    /// [`VersionInfo::capabilities`], in the order they were added.
    capabilities: Vec<String>,
    /// The speech model, kept loaded between transcriptions.
    transcriber: Arc<concat_speech::Transcriber>,
    /// The reading-aloud model, kept loaded between readings.
    speech: Arc<concat_speech::Speech>,
    /// The one-at-a-time slot reversed copies are written through.
    reversers: Arc<concat_host::Reversers>,
}

/// A way to stop the jobs of one [`Api`] from any thread, without waiting
/// for its thread to be free: what a server that is shutting down calls
/// before it waits for that thread. Cheap to clone.
#[derive(Clone)]
pub struct Stopper {
    book: Arc<job::Book>,
    cutouts: Arc<Cutouts>,
}

impl Stopper {
    /// How many jobs this API started are running now: what a stop would
    /// cancel, for a front end that asks before it stops them.
    pub fn running(&self) -> usize {
        self.book.running()
    }

    /// Asks every job this API started to stop at its next frame - its
    /// own and no other, even where the export slot is shared with a
    /// window. Each then ends with [`ErrorCode::Cancelled`]. How many
    /// were running.
    pub fn cancel_all(&self) -> usize {
        let running = self.book.cancel_all();
        if running > 0 {
            // A job still finding masks is in the analysis, not the
            // render; the analysis is this API's own.
            self.cutouts.cancel();
        }
        running
    }
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
        let book = Arc::new(job::Book::default());
        let events: EventSink = {
            let book = Arc::clone(&book);
            Arc::new(move |event: Event| {
                book.record(&event);
                events(event);
            })
        };
        Api {
            book,
            titles: Titles::new(&dirs),
            cutouts: Arc::new(Cutouts::new(&dirs.data)),
            stabilizers: Arc::new(Stabilizers::new()),
            monitor: Monitor::new(),
            exporter: Exporter::new(),
            sessions: BTreeMap::new(),
            roots: Vec::new(),
            open: OpenProjects::default(),
            lending: lend::Lending::default(),
            events,
            jobs: Jobs::default(),
            dirs,
            capabilities: CAPABILITIES.iter().map(|name| (*name).to_owned()).collect(),
            transcriber: Arc::new(concat_speech::Transcriber::new()),
            speech: Arc::new(concat_speech::Speech::new()),
            reversers: Arc::new(concat_host::Reversers::new()),
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

    /// What stops this API's jobs from another thread; see [`Stopper`].
    pub fn stopper(&self) -> Stopper {
        Stopper {
            book: Arc::clone(&self.book),
            cutouts: Arc::clone(&self.cutouts),
        }
    }

    /// [`Request::ExportStatus`].
    pub fn job_status(&mut self, job: &str) -> Result<JobStatus, ApiError> {
        self.jobs.reap(&self.book);
        self.book.status(job).ok_or_else(|| {
            ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job}: it never ran, or ended long ago"),
            )
        })
    }

    /// The export slot, for an embedder that shares it with a window.
    pub fn exporter(&self) -> Exporter {
        self.exporter.clone()
    }

    /// Takes an embedder's export slot in place of its own, so "one export
    /// at a time" holds across a window and this API together.
    pub fn share_exporter(&mut self, exporter: Exporter) {
        self.exporter = exporter;
    }

    /// Takes an embedder's register of open projects in place of its own,
    /// so a folder the window has open is refused here, and the other way
    /// round.
    pub fn share_open_projects(&mut self, open: OpenProjects) {
        self.open = open;
    }

    /// Takes the embedding window as a lender: from here on a project the
    /// window has open is asked for instead of refused; see [`Lender`].
    pub fn set_lender(&mut self, lender: Arc<dyn Lender>) {
        self.lending.set_lender(lender);
    }

    /// Confines every write to `roots`: from here on a created project, an
    /// instantiated template, an export and a preview file must land under
    /// one of them, and any other path is `refused`. Reads - a probe, a
    /// project opened by path - are not confined. An empty list confines
    /// nothing, which is right for the process's own owner at a terminal
    /// and wrong for a socket.
    pub fn restrict_writes_to(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots
            .into_iter()
            .map(|root| root.canonicalize().unwrap_or(root))
            .collect();
    }

    /// The roots writes are confined to; empty when they are not.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// `path` as somewhere this API may write, or why not.
    fn writable(&self, path: &str) -> Result<(), ApiError> {
        if self.roots.is_empty() {
            return Ok(());
        }
        let outside = || {
            let roots = self
                .roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            ApiError::new(
                ErrorCode::Refused,
                format!("{path} is outside where this API writes ({roots})"),
            )
        };
        let given = Path::new(path);
        // A `..` past the part of the path that exists cannot be resolved
        // and would walk out of a root on paper while staying under it in
        // this check; there is no reason a caller needs one.
        if given
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(outside());
        }
        let target = resolved(given);
        if self.roots.iter().any(|root| target.starts_with(root)) {
            Ok(())
        } else {
            Err(outside())
        }
    }

    /// How many jobs are still running.
    pub fn running(&mut self) -> usize {
        self.jobs.reap(&self.book);
        self.jobs.running.len()
    }

    /// Waits for every running job to end. A transport calls this before
    /// it exits, so an export begun on its last line still finishes.
    pub fn finish(&mut self) {
        self.jobs.finish();
    }

    /// Runs one request. The response is what the request is worth; a job
    /// it begins reports through the sink from then on.
    pub fn dispatch(&mut self, request: Request) -> Response {
        self.jobs.reap(&self.book);
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
            Request::ProjectGet { path } => view(view_of(self.session(&path)?, None)),
            Request::ProjectDocument { path } => Ok(Reply::Document(document_json(
                &self.session(&path)?.document(),
            ))),
            Request::ProjectSave { path, name } => {
                self.save(&path, name.as_deref())?;
                Ok(Reply::Done(Done {}))
            }
            Request::ProjectSetVideo { path, video } => {
                let frame = video.doc().map_err(ApiError::invalid)?;
                let session = self.session(&path)?;
                let sequence = session.showing().ok_or_else(|| {
                    ApiError::new(ErrorCode::Refused, "There is no timeline open")
                })?;
                view(self.apply_doc(&path, DocCommand::SetTimelineVideo { sequence, frame })?)
            }
            Request::EditApply { path, command } => view(self.apply(&path, *command)?),
            Request::EditUndo { path } => {
                let session = self.session_mut(&path)?;
                session.undo();
                view(view_of(session, None))
            }
            Request::EditRedo { path } => {
                let session = self.session_mut(&path)?;
                session.redo();
                view(view_of(session, None))
            }
            Request::MediaProbe { path } => {
                Ok(Reply::Media(media::probe(&path).map_err(ApiError::failed)?))
            }
            Request::MediaImport { path, file } => view(self.import(&path, &file)?),
            Request::CatalogueList { kind } => Ok(Reply::Packages(catalogue(kind.as_deref())?)),
            Request::TemplateList => Ok(Reply::Templates(templates::list(&self.dirs.config))),
            Request::ShapesList => Ok(Reply::Shapes(shape_cards())),
            Request::ClipsBounds { path } => Ok(Reply::Bounds(self.bounds(&path)?)),
            Request::ProjectCover {
                path,
                output,
                width,
                height,
            } => Ok(Reply::Written(self.cover(
                &path,
                &output,
                width.zip(height),
            )?)),
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
            Request::CaptionsTranscribe {
                path,
                clip_id,
                model,
                language,
            } => Ok(Reply::Started(
                self.transcribe(&path, &clip_id, model, language)?,
            )),
            Request::CaptionsPlace {
                path,
                clip_id,
                segments,
                placement,
                size,
            } => view(self.place_captions(
                &path,
                &clip_id,
                &segments,
                placement.as_deref(),
                size.as_deref(),
            )?),
            Request::CaptionsCancel { job } => {
                self.cancel_transcription(&job)?;
                Ok(Reply::Done(Done {}))
            }
            Request::ModelsList => Ok(Reply::Models(self.models()?)),
            Request::MediaFilmstrip {
                file,
                from,
                to,
                count,
                height,
            } => Ok(Reply::Picture(filmstrip(&file, from, to, count, height)?)),
            Request::MediaPeaks {
                path,
                file,
                stream,
                per_second,
            } => {
                let project = match path.as_deref() {
                    Some(path) => Some(self.session(path)?.path().to_owned()),
                    None => None,
                };
                Ok(Reply::Peaks(peaks(
                    &file,
                    stream,
                    project.as_deref(),
                    per_second,
                )?))
            }
            Request::ModelsDownload { kind, id } => {
                Ok(Reply::Started(self.download_model(&kind, id)?))
            }
            Request::ModelsDelete { kind, id } => {
                self.delete_model(&kind, &id)?;
                Ok(Reply::Done(Done {}))
            }
            Request::SpeechSpeak {
                path,
                text,
                model,
                voice,
                speed,
            } => Ok(Reply::Started(
                self.speak(&path, text, model, voice, speed)?,
            )),
            Request::SpeechCancel { job } => {
                self.cancel_reading(&job)?;
                Ok(Reply::Done(Done {}))
            }
            Request::ClipsFreeze {
                path,
                clip_id,
                time,
                duration,
            } => view(self.freeze(&path, &clip_id, time, duration)?),
            Request::ClipsReplace {
                path,
                clip_id,
                file,
            } => view(self.replace(&path, &clip_id, &file)?),
            Request::ClipsReverse { path, clip_id } => {
                Ok(Reply::Started(self.reverse(&path, &clip_id)?))
            }
            Request::ClipsUseReversed {
                path,
                clip_id,
                file,
                source_start,
                covered,
            } => view(self.use_reversed(&path, &clip_id, &file, source_start, covered)?),
            Request::ReverseCancel { job } => {
                self.cancel_reverse(&job)?;
                Ok(Reply::Done(Done {}))
            }
            Request::ExportCancel { job } => {
                self.cancel(&job)?;
                Ok(Reply::Done(Done {}))
            }
            Request::ExportStatus { job } => Ok(Reply::Job(self.job_status(&job)?)),
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
        self.writable(location)?;
        let frame = video.doc().map_err(ApiError::invalid)?;
        let info = projects::create(
            location,
            name,
            video.width,
            video.height,
            video.rate_num,
            video.rate_den,
            video.canvas_rgba().map_err(ApiError::invalid)?,
        )
        .map_err(ApiError::failed)?;
        let path = info.path.clone();
        let view = self.adopt(info)?;
        // The colour the timeline is output in is the document's, not the
        // manifest's.
        if frame.color != dm::ColorSpace::Sdr
            && let Some(sequence) = self.session(&path)?.showing()
        {
            return self.apply_doc(&path, DocCommand::SetTimelineVideo { sequence, frame });
        }
        Ok(view)
    }

    /// [`Request::ProjectOpen`].
    pub fn open(&mut self, path: &str) -> Result<EditorView, ApiError> {
        let key = key_of(path);
        if let Some(session) = self.sessions.get(&key) {
            return Ok(view_of(session, None));
        }
        let info = projects::open(path).map_err(ApiError::failed)?;
        self.adopt(info)
    }

    /// Opens a session on a project the host just described and puts it at
    /// the front of the recents list.
    fn adopt(&mut self, info: ProjectInfo) -> Result<EditorView, ApiError> {
        self.lending.borrow(&self.open, &info.path)?;
        if let Err(refusal) = self.open.claim(&info.path, Holder::Api) {
            self.lending.returned(&info.path);
            // A window holding it is the case an agent meets: say how to
            // go through that window instead of around it.
            let how = match &refusal {
                Refusal::Elsewhere(owner) if owner.role == "window" => {
                    ". To work on it from an agent, turn on Settings > Automation in that window and run the agent with `concat --mcp window`: the window then lends it until the agent closes it"
                }
                Refusal::Elsewhere(_) => ". Close it there first",
                Refusal::Here(_) => "",
            };
            return Err(ApiError::new(
                ErrorCode::Refused,
                format!("{} is {refusal}{how}", info.path),
            ));
        }
        let session = match Session::open_info(&info) {
            Ok(session) => session,
            Err(error) => {
                self.open.release(&info.path);
                self.lending.returned(&info.path);
                return Err(ApiError::failed(error));
            }
        };
        // Recents are a convenience for the launch screen; a machine whose
        // config folder cannot be written still edits.
        let _ = projects::remember(&self.dirs.config, &info);
        let view = view_of(&session, None);
        self.sessions.insert(key_of(&info.path), session);
        Ok(view)
    }

    /// [`Request::ProjectClose`].
    pub fn close(&mut self, path: &str, save: bool) -> Result<(), ApiError> {
        if save {
            self.save(path, None)?;
        }
        let closed = self
            .sessions
            .remove(&key_of(path))
            .map(drop)
            .ok_or_else(|| not_open(path));
        if closed.is_ok() {
            self.open.release(path);
            self.lending.returned(path);
        }
        closed
    }

    /// The graphics adapter `preview.frame` draws on, as its back end and
    /// its name, or None when it draws on the processor.
    pub fn adapter(&self) -> Option<(String, String)> {
        self.monitor.adapter()
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
        // A cover of a picture is a still of the bin: added first, when it
        // is not there, then named.
        if let Command::SetCover {
            cover: Some(wire::Cover::Picture { path: file }),
        } = &command
        {
            let held = self
                .session(path)?
                .document()
                .media
                .iter()
                .any(|(_, media)| media.path == *file);
            if !held {
                let media = media::probe(file).map_err(ApiError::failed)?.to_new_media();
                self.apply_doc(path, DocCommand::AddMedia { media })?;
            }
        }
        let document = self.session(path)?.document();
        let command = wire::to_doc(&document, &command).map_err(ApiError::invalid)?;
        self.apply_doc(path, command)
    }

    /// Applies one of the document's own commands.
    pub fn apply_doc(&mut self, path: &str, command: DocCommand) -> Result<EditorView, ApiError> {
        let session = self.session_mut(path)?;
        let change = session.apply(&command).map_err(refused)?;
        Ok(view_of(session, change.applied.created))
    }

    /// Probes `file` and adds it to the bin of the project at `path`.
    pub fn import(&mut self, path: &str, file: &str) -> Result<EditorView, ApiError> {
        let media = media::probe(file).map_err(ApiError::failed)?.to_new_media();
        self.apply_doc(path, DocCommand::AddMedia { media })
    }

    /// [`Request::TemplateInstantiate`].
    pub fn instantiate(
        &mut self,
        template: &str,
        location: &str,
        name: &str,
        fills: Vec<Fill>,
    ) -> Result<EditorView, ApiError> {
        self.writable(location)?;
        // Every file is probed before anything is made, so a bad path
        // refuses the whole request rather than leaving a folder behind.
        let fills = fills
            .into_iter()
            .map(|fill| {
                Ok(SlotFill {
                    media_id: fill.media_id.parse().map_err(|error| {
                        ApiError::invalid(format!("{:?} is no file's id: {error}", fill.media_id))
                    })?,
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
            &session.frame(),
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
        self.writable(&spec.output)?;
        let session = self.session(path)?;
        let document = session.document();
        if document.showing_end().display_seconds() <= 0.0 {
            return Err(ApiError::new(
                ErrorCode::Refused,
                "There is nothing on the timeline to export",
            ));
        }
        let frame = session.frame();
        let (width, height) = checked_size(
            spec.width.unwrap_or(frame.width.get()),
            spec.height.unwrap_or(frame.height.get()),
        )?;
        if let Some(crf) = spec.crf
            && crf > MAX_CRF
        {
            return Err(ApiError::invalid(format!("crf {crf} is over {MAX_CRF}")));
        }
        let rate_num = spec.rate_num.unwrap_or(i64::from(frame.rate.num()));
        let rate_den = spec.rate_den.unwrap_or(i64::from(frame.rate.den()));
        checked_rate(rate_num, rate_den)?;
        let hdr = frame.color != dm::ColorSpace::Sdr && spec.hdr.unwrap_or(true);
        let codec = match spec.codec.as_deref() {
            None if hdr => export::VideoCodec::Hevc,
            None => export::VideoCodec::H264,
            Some(name) => export::VideoCodec::parse(name).ok_or_else(|| {
                ApiError::invalid(format!(
                    "unknown codec {name:?}: h264, hevc, av1, prores-4444 or vp9-alpha"
                ))
            })?,
        };
        if hdr && codec == export::VideoCodec::H264 {
            return Err(ApiError::invalid(
                "H.264 does not carry HDR: hevc or av1, or hdr false for SDR".to_owned(),
            ));
        }
        let color_range = match spec.color_range.as_deref() {
            None => export::ColorRange::Limited,
            Some(name) => export::ColorRange::parse(name).ok_or_else(|| {
                ApiError::invalid(format!("unknown colour range {name:?}: limited or full"))
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
            rate_mode: export::RateMode::Vbr,
            bitrate_kbps: 0,
            color_range,
            hdr,
        };

        let project_path = session.path().to_owned();
        let project_dir = PathBuf::from(&project_path);
        let masks: Vec<(String, AnalyseRequest)> = Cutouts::requests(&document, &project_dir)
            .into_iter()
            .filter(|(_, request)| Cutouts::outstanding(request) != 0)
            .collect();
        let steadies: Vec<(String, StabilizeRequest)> =
            Stabilizers::requests(&document, &project_dir)
                .into_iter()
                .filter(|(_, request)| !Stabilizers::outstanding(request).is_empty())
                .collect();
        // In a worker where this program serves them, as the window's own
        // exports are: a crash there is an `export.failed` for the caller,
        // not this process gone (#762). The worker paints the titles.
        let render = match concat_host::export_apart::program() {
            Some(_) => Render::Apart(Box::new(concat_host::export_apart::Order::new(
                &document,
                project_dir.clone(),
                &host_spec,
                width,
                height,
                rate_num,
                rate_den,
            ))),
            None => {
                let titles = self.title_clips(session, width, height);
                let mut request = export::request(session, &host_spec, titles);
                request.width = width;
                request.height = height;
                request.rate_num = rate_num;
                request.rate_den = rate_den;
                Render::Here(Box::new(request))
            }
        };

        let slot = self
            .exporter
            .begin()
            .map_err(|message| ApiError::new(ErrorCode::Busy, message))?;
        let started = Started {
            job: self.jobs.mint(),
            path: project_path,
            output: spec.output.clone(),
        };
        self.book.start(
            JobStatus::running(&started.job, &started.path, &started.output),
            slot.cancel_handle(),
        );
        let job = ExportJob {
            name: started.job.clone(),
            path: started.path.clone(),
            render,
            width,
            height,
            masks,
            cutouts: Arc::clone(&self.cutouts),
            steadies,
            stabilizers: Arc::clone(&self.stabilizers),
            events: Arc::clone(&self.events),
        };
        // On the scheduler's foreground lane, across the cores it grants:
        // the export takes its turn with whatever else in the process
        // computes, the window's jobs included when the API is the
        // window's own.
        use concat_host::jobs::sched::{End, Job, Lane};
        let ticket = concat_host::jobs::scheduler().submit_then(
            Job::infallible(Lane::Foreground, move |_| job.run(slot))
                .label("export")
                .wide(),
            |_| {},
        );
        if let Some(End::Refused(why)) = ticket.end() {
            self.book.abandon(&started.job);
            return Err(ApiError::failed(format!(
                "could not start the export: {why:?}"
            )));
        }
        self.jobs.running.push((started.job.clone(), ticket));
        Ok(started)
    }

    /// [`Request::ExportCancel`].
    pub fn cancel(&mut self, job: &str) -> Result<(), ApiError> {
        self.jobs.reap(&self.book);
        if !self.jobs.running.iter().any(|(name, _)| name == job) {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job} is running"),
            ));
        }
        // Whichever phase it is in: the analysis ahead of the render, or
        // the render itself.
        self.cutouts.cancel();
        self.stabilizers.cancel();
        self.exporter.cancel();
        Ok(())
    }

    /// [`Request::ClipsBounds`]: each title's and shape's block as the
    /// painter left it at the timeline's own size, in fractions of it.
    fn bounds(&self, path: &str) -> Result<Vec<ClipBounds>, ApiError> {
        let session = self.session(path)?;
        let frame = session.frame();
        let (width, height) = checked_size(frame.width.get(), frame.height.get())?;
        let (w, h) = (f64::from(width), f64::from(height));
        Ok(self
            .titles
            .clips_live(&session.document(), width, height)
            .into_iter()
            .map(|title| ClipBounds {
                clip_id: title.clip_id.to_string(),
                width: f64::from(title.block.0) / w,
                height: f64::from(title.block.1) / h,
                offset_x: f64::from(title.offset.0) / w,
                offset_y: f64::from(title.offset.1) / h,
            })
            .collect())
    }

    /// The timeline's titles painted for a `width` × `height` frame, as
    /// the stills that stand in for them.
    fn title_clips(&self, session: &Session, width: u32, height: u32) -> Vec<ExportClip> {
        self.titles
            .clips(&session.document(), width, height)
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
        self.writable(output)?;
        let (width, height, pixels) = self.pixels(path, time, size)?;
        write_png(Path::new(output), width, height, &pixels)?;
        Ok(Written {
            path: output.to_owned(),
            width,
            height,
        })
    }

    /// [`Request::ProjectCover`]: the cover, to a file.
    pub fn cover(
        &mut self,
        path: &str,
        output: &str,
        size: Option<(u32, u32)>,
    ) -> Result<Written, ApiError> {
        self.writable(output)?;
        let session = self.session(path)?;
        let frame = session.frame();
        let (width, height) = size.unwrap_or((frame.width.get(), frame.height.get()));
        let (width, height) = checked_size(width, height)?;
        let document = session.document();
        let length = document.showing_end().display_seconds();
        let pixels = match document.cover.clone() {
            Some(dm::Cover::Picture { media }) => {
                let file = document
                    .media
                    .get(media)
                    .map(|media| media.path.clone())
                    .ok_or_else(|| ApiError::failed("the cover's picture is not in the bin"))?;
                picture_cover(&file, width, height)?
            }
            Some(dm::Cover::Frame { into }) => {
                self.pixels(
                    path,
                    into.display_seconds().min(length),
                    Some((width, height)),
                )?
                .2
            }
            _ => {
                self.pixels(path, early_frame(length), Some((width, height)))?
                    .2
            }
        };
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
        let settings = session.frame();
        let (width, height) = size.unwrap_or((settings.width.get(), settings.height.get()));
        let (width, height) = checked_size(width, height)?;
        let mut clips = session.flattened_clips();
        // Painted at the timeline's size in memory, whatever size this
        // frame is: the compositor scales them as it scales any picture.
        clips.extend(self.title_pictures(path)?);
        let pixels = self
            .monitor
            .frame(
                Arc::new(clips),
                &settings,
                FrameSpec {
                    time,
                    width,
                    height,
                    moving: false,
                    proxy: false,
                    color_space: concat_export::ColorSpace::of(&settings.color),
                    canvas: [
                        settings.canvas.r,
                        settings.canvas.g,
                        settings.canvas.b,
                        settings.canvas.a,
                    ],
                    nearest: false,
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

    /// [`Request::CaptionsTranscribe`]: the clip's sound heard on the
    /// scheduler's background lane, its words coming back as an event.
    fn transcribe(
        &mut self,
        path: &str,
        clip_id: &str,
        model: String,
        language: Option<String>,
    ) -> Result<Started, ApiError> {
        let (request, project_path) = {
            let session = self.session(path)?;
            let clip = facts(&session.document(), clip_id)?;
            let media = clip
                .media
                .as_ref()
                .ok_or_else(|| ApiError::invalid(format!("clip {clip_id} has no file to hear")))?;
            let request = concat_speech::transcribe::TranscribeRequest {
                path: media.path.clone(),
                audio_stream: clip.stream,
                source_start: clip.source_start,
                window: clip.duration * clip.speed,
                language: (!concat_speech::transcribe::english_only(&model))
                    .then_some(language)
                    .flatten(),
                model_id: model,
            };
            (request, session.path().to_owned())
        };
        let started = Started {
            job: self.jobs.mint(),
            path: project_path.clone(),
            output: String::new(),
        };
        let job = started.job.clone();
        let clip_id = clip_id.to_owned();
        let transcriber = Arc::clone(&self.transcriber);
        let dirs = self.dirs.clone();
        let events = Arc::clone(&self.events);
        self.background(started, "transcription", move || {
            let said = {
                let (job, path, events) = (job.clone(), project_path.clone(), Arc::clone(&events));
                move |stage: concat_speech::JobStage| {
                    let (stage, fraction) = match stage {
                        concat_speech::JobStage::Reading => ("reading", 0.0),
                        concat_speech::JobStage::Loading => ("loading", 0.0),
                        concat_speech::JobStage::Working { done, .. } => ("hearing", done),
                    };
                    events(Event::CaptionsProgress {
                        job: job.clone(),
                        path: path.clone(),
                        stage: stage.to_owned(),
                        fraction,
                    });
                }
            };
            match transcriber.transcribe(&dirs, &request, said) {
                Ok(segments) => Event::CaptionsDone {
                    job,
                    path: project_path,
                    clip_id,
                    segments: segments
                        .into_iter()
                        .map(|segment| CaptionSegment {
                            start: segment.start,
                            end: segment.end,
                            text: segment.text,
                        })
                        .collect(),
                },
                Err(halt) => Event::CaptionsFailed {
                    job,
                    path: project_path,
                    error: halt_error(halt, "The transcription was cancelled"),
                },
            }
        })
    }

    /// [`Request::CaptionsPlace`]: one title a stretch of speech, over the
    /// clip it was heard in, in one undo step.
    fn place_captions(
        &mut self,
        path: &str,
        clip_id: &str,
        segments: &[CaptionSegment],
        placement: Option<&str>,
        size: Option<&str>,
    ) -> Result<EditorView, ApiError> {
        use concat_host::captions::{
            CAPTION_OFFSETS, CAPTION_SIZES, Heard, heard_cues, place_captions,
        };
        let offset = match placement.unwrap_or("bottom") {
            "bottom" => CAPTION_OFFSETS[0],
            "middle" => CAPTION_OFFSETS[1],
            "top" => CAPTION_OFFSETS[2],
            other => {
                return Err(ApiError::invalid(format!(
                    "unknown placement {other:?}: top, middle or bottom"
                )));
            }
        };
        let font_size = match size.unwrap_or("medium") {
            "small" => CAPTION_SIZES[0],
            "medium" => CAPTION_SIZES[1],
            "large" => CAPTION_SIZES[2],
            other => {
                return Err(ApiError::invalid(format!(
                    "unknown size {other:?}: small, medium or large"
                )));
            }
        };
        let (start, speed) = {
            let session = self.session(path)?;
            let clip = facts(&session.document(), clip_id)?;
            (clip.start, clip.speed)
        };
        let heard: Vec<Heard> = segments
            .iter()
            .map(|segment| Heard {
                start: segment.start,
                end: segment.end,
                text: segment.text.clone(),
            })
            .collect();
        let Some(command) =
            place_captions(heard_cues(&heard, start, speed), (offset, font_size), None)
        else {
            return Err(ApiError::invalid("there are no words to place"));
        };
        self.apply_doc(path, command)
    }

    fn models(&self) -> Result<ModelsInfo, ApiError> {
        let heard = concat_speech::Transcriber::status(&self.dirs).map_err(ApiError::failed)?;
        let read = concat_speech::Speech::status(&self.dirs).map_err(ApiError::failed)?;
        let family = |family: concat_speech::tts::Family| match family {
            concat_speech::tts::Family::Chatterbox => "chatterbox".to_owned(),
            concat_speech::tts::Family::System => "system".to_owned(),
        };
        Ok(ModelsInfo {
            transcribers: heard
                .models
                .into_iter()
                .map(|model| ModelInfo {
                    id: model.id,
                    label: model.label,
                    blurb: model.blurb,
                    family: String::new(),
                    english_only: model.english_only,
                    size_bytes: model.size_bytes,
                    downloaded: model.downloaded,
                })
                .collect(),
            speech: read
                .models
                .into_iter()
                .map(|model| ModelInfo {
                    id: model.id,
                    label: model.label,
                    blurb: model.blurb,
                    family: family(model.family),
                    english_only: false,
                    size_bytes: model.size_bytes,
                    downloaded: model.downloaded,
                })
                .collect(),
            voices: read
                .voices
                .into_iter()
                .map(|voice| VoiceItem {
                    id: voice.id,
                    name: voice.name,
                    family: family(voice.family),
                })
                .collect(),
        })
    }

    /// Starts `work` on the scheduler's background lane as job `started`,
    /// its outcome the event `work` hands back.
    fn background(
        &mut self,
        started: Started,
        label: &'static str,
        work: impl FnOnce() -> Event + Send + 'static,
    ) -> Result<Started, ApiError> {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.book
            .start(JobStatus::running(&started.job, &started.path, ""), cancel);
        let events = Arc::clone(&self.events);
        use concat_host::jobs::sched::{End, Job, Lane};
        let ticket = concat_host::jobs::scheduler().submit_then(
            Job::infallible(Lane::Background, move |_| events(work())).label(label),
            |_| {},
        );
        if let Some(End::Refused(why)) = ticket.end() {
            self.book.abandon(&started.job);
            return Err(ApiError::failed(format!(
                "could not start the {label}: {why:?}"
            )));
        }
        self.jobs.running.push((started.job.clone(), ticket));
        Ok(started)
    }

    /// [`Request::ModelsDownload`].
    fn download_model(&mut self, kind: &str, id: String) -> Result<Started, ApiError> {
        let speech = match kind {
            "transcriber" => false,
            "speech" => true,
            other => {
                return Err(ApiError::invalid(format!(
                    "unknown kind {other:?}: transcriber or speech"
                )));
            }
        };
        let started = Started {
            job: self.jobs.mint(),
            path: String::new(),
            output: String::new(),
        };
        let job = started.job.clone();
        let dirs = self.dirs.clone();
        let events = Arc::clone(&self.events);
        let transcriber = Arc::clone(&self.transcriber);
        let reader = Arc::clone(&self.speech);
        self.background(started, "model download", move || {
            let progress = {
                let (job, events) = (job.clone(), Arc::clone(&events));
                move |step: concat_speech::DownloadProgress| {
                    events(Event::ModelsProgress {
                        job: job.clone(),
                        id: step.id,
                        received: step.received,
                        total: step.total,
                        unpacking: step.unpacking,
                    });
                }
            };
            let outcome = if speech {
                reader.download_model(&dirs, &id, progress)
            } else {
                transcriber.download_model(&dirs, &id, progress)
            };
            match outcome {
                Ok(()) => Event::ModelsDone { job, id },
                Err(halt) => Event::ModelsFailed {
                    job,
                    id,
                    error: halt_error(halt, "The download was cancelled"),
                },
            }
        })
    }

    /// [`Request::ModelsDelete`].
    fn delete_model(&mut self, kind: &str, id: &str) -> Result<(), ApiError> {
        match kind {
            "transcriber" => self.transcriber.delete_model(&self.dirs, id),
            "speech" => self.speech.delete_model(&self.dirs, id),
            other => {
                return Err(ApiError::invalid(format!(
                    "unknown kind {other:?}: transcriber or speech"
                )));
            }
        }
        .map_err(ApiError::failed)
    }

    /// [`Request::SpeechSpeak`].
    fn speak(
        &mut self,
        path: &str,
        text: String,
        model: String,
        voice: i32,
        speed: Option<f32>,
    ) -> Result<Started, ApiError> {
        if text.trim().is_empty() {
            return Err(ApiError::invalid("there is no text to read"));
        }
        let project = self.session(path)?.path().to_owned();
        let request = concat_speech::tts::SpeakRequest {
            model_id: model,
            voice,
            reference: None,
            text,
            speed: speed.unwrap_or(1.0),
            project: project.clone(),
        };
        let started = Started {
            job: self.jobs.mint(),
            path: project.clone(),
            output: String::new(),
        };
        let job = started.job.clone();
        let dirs = self.dirs.clone();
        let events = Arc::clone(&self.events);
        let reader = Arc::clone(&self.speech);
        self.background(started, "reading", move || {
            let progress = {
                let (job, path, events) = (job.clone(), project.clone(), Arc::clone(&events));
                move |stage: concat_speech::JobStage| {
                    let (stage, fraction) = match stage {
                        concat_speech::JobStage::Reading | concat_speech::JobStage::Loading => {
                            ("loading", 0.0)
                        }
                        concat_speech::JobStage::Working { done, .. } => ("reading", done),
                    };
                    events(Event::SpeechProgress {
                        job: job.clone(),
                        path: path.clone(),
                        stage: stage.to_owned(),
                        fraction,
                    });
                }
            };
            match reader.speak(&dirs, &request, progress) {
                Ok(said) => Event::SpeechDone {
                    job,
                    path: project,
                    file: said.path,
                    duration: said.duration,
                },
                Err(halt) => Event::SpeechFailed {
                    job,
                    path: project,
                    error: halt_error(halt, "The reading was cancelled"),
                },
            }
        })
    }

    /// [`Request::SpeechCancel`].
    fn cancel_reading(&mut self, job: &str) -> Result<(), ApiError> {
        self.jobs.reap(&self.book);
        if !self.jobs.running.iter().any(|(name, _)| name == job) {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job} is running"),
            ));
        }
        self.speech.cancel();
        Ok(())
    }

    /// [`Request::ClipsFreeze`]: what the window's Freeze does - the still
    /// written through the same `concat_host::media::freeze_still`, then
    /// the command that splits and holds.
    fn freeze(
        &mut self,
        path: &str,
        clip_id: &str,
        time: f64,
        duration: Option<f64>,
    ) -> Result<EditorView, ApiError> {
        let (item, still) = {
            let session = self.session(path)?;
            let clip = facts(&session.document(), clip_id)?;
            let still = match clip.kind {
                Some(ClipKind::Image) => None,
                Some(ClipKind::Video) => {
                    if time <= clip.start || time >= clip.start + clip.duration {
                        return Err(ApiError::invalid(format!(
                            "{time} s is not inside clip {clip_id}"
                        )));
                    }
                    let media = clip.media.as_ref().ok_or_else(|| {
                        ApiError::invalid(format!("clip {clip_id} has no file to hold"))
                    })?;
                    let seconds = clip.source_start + (time - clip.start) * clip.speed;
                    Some(
                        media::freeze_still(session.path(), &media.path, clip_id, seconds)
                            .map_err(ApiError::failed)?,
                    )
                }
                _ => {
                    return Err(ApiError::invalid(format!(
                        "clip {clip_id} is not a picture: only a video or an image holds"
                    )));
                }
            };
            (clip.id, still)
        };
        let at = concat_doc::Tick::from_display_seconds(time)
            .ok_or_else(|| ApiError::invalid("the time is out of range"))?;
        let len = concat_doc::Tick::from_display_seconds(duration.unwrap_or(1.0).max(0.0))
            .and_then(|end| concat_doc::TickLen::try_from(end.get()).ok());
        self.apply_doc(
            path,
            DocCommand::FreezeFrame {
                item,
                at,
                len,
                still,
            },
        )
    }

    /// [`Request::ClipsReplace`].
    fn replace(&mut self, path: &str, clip_id: &str, file: &str) -> Result<EditorView, ApiError> {
        let (item, start, covered) = {
            let clip = facts(&self.session(path)?.document(), clip_id)?;
            (clip.id, clip.source_start, clip.duration * clip.speed)
        };
        let summary = media::probe(file).map_err(ApiError::failed)?;
        // A still has no in-point; a file too short to show the clip's
        // span from its in-point shows it from its start.
        let from = match summary.duration {
            Some(length) if start + covered > length => 0.0,
            _ => start,
        };
        self.apply_doc(
            path,
            DocCommand::ReplaceClipMedia {
                item,
                file: summary.to_new_media(),
                enhanced: false,
                copy_start: concat_doc::TickDur::ZERO,
                source_start: concat_doc::Tick::from_display_seconds(from.max(0.0)),
            },
        )
    }

    /// [`Request::ClipsReverse`]: the span the clip shows of its file -
    /// its in-point, and its length at its rate - written backwards on the
    /// scheduler's background lane, as the window's Reverse does.
    fn reverse(&mut self, path: &str, clip_id: &str) -> Result<Started, ApiError> {
        if self.reversers.is_busy() {
            return Err(ApiError::failed("a reverse is already running"));
        }
        let (request, project_path) = {
            let session = self.session(path)?;
            let clip = facts(&session.document(), clip_id)?;
            let audio_only = match clip.kind {
                Some(ClipKind::Video) => false,
                Some(ClipKind::Audio) => true,
                _ => {
                    return Err(ApiError::invalid(format!(
                        "clip {clip_id} is neither video nor sound: nothing to reverse"
                    )));
                }
            };
            let media = clip.media.as_ref().ok_or_else(|| {
                ApiError::invalid(format!("clip {clip_id} has no file to reverse"))
            })?;
            let start = clip.source_start;
            let covered = clip.duration * clip.speed;
            let target = concat_host::reverse::target_for(
                Path::new(session.path()),
                &media.path,
                start,
                covered,
                audio_only,
            )
            .ok_or_else(|| ApiError::failed(format!("could not read {}", media.path)))?;
            let request = concat_host::ReverseRequest {
                media_path: media.path.clone(),
                audio_only,
                start,
                duration: covered,
                target,
            };
            (request, session.path().to_owned())
        };
        let started = Started {
            job: self.jobs.mint(),
            path: project_path.clone(),
            output: String::new(),
        };
        let job = started.job.clone();
        let clip_id = clip_id.to_owned();
        let events = Arc::clone(&self.events);
        let reversers = Arc::clone(&self.reversers);
        self.background(started, "reverse", move || {
            let mut last = -1.0f32;
            let mut progress = |fraction: f32| {
                if fraction - last >= 0.01 {
                    last = fraction;
                    events(Event::ReverseProgress {
                        job: job.clone(),
                        path: project_path.clone(),
                        clip_id: clip_id.clone(),
                        fraction,
                    });
                }
            };
            match reversers.reverse(&request, &mut progress) {
                Ok(file) => Event::ReverseDone {
                    job,
                    path: project_path,
                    clip_id,
                    file: file.to_string_lossy().into_owned(),
                    source_start: request.start,
                    covered: request.duration,
                },
                Err(halt) => Event::ReverseFailed {
                    job,
                    path: project_path,
                    error: halt_error(halt, "The reverse was cancelled"),
                },
            }
        })
    }

    /// [`Request::ClipsUseReversed`]: the clip re-pointed at its reversed
    /// copy, unless it has moved on its file since the copy was asked for.
    fn use_reversed(
        &mut self,
        path: &str,
        clip_id: &str,
        file: &str,
        source_start: f64,
        covered: f64,
    ) -> Result<EditorView, ApiError> {
        let (clip_item, mut item) = {
            let session = self.session(path)?;
            let clip = facts(&session.document(), clip_id)?;
            if (clip.source_start - source_start).abs() > 1e-6
                || (clip.duration * clip.speed - covered).abs() > 1e-6
            {
                return Err(ApiError::invalid(format!(
                    "clip {clip_id} changed while it was being reversed: reverse it again"
                )));
            }
            let mut item = media::probe(file).map_err(ApiError::failed)?.to_new_media();
            if let Some(original) = &clip.media {
                item.name = format!("{} (reversed)", original.name);
            }
            (clip.id, item)
        };
        item.origin = Some(dm::MediaOrigin::Processed);
        self.apply_doc(
            path,
            DocCommand::ReplaceClipMedia {
                item: clip_item,
                file: item,
                enhanced: false,
                copy_start: concat_doc::TickDur::ZERO,
                source_start: Some(concat_doc::Tick::ZERO),
            },
        )
    }

    /// [`Request::ReverseCancel`].
    fn cancel_reverse(&mut self, job: &str) -> Result<(), ApiError> {
        self.jobs.reap(&self.book);
        if !self.jobs.running.iter().any(|(name, _)| name == job) {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job} is running"),
            ));
        }
        self.reversers.cancel();
        Ok(())
    }

    /// [`Request::CaptionsCancel`].
    fn cancel_transcription(&mut self, job: &str) -> Result<(), ApiError> {
        self.jobs.reap(&self.book);
        if !self.jobs.running.iter().any(|(name, _)| name == job) {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no job {job} is running"),
            ));
        }
        self.transcriber.cancel();
        Ok(())
    }

    /// The open project at `path`, read-only: for an embedder that shows it
    /// as well as editing it through requests - the phone app's preview,
    /// which draws and plays the edit this API holds, so the two never open
    /// the folder twice. An error when it is not open here.
    pub fn open_session(&self, path: &str) -> Result<&Session, ApiError> {
        self.session(path)
    }

    /// The open project's titles, captions and shapes as the pictures the
    /// compositor draws, for an embedder that shows the edit itself: each
    /// painted in memory at the timeline's own size - once, whatever size
    /// the preview is drawn at - and held in the process's reader pool
    /// under a name no file has, which any monitor of this process reads.
    /// Only the ones not painted yet are painted; an edit to one caption
    /// paints that one. Add them to [`Session::flattened_clips`], which
    /// leaves them out.
    pub fn title_pictures(&self, path: &str) -> Result<Vec<ExportClip>, ApiError> {
        let session = self.session(path)?;
        let frame = session.frame();
        let (width, height) = checked_size(frame.width.get(), frame.height.get())?;
        Ok(self
            .titles
            .clips_live(&session.document(), width, height)
            .into_iter()
            .map(|title| {
                if let Some(held) = title.held {
                    self.monitor.hold_sparse(Path::new(&title.clip.path), held);
                }
                title.clip
            })
            .collect())
    }

    fn session_mut(&mut self, path: &str) -> Result<&mut Session, ApiError> {
        self.sessions
            .get_mut(&key_of(path))
            .ok_or_else(|| not_open(path))
    }
}

/// [`Request::MediaFilmstrip`].
fn filmstrip(
    file: &str,
    from: Option<f64>,
    to: Option<f64>,
    count: u32,
    height: u32,
) -> Result<Picture, ApiError> {
    let strip = media::filmstrip_between(
        file,
        from.unwrap_or(0.0),
        to.unwrap_or(f64::INFINITY),
        count,
        height,
    )
    .map_err(ApiError::failed)?;
    let mut png = Vec::new();
    encode_png(&mut png, strip.width(), strip.height(), strip.pixels())
        .map_err(|error| ApiError::failed(format!("could not encode the strip: {error}")))?;
    Ok(Picture {
        width: strip.width(),
        height: strip.height(),
        png: base64::engine::general_purpose::STANDARD.encode(png),
    })
}

/// [`Request::MediaPeaks`]: the file's outline, gathered down to
/// `per_second` slices a second.
fn peaks(
    file: &str,
    stream: Option<u32>,
    project: Option<&str>,
    per_second: Option<f32>,
) -> Result<PeaksInfo, ApiError> {
    let peaks = media::peaks(file, stream, project).map_err(ApiError::failed)?;
    Ok(gather_peaks(
        &peaks.min,
        &peaks.max,
        peaks.buckets_per_second,
        per_second.unwrap_or(20.0),
    ))
}

/// Slices of `own` a second gathered into `wanted` a second, each the
/// lowest and highest of the slices it covers. Never finer than `own`.
fn gather_peaks(min: &[f32], max: &[f32], own: f32, wanted: f32) -> PeaksInfo {
    let wanted = wanted.clamp(1.0, own.max(1.0));
    let step = (own / wanted).max(1.0) as f64;
    let total = min.len().min(max.len());
    let slices = ((total as f64) / step).ceil() as usize;
    let mut out_min = Vec::with_capacity(slices);
    let mut out_max = Vec::with_capacity(slices);
    for slice in 0..slices {
        let first = (slice as f64 * step) as usize;
        let last = (((slice + 1) as f64 * step) as usize)
            .min(total)
            .max(first + 1);
        let low = min[first..last].iter().copied().fold(0.0_f32, f32::min);
        let high = max[first..last].iter().copied().fold(0.0_f32, f32::max);
        out_min.push(low);
        out_max.push(high);
    }
    PeaksInfo {
        min: out_min,
        max: out_max,
        per_second: (own as f64 / step) as f32,
    }
}

/// A job's halt as the error its failed event carries.
fn halt_error(halt: Halt, cancelled: &str) -> ApiError {
    match halt {
        Halt::Cancelled => ApiError::new(ErrorCode::Cancelled, cancelled),
        Halt::Failed(message) => ApiError::failed(message),
    }
}

/// The jobs the API has begun, by name, with the scheduler's ticket for
/// each.
#[derive(Default)]
struct Jobs {
    minted: u64,
    running: Vec<(String, concat_host::jobs::sched::Ticket)>,
}

impl Jobs {
    /// The next name.
    fn mint(&mut self) -> String {
        self.minted += 1;
        format!("j{}", self.minted)
    }

    /// Forgets the jobs that have ended. One that ended without its last
    /// event - a panic in the render - is booked as failed.
    fn reap(&mut self, book: &job::Book) {
        self.running.retain(|(name, ticket)| {
            let ended = ticket.is_finished();
            if ended {
                book.abandon(name);
            }
            !ended
        });
    }

    /// Waits for the rest.
    fn finish(&mut self) {
        for (_, ticket) in self.running.drain(..) {
            while ticket.wait(std::time::Duration::from_secs(60)).is_none() {}
        }
    }
}

/// Where an export is drawn.
enum Render {
    /// In this process, from the request made here.
    Here(Box<ExportRequest>),
    /// In a worker of [`concat_host::export_apart::program`], from the
    /// order it makes its request from; in this process if none starts.
    Apart(Box<concat_host::export_apart::Order>),
}

/// An export in flight: everything its thread needs, owned.
struct ExportJob {
    name: String,
    path: String,
    render: Render,
    width: u32,
    height: u32,
    masks: Vec<(String, AnalyseRequest)>,
    cutouts: Arc<Cutouts>,
    steadies: Vec<(String, StabilizeRequest)>,
    stabilizers: Arc<Stabilizers>,
    events: EventSink,
}

impl ExportJob {
    /// The whole job, start to end, on the thread it is called from. The
    /// slot is held until this returns, so the next export waits its turn.
    fn run(self, slot: concat_host::jobs::Job) {
        let outcome = self
            .masks(&slot)
            .and_then(|()| self.steadies(&slot))
            .and_then(|()| self.draw(&slot));
        let event = match outcome {
            Ok(output) => Event::ExportDone {
                job: self.name.clone(),
                path: self.path.clone(),
                output,
                width: self.width,
                height: self.height,
            },
            // Which of the two it was is the job's own answer: the flag
            // may be raised a moment after a failure, and a failure's words
            // may say anything.
            Err(halt) => Event::ExportFailed {
                job: self.name.clone(),
                path: self.path.clone(),
                error: match halt {
                    Halt::Cancelled => {
                        ApiError::new(ErrorCode::Cancelled, "The export was cancelled")
                    }
                    Halt::Failed(message) => ApiError::failed(message),
                },
            },
        };
        (self.events)(event);
    }

    /// The render itself, where [`Render`] says.
    fn draw(&self, slot: &concat_host::jobs::Job) -> Result<String, Halt> {
        let mut progress = |progress: export::Progress| {
            (self.events)(Event::progress(&self.name, &self.path, progress));
        };
        let order = match &self.render {
            Render::Here(request) => {
                return export::run(request, slot.cancel_flag(), progress);
            }
            Render::Apart(order) => order,
        };
        // The idle preview decoders let go first, as the window's export
        // does: the chip has few decoder sessions, and the worker's own
        // decoders need them.
        concat_host::scheduler()
            .pool()
            .close_idle(std::time::Duration::ZERO);
        if let Some(program) = concat_host::export_apart::program()
            && let Some(result) =
                concat_host::export_apart::run(program, order, slot.cancel_flag(), &mut progress)
        {
            return result;
        }
        concat_host::export_apart::run_here(order, slot.cancel_flag(), &mut progress)
    }

    /// Runs every cutout analysis the timeline still needs, one after the
    /// other, so the render that follows cuts every clip it should.
    fn masks(&self, slot: &concat_host::jobs::Job) -> Result<(), Halt> {
        for (media_id, request) in &self.masks {
            if slot.cancelled() {
                return Err(Halt::Cancelled);
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

impl ExportJob {
    /// Measures every stabilized clip's media the timeline still needs,
    /// one after the other, so the render that follows steadies every
    /// clip it should.
    fn steadies(&self, slot: &concat_host::jobs::Job) -> Result<(), Halt> {
        for (media_id, request) in &self.steadies {
            if slot.cancelled() {
                return Err(Halt::Cancelled);
            }
            self.stabilizers.analyse(request, &mut |fraction| {
                (self.events)(Event::StabilizeProgress {
                    job: self.name.clone(),
                    path: self.path.clone(),
                    media_id: media_id.clone(),
                    fraction,
                });
            })?;
        }
        Ok(())
    }
}

/// The frame a project is shown by when it names no cover: a quarter of
/// the way in, and never past two seconds, so a long edit is known by its
/// opening rather than by some middle.
fn early_frame(length: f64) -> f64 {
    (length * 0.25).clamp(0.0, 2.0)
}

/// A picture from disk as a `width` × `height` cover: fitted to fill it,
/// cut at the edges that spill, as a card crops.
fn picture_cover(file: &str, width: u32, height: u32) -> Result<Vec<u8>, ApiError> {
    let summary = media::probe(file).map_err(ApiError::failed)?;
    let video = summary
        .video
        .ok_or_else(|| ApiError::invalid(format!("{file} has no picture")))?;
    let aspect = f64::from(video.width.max(1)) / f64::from(video.height.max(1));
    // Decoded just wide enough that its height covers the frame's too.
    let across = (f64::from(height) * aspect).ceil().max(f64::from(width)) as u32;
    let still = media::still_at(file, 0.0, across).map_err(ApiError::failed)?;
    let (sw, sh) = (still.width(), still.height());
    let scale = (f64::from(sw) / f64::from(width)).min(f64::from(sh) / f64::from(height));
    let left = (f64::from(sw) - f64::from(width) * scale) / 2.0;
    let top = (f64::from(sh) - f64::from(height) * scale) / 2.0;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let sy = ((top + (f64::from(y) + 0.5) * scale) as u32).min(sh - 1);
        for x in 0..width {
            let sx = ((left + (f64::from(x) + 0.5) * scale) as u32).min(sw - 1);
            pixels.extend_from_slice(&still.pixel(sx, sy).unwrap_or([0, 0, 0, 255]));
        }
    }
    Ok(pixels)
}

/// [`Request::ShapesList`]: every Stickers card with the look it places.
fn shape_cards() -> Vec<ShapeCard> {
    wire::shape_cards()
        .into_iter()
        .map(|(id, style)| ShapeCard { id, style })
        .collect()
}

/// The view a reply carries: the document as its file writes it, the
/// history's state, the settings, and the id an edit made.
fn view_of(session: &Session, created: Option<AnyId>) -> EditorView {
    let frame = session.frame();
    EditorView {
        project: document_json(&session.document()),
        can_undo: session.can_undo(),
        can_redo: session.can_redo(),
        settings: SettingsView {
            name: session.name().to_owned(),
            width: frame.width.get(),
            height: frame.height.get(),
            rate_num: i64::from(frame.rate.num()),
            rate_den: i64::from(frame.rate.den()),
        },
        created_id: created.map(|id| id.to_string()),
    }
}

/// A document as the JSON its file is.
fn document_json(document: &Document<Model>) -> serde_json::Value {
    serde_json::from_str(&concat_doc::file::write(document)).unwrap_or_default()
}

/// What a clip is, as a file's clip: video, sound or still.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ClipKind {
    Video,
    Audio,
    Image,
}

/// A clip, in the seconds the API speaks.
struct Facts {
    id: ItemId,
    kind: Option<ClipKind>,
    media: Option<dm::Media>,
    stream: Option<u32>,
    start: f64,
    duration: f64,
    source_start: f64,
    speed: f64,
}

/// The clip `clip_id` names, in seconds.
fn facts(document: &Model, clip_id: &str) -> Result<Facts, ApiError> {
    let no_clip = || ApiError::invalid(format!("no clip {clip_id} on the timeline"));
    let id: ItemId = clip_id.parse().map_err(|_| no_clip())?;
    let item = document.items.get(id).ok_or_else(no_clip)?;
    let start = item.start.display_seconds();
    let duration = concat_doc::TickDur::from(item.len).display_seconds();
    let (mut kind, mut media, mut stream, mut source_start, mut speed) =
        (None, None, None, 0.0, 1.0);
    if let dm::Content::Media(cut) = &item.content {
        let file = document
            .media
            .get(cut.media)
            .map(|file| file.as_ref().clone());
        kind = file
            .as_ref()
            .and_then(|file| match (&file.kind, &cut.take) {
                (dm::MediaKind::Audio, _) | (_, dm::Take::Sound) => Some(ClipKind::Audio),
                (dm::MediaKind::Image, _) => Some(ClipKind::Image),
                (dm::MediaKind::Video, _) => Some(ClipKind::Video),
                _ => None,
            });
        media = file;
        stream = cut.stream;
        let from = cut
            .time
            .source_of(item.offset)
            .map_or(0.0, |t| t.display_seconds());
        let to = cut
            .time
            .source_of(item.offset + item.len)
            .map_or(from, |t| t.display_seconds());
        source_start = from.min(to);
        if duration > 0.0 {
            speed = ((to - from) / duration).abs();
        }
    }
    Ok(Facts {
        id,
        kind,
        media,
        stream,
        start,
        duration,
        source_start,
        speed,
    })
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
impl Drop for Api {
    /// The register is shared with an embedder that outlives this API:
    /// what it held open is given back.
    fn drop(&mut self) {
        for key in self.sessions.keys() {
            self.open.release(key);
        }
    }
}

/// A frame size a caller may ask for, or why not.
fn checked_size(width: u32, height: u32) -> Result<(u32, u32), ApiError> {
    if width == 0 || height == 0 {
        return Err(ApiError::invalid("A frame needs a width and a height"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(ApiError::invalid(format!(
            "{width}×{height} is over {MAX_SIDE} a side"
        )));
    }
    Ok((width, height))
}

/// A frame rate a caller may ask for, or why not.
fn checked_rate(num: i64, den: i64) -> Result<(), ApiError> {
    if num <= 0 || den <= 0 {
        return Err(ApiError::invalid(
            "A frame rate needs a positive numerator and denominator",
        ));
    }
    if num as f64 / den as f64 > MAX_RATE {
        return Err(ApiError::invalid(format!(
            "{num}/{den} is over {MAX_RATE} frames a second"
        )));
    }
    Ok(())
}

/// `path` with its symlinks resolved as far as it exists: the deepest
/// ancestor that is there, canonicalised, with the rest appended. A file
/// not written yet still says where it would land.
fn resolved(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

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
    use crate::wire::{MediaKind, NewMedia};
    use serde_json::json;
    use std::sync::Mutex;

    /// The document of the open project at `path`.
    fn doc(api: &Api, path: &str) -> Document<Model> {
        api.open_session(path).expect("open").document()
    }

    /// The items of the timeline showing, in order of their starts.
    fn clips(api: &Api, path: &str) -> Vec<(ItemId, dm::Item)> {
        let document = doc(api, path);
        let mut out: Vec<(ItemId, dm::Item)> = document
            .items
            .iter()
            .map(|(id, item)| (id, item.as_ref().clone()))
            .collect();
        out.sort_by_key(|(_, item)| item.start);
        out
    }

    fn secs(t: concat_doc::Tick) -> f64 {
        t.display_seconds()
    }

    fn long(l: concat_doc::TickLen) -> f64 {
        concat_doc::TickDur::from(l).display_seconds()
    }

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
            origin: None,
            color_space: Default::default(),
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
        scratch.path().join(name).to_string_lossy().into_owned()
    }

    /// Slices gather to the asked resolution, each keeping the extremes of
    /// what it covers, and never finer than the file's own.
    #[test]
    fn peaks_gather_to_the_resolution_asked() {
        let min = [-0.1, -0.5, -0.2, -0.9, -0.3];
        let max = [0.2, 0.1, 0.8, 0.3, 0.4];
        let gathered = gather_peaks(&min, &max, 100.0, 50.0);
        assert_eq!(gathered.per_second, 50.0);
        assert_eq!(gathered.min, vec![-0.5, -0.9, -0.3]);
        assert_eq!(gathered.max, vec![0.2, 0.8, 0.4]);
        let same = gather_peaks(&min, &max, 100.0, 400.0);
        assert_eq!(same.min.len(), 5, "never finer than the file's own");
    }

    #[test]
    fn version_names_what_the_build_serves() {
        let (mut api, _scratch, _events) = api();
        let base = api.version();
        assert_eq!(base.api_version, API_VERSION);
        assert_eq!(
            base.capabilities,
            ["events", "captions", "models", "speech"].map(str::to_owned)
        );
        assert!(!base.capabilities.iter().any(|name| name == "gpu"));

        api.add_capability("json-rpc");
        api.add_capability("json-rpc");
        api.add_capability("unix-socket");
        let served = serde_json::to_value(ok(api.dispatch(Request::Version))).expect("JSON");
        assert_eq!(
            served["capabilities"],
            json!([
                "events",
                "captions",
                "models",
                "speech",
                "json-rpc",
                "unix-socket"
            ]),
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
                canvas: None,
                width: 1080,
                height: 1920,
                rate_num: 60,
                rate_den: 1,
                color_space: Default::default(),
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
        assert_eq!(clips(&api, &path).len(), 1);

        ok(api.dispatch(Request::ProjectClose {
            path: path.clone(),
            save: true,
        }));
        assert_eq!(
            err(api.dispatch(Request::ProjectGet { path: path.clone() })),
            not_open(&path)
        );

        let reopened = view(ok(api.dispatch(Request::ProjectOpen { path: path.clone() })));
        let held = clips(&api, &path);
        assert_eq!(held.len(), 1);
        assert_eq!(secs(held[0].1.start), 2.0);
        assert!(!reopened.can_undo, "history does not survive a save");

        let recents = match ok(api.dispatch(Request::ProjectList)) {
            Reply::Projects(list) => list,
            _ => panic!("not a list"),
        };
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "Round trip");
    }

    /// A clip's mask over the wire, as a caller writes it: put on through
    /// `updateClip`'s patch, its numbers keyed through `setClipKey`, and
    /// read back in the project the reply carries (#239).
    #[test]
    fn a_mask_is_set_and_keyed_through_the_api() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Masked");
        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/still.png"),
            }),
        })));
        let placed = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id: added.created_id.expect("minted"),
                start: 0.0,
            }),
        })));
        let _ = placed;
        let clip_id = clips(&api, &path)[0].0.to_string();
        let command = json!({ "op": "updateClip", "clipId": clip_id,
                "patch": { "masks": [{ "source": { "kind": "shape", "shape": "star" },
                                       "feather": 0.3, "invert": true }] } });
        let request: Request = serde_json::from_value(json!({
            "method": "edit.apply", "path": path, "command": command
        }))
        .expect("parses");
        ok(api.dispatch(request));
        let mask_id = clips(&api, &path)[0]
            .1
            .masks
            .iter()
            .find_map(dm::Entry::known)
            .expect("a mask")
            .id
            .to_string();
        for (at, value) in [(0.0, 0.2), (1.0, 1.2)] {
            let request: Request = serde_json::from_value(json!({
                "method": "edit.apply", "path": path, "command": {
                    "op": "setMaskKey", "clipId": clip_id, "maskId": mask_id,
                    "number": "width", "at": at, "value": value }
            }))
            .expect("parses");
            ok(api.dispatch(request));
        }
        let (_, clip) = clips(&api, &path).remove(0);
        let mask = clip
            .masks
            .iter()
            .find_map(dm::Entry::known)
            .expect("a mask");
        let dm::MaskSource::Shape { shape, width, .. } = &mask.source else {
            panic!("a shape");
        };
        assert_eq!(*shape, dm::MaskShape::Star);
        assert!(mask.invert && (mask.feather.value - 0.3).abs() < 1e-12);
        let width = concat_export::flatten::value_at(&clip, width, 0.5);
        assert!((width - 0.7).abs() < 1e-9, "the key rides: {width}");
    }

    /// Heard words go on the timeline as one title each, where they were
    /// said over their clip, in one undo step; a request with no words, or
    /// for a clip that is not there, is refused.
    #[test]
    fn heard_words_are_placed_as_captions_over_their_clip() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Captioned");
        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/still.png"),
            }),
        })));
        let placed = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id: added.created_id.expect("minted"),
                start: 2.0,
            }),
        })));
        let _ = placed;
        let clip_id = clips(&api, &path)[0].0.to_string();
        let request: Request = serde_json::from_value(json!({
            "method": "captions.place", "path": path, "clipId": clip_id,
            "segments": [
                { "start": 0.0, "end": 1.5, "text": " Hello there " },
                { "start": 1.5, "end": 1.6, "text": "   " },
                { "start": 2.0, "end": 3.0, "text": "again" }
            ],
            "placement": "top"
        }))
        .expect("parses");
        ok(api.dispatch(request));
        let titles: Vec<dm::Item> = clips(&api, &path)
            .into_iter()
            .map(|(_, item)| item)
            .filter(|item| matches!(item.content, dm::Content::Title(_)))
            .collect();
        assert_eq!(titles.len(), 2, "the silent stretch is left out");
        let first = titles
            .iter()
            .find(|clip| (secs(clip.start) - 2.0).abs() < 1e-9)
            .expect("the first caption where it was said");
        assert!((long(first.len) - 1.5).abs() < 1e-9);
        assert!(
            first.picture.offset_y.value < 0.0,
            "top is above the middle (offsets run down): {}",
            first.picture.offset_y.value
        );
        // One undo takes every caption away.
        ok(api.dispatch(Request::EditUndo { path: path.clone() }));
        assert!(
            clips(&api, &path)
                .iter()
                .all(|(_, clip)| !matches!(clip.content, dm::Content::Title(_)))
        );
        err(api.dispatch(Request::CaptionsPlace {
            path: path.clone(),
            clip_id: clip_id.clone(),
            segments: Vec::new(),
            placement: None,
            size: None,
        }));
        err(api.dispatch(Request::CaptionsPlace {
            path,
            clip_id: "no-such-clip".to_owned(),
            segments: vec![CaptionSegment {
                start: 0.0,
                end: 1.0,
                text: "hi".to_owned(),
            }],
            placement: None,
            size: None,
        }));
    }

    /// Freeze splits a picture and holds it in one undo step; reverse and
    /// replace refuse what they cannot do before touching anything.
    #[test]
    fn a_clip_is_frozen_and_reverse_and_replace_refuse_what_they_cannot_do() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Held");
        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/still.png"),
            }),
        })));
        let placed = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id: added.created_id.expect("minted"),
                start: 0.0,
            }),
        })));
        let _ = placed;
        let (clip_id, item) = clips(&api, &path).remove(0);
        let clip = facts(&doc(&api, &path), &clip_id.to_string()).expect("facts");
        let half = long(item.len) / 2.0;
        let request: Request = serde_json::from_value(json!({
            "method": "clips.freeze", "path": path, "clipId": clip_id.to_string(),
            "time": half, "duration": 2.0
        }))
        .expect("parses");
        ok(api.dispatch(request));
        let held = clips(&api, &path);
        assert_eq!(held.len(), 3, "the clip, the hold, the rest of the clip");
        let hold = held
            .iter()
            .find(|(_, held)| (secs(held.start) - half).abs() < 1e-9)
            .expect("a hold where the playhead was");
        assert!((long(hold.1.len) - 2.0).abs() < 1e-9);
        ok(api.dispatch(Request::EditUndo { path: path.clone() }));
        assert_eq!(clips(&api, &path).len(), 1, "one undo step");
        let clip_id = clip_id.to_string();

        let refused = err(api.dispatch(Request::ClipsReverse {
            path: path.clone(),
            clip_id: clip_id.clone(),
        }));
        assert!(refused.message.contains("nothing to reverse"), "{refused}");
        let moved = err(api.dispatch(Request::ClipsUseReversed {
            path: path.clone(),
            clip_id: clip_id.clone(),
            file: "/nowhere/reversed.mp4".to_owned(),
            source_start: clip.source_start + 1.0,
            covered: clip.duration * clip.speed,
        }));
        assert!(moved.message.contains("changed"), "{moved}");
        err(api.dispatch(Request::ClipsReplace {
            path: path.clone(),
            clip_id: clip_id.clone(),
            file: "/nowhere/else.mp4".to_owned(),
        }));
        err(api.dispatch(Request::ClipsFreeze {
            path,
            clip_id: "no-such-clip".to_owned(),
            time: 0.5,
            duration: None,
        }));
    }

    #[test]
    fn a_refusal_is_the_command_layers_sentence_with_its_code() {
        let (mut api, scratch, _) = api();
        let path = project(&mut api, &scratch, "Refused");
        let document = doc(&api, &path);
        let showing = document.view.showing.expect("a timeline");
        let track_id = document
            .lanes
            .index()
            .of(showing)
            .next()
            .expect("a lane")
            .0
            .to_string();
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
        assert!(refused.message.contains("m999"), "{}", refused.message);
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
        assert!(clips(&api, &path).is_empty());
        assert!(undone.can_redo);
        ok(api.dispatch(Request::EditRedo { path: path.clone() }));
        assert_eq!(clips(&api, &path).len(), 1);
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
        let saved = std::fs::read_to_string(concat_host::session::document_path(Path::new(&path)))
            .expect("saved");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).expect("JSON"),
            document
        );
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
        drop(events);
        assert_eq!(api.running(), 0);
        // Asked afterwards, by a caller who was not listening, the job
        // says the same.
        let status = match ok(api.dispatch(Request::ExportStatus {
            job: "j1".to_owned(),
        })) {
            Reply::Job(status) => status,
            _ => panic!("not a job's status"),
        };
        assert_eq!(status.state, JobState::Failed);
        assert_eq!(status.path, path);
        assert_eq!(
            status.error.map(|error| error.code),
            Some(ErrorCode::Failed)
        );
        let unknown = err(api.dispatch(Request::ExportStatus {
            job: "j9".to_owned(),
        }));
        assert_eq!(unknown.code, ErrorCode::NotFound);
        assert_eq!(api.stopper().cancel_all(), 0, "nothing left to stop");
    }

    #[test]
    fn an_export_that_fails_on_a_file_under_a_folder_named_cancelled_ends_as_failed() {
        let (mut api, scratch, events) = api();
        let path = project(&mut api, &scratch, "Word");
        let added = view(ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddMedia {
                item: still("/nowhere/cancelled/still.png"),
            }),
        })));
        ok(api.dispatch(Request::EditApply {
            path: path.clone(),
            command: Box::new(Command::AddClipAtFirstFree {
                media_id: added.created_id.expect("minted"),
                start: 0.0,
            }),
        }));
        ok(api.dispatch(Request::ExportRun {
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
        api.finish();
        let events = events.lock().expect("events");
        match events.last().expect("the job said how it ended") {
            Event::ExportFailed { error, .. } => {
                assert_eq!(error.code, ErrorCode::Failed, "{error}");
                assert!(
                    error.message.contains("/nowhere/cancelled/still.png"),
                    "the failure names the file, folder and all: {error}"
                );
            }
            other => panic!("ended with {other:?}"),
        }
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

    #[test]
    fn writes_stay_under_the_roots() {
        let (mut api, scratch, _events) = api();
        let allowed = scratch.path().join("allowed");
        std::fs::create_dir_all(&allowed).expect("allowed");
        api.restrict_writes_to(vec![allowed.clone()]);
        let outside = scratch
            .path()
            .join("elsewhere")
            .to_string_lossy()
            .into_owned();
        let refused = err(api.dispatch(Request::ProjectCreate {
            location: outside.clone(),
            name: "Out".to_owned(),
            video: None,
        }));
        assert_eq!(refused.code, ErrorCode::Refused);
        assert!(!scratch.path().join("elsewhere").exists(), "nothing made");
        let inside = allowed.to_string_lossy().into_owned();
        ok(api.dispatch(Request::ProjectCreate {
            location: inside.clone(),
            name: "In".to_owned(),
            video: None,
        }));
        let path = format!("{inside}/In");
        for output in [
            format!("{inside}/../elsewhere/frame.png"),
            format!("{outside}/frame.png"),
        ] {
            let refused = err(api.dispatch(Request::PreviewFrame {
                path: path.clone(),
                time: 0.0,
                output: Some(output.clone()),
                width: None,
                height: None,
            }));
            assert_eq!(refused.code, ErrorCode::Refused, "{output}");
        }
        // Under the root, into a folder not there yet, is fine.
        let written = serde_json::to_value(ok(api.dispatch(Request::PreviewFrame {
            path,
            time: 0.0,
            output: Some(format!("{inside}/frames/first.png")),
            width: Some(16),
            height: Some(9),
        })))
        .expect("JSON");
        assert!(
            written["path"]
                .as_str()
                .is_some_and(|written| written.ends_with("first.png"))
        );
    }

    #[test]
    fn a_frame_over_8k_a_side_is_invalid() {
        let (mut api, scratch, _events) = api();
        let path = project(&mut api, &scratch, "Big");
        let refused = err(api.dispatch(Request::PreviewFrame {
            path,
            time: 0.0,
            output: None,
            width: Some(65_535),
            height: Some(65_535),
        }));
        assert_eq!(refused.code, ErrorCode::Invalid);
    }

    #[test]
    fn a_project_the_window_holds_is_refused_and_the_other_way_round() {
        let (mut api, scratch, _events) = api();
        let path = project(&mut api, &scratch, "Mine");
        ok(api.dispatch(Request::ProjectClose {
            path: path.clone(),
            save: false,
        }));
        let open = OpenProjects::default();
        api.share_open_projects(open.clone());
        open.claim(&path, Holder::Window)
            .expect("the window has it");
        let refused = err(api.dispatch(Request::ProjectOpen { path: path.clone() }));
        assert_eq!(refused.code, ErrorCode::Refused);
        open.release(&path);
        ok(api.dispatch(Request::ProjectOpen { path: path.clone() }));
        assert_eq!(open.holder(&path), Some(Holder::Api));
        assert_eq!(
            open.claim(&path, Holder::Window),
            Err(Refusal::Here(Holder::Api)),
            "and the window is refused in turn"
        );
        assert!(
            Path::new(&path).join(lock::FILE).exists(),
            "and every other process by the lock"
        );
        ok(api.dispatch(Request::ProjectClose {
            path: path.clone(),
            save: false,
        }));
        assert_eq!(open.holder(&path), None, "closing gives it back");
        assert!(!Path::new(&path).join(lock::FILE).exists(), "lock and all");
    }

    /// Two registers are two processes as far as the lock goes: an API
    /// that does not share the window's register - `concat --mcp` on its
    /// own - is kept off a folder the window has, and told the way in.
    #[test]
    fn a_project_another_process_holds_is_refused_with_the_way_in() {
        let (mut other, _other_scratch, _other_events) = api();
        let (mut api, scratch, _events) = api();
        let path = project(&mut api, &scratch, "Held");
        ok(api.dispatch(Request::ProjectClose {
            path: path.clone(),
            save: false,
        }));
        let window = lock::ProjectLock::acquire(Path::new(&path), "window").expect("the window");
        let refused = err(api.dispatch(Request::ProjectOpen { path: path.clone() }));
        assert_eq!(refused.code, ErrorCode::Refused);
        assert!(
            refused.message.contains("a Concat window (process"),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("concat --mcp window"),
            "{}",
            refused.message
        );
        drop(window);
        ok(api.dispatch(Request::ProjectOpen { path: path.clone() }));

        let refused = err(other.dispatch(Request::ProjectOpen { path: path.clone() }));
        assert!(
            refused.message.contains("another Concat process"),
            "{}",
            refused.message
        );
    }
}
