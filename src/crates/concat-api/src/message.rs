// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What crosses the API boundary: a [`Request`] in, a [`Response`] out,
//! and [`Event`]s along the way.
//!
//! All three are serde types with a fixed JSON shape, and that shape is the
//! contract: a transport is free to carry it over stdin, a socket or a
//! function call, but never to reinterpret it. Requests are tagged by
//! `method`, events by `event`, and a response is `{"result": ...}` or
//! `{"error": {"code": ..., "message": ...}}`. Fields are camelCase,
//! matching the document and the command layer, so a caller learns one
//! spelling. On a line or a socket the three travel inside the JSON-RPC 2.0
//! envelope [`crate::rpc`] describes, which adds an id to tie a response to
//! its request and nothing else.
//!
//! A method that takes longer than a caller should wait - an export - is a
//! job: the response names it at once, and everything that happens to it
//! comes back as [`Event`]s carrying that name, ending in `export.done` or
//! `export.failed`. A refusal is an [`ApiError`]: a [`ErrorCode`] a program
//! can branch on and the sentence a person would be shown.
//!
//! The edit vocabulary is not redefined here. [`Request::EditApply`] carries
//! a `concat_project` [`Command`] as it is, so every operation the window
//! can perform is one an API caller can perform, with the same clamps and
//! the same refusals, and a new command needs nothing added here.

use concat_host::export::Progress;
use concat_host::media::MediaSummary;
use concat_host::session::EditorView;
use concat_host::templates::TemplateInfo;
use concat_host::{AppDirs, ProjectInfo};
use concat_project::Command;
use concat_project::model::VideoSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The version of this contract. Bumped when a method's shape changes in a
/// way a caller written against the previous one would misread; adding a
/// method or an optional field does not bump it.
pub const API_VERSION: &str = "0.2";

/// What a caller asks for. One variant per method, named `area.verb`.
///
/// Every method that touches a project names it by its folder, the same
/// path [`Request::ProjectOpen`] took; the API keeps one session per folder
/// and a method on a folder that is not open is an error, not a silent
/// open, so a caller always knows what it is editing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase")]
pub enum Request {
    /// The API's version, the build behind it, and what that build
    /// serves. The one method to call first: its reply's `capabilities`
    /// names the transports and optional features this server has, so a
    /// caller knows what to ask for before it asks.
    #[serde(rename = "version")]
    Version,

    /// Creates a project folder under `location`, named `name`, and opens
    /// it. Refuses a folder that already holds a project.
    #[serde(rename = "project.create")]
    ProjectCreate {
        /// The directory the project folder is made in.
        location: String,
        /// The project's name; the folder is named after it.
        name: String,
        /// Frame and rate; 1080p at 30 when absent.
        #[serde(default)]
        video: Option<VideoSettings>,
    },
    /// Opens a project folder. Opening one already open returns its state
    /// as it stands, edits and all.
    #[serde(rename = "project.open")]
    ProjectOpen {
        /// The project folder.
        path: String,
    },
    /// Closes an open project, saving first when asked. Unsaved edits are
    /// dropped otherwise.
    #[serde(rename = "project.close")]
    ProjectClose {
        /// The project folder.
        path: String,
        /// Whether to write the document before closing.
        #[serde(default)]
        save: bool,
    },
    /// The projects this machine opened most recently, newest first.
    #[serde(rename = "project.list")]
    ProjectList,
    /// The state of an open project.
    #[serde(rename = "project.get")]
    ProjectGet {
        /// The project folder.
        path: String,
    },
    /// The document exactly as a save would write it.
    #[serde(rename = "project.document")]
    ProjectDocument {
        /// The project folder.
        path: String,
    },
    /// Writes the document to the project folder.
    #[serde(rename = "project.save")]
    ProjectSave {
        /// The project folder.
        path: String,
        /// A new name for the project, when renaming.
        #[serde(default)]
        name: Option<String>,
    },
    /// Sets the active timeline's frame and rate, as an undoable edit.
    #[serde(rename = "project.setVideo")]
    ProjectSetVideo {
        /// The project folder.
        path: String,
        /// The frame and rate.
        video: VideoSettings,
    },

    /// Applies one edit command. The command's own notes say what each
    /// does; a refusal comes back as the sentence the window would show.
    #[serde(rename = "edit.apply")]
    EditApply {
        /// The project folder.
        path: String,
        /// The edit. Boxed because a command is the largest thing a
        /// request carries; the JSON is the command as it is.
        command: Box<Command>,
    },
    /// Steps the history back one edit.
    #[serde(rename = "edit.undo")]
    EditUndo {
        /// The project folder.
        path: String,
    },
    /// Steps the history forward one edit.
    #[serde(rename = "edit.redo")]
    EditRedo {
        /// The project folder.
        path: String,
    },

    /// Reports what is inside a media file without touching any project.
    #[serde(rename = "media.probe")]
    MediaProbe {
        /// The file.
        path: String,
    },
    /// Probes a file and adds it to a project's bin: what dropping a file
    /// on the window does. A path already in the bin is a no-op.
    #[serde(rename = "media.import")]
    MediaImport {
        /// The project folder.
        path: String,
        /// The file to import.
        file: String,
    },

    /// Every effect package the build knows, with its parameters, so a
    /// caller can build a valid chain without reading a manifest.
    #[serde(rename = "catalogue.list")]
    CatalogueList {
        /// Restrict to one kind: "effect", "filter", "audio", "transition"
        /// or "generator".
        #[serde(default)]
        kind: Option<String>,
    },

    /// The template library.
    #[serde(rename = "template.list")]
    TemplateList,
    /// Makes a project from a template with every slot filled, and opens
    /// it. Every slot must be filled; a set that leaves one empty makes
    /// nothing.
    #[serde(rename = "template.instantiate")]
    TemplateInstantiate {
        /// The template bundle folder.
        template: String,
        /// The directory the project folder is made in.
        location: String,
        /// The project's name.
        name: String,
        /// The file for each slot.
        fills: Vec<Fill>,
    },
    /// Packs an open project into a new template bundle.
    #[serde(rename = "template.save")]
    TemplateSave {
        /// The project folder.
        path: String,
        /// The template's name.
        name: String,
    },

    /// Renders an open project's active timeline to a file, exactly as
    /// the window's Export does: cutouts analysed, titles painted, then the
    /// frame loop and the mix. Returns at once with the job's name; the
    /// render runs on its own thread and reports through [`Event`]s, ending
    /// in [`Event::ExportDone`] or [`Event::ExportFailed`]. One export runs
    /// at a time; a second is refused as [`ErrorCode::Busy`].
    #[serde(rename = "export.run")]
    ExportRun {
        /// The project folder.
        path: String,
        /// How to render it.
        #[serde(flatten)]
        spec: ExportSpec,
    },
    /// Stops a running export at its next frame. The job then ends with
    /// [`Event::ExportFailed`] carrying [`ErrorCode::Cancelled`].
    #[serde(rename = "export.cancel")]
    ExportCancel {
        /// The job [`Started`] named.
        job: String,
    },

    /// Composites the true frame at one instant. Written as a PNG to
    /// `output` when there is one, and handed back inline as a
    /// [`Picture`] otherwise, for a caller on the far side of a socket.
    #[serde(rename = "preview.frame")]
    PreviewFrame {
        /// The project folder.
        path: String,
        /// The timeline instant, in seconds.
        time: f64,
        /// The file to write; absent means the picture comes back inline.
        #[serde(default)]
        output: Option<String>,
        /// Frame width; the timeline's when absent.
        #[serde(default)]
        width: Option<u32>,
        /// Frame height; the timeline's when absent.
        #[serde(default)]
        height: Option<u32>,
    },
}

/// One template slot and the file that takes it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    /// The placeholder's media id in the template.
    pub media_id: String,
    /// The file to put there; probed on the way in.
    pub file: String,
}

/// How an export is rendered. Everything but the output is optional and
/// defaults to what the window's sheet defaults to.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSpec {
    /// The file to write.
    pub output: String,
    /// Constant rate factor; lower is better and bigger. 20 when absent.
    #[serde(default)]
    pub crf: Option<u8>,
    /// The x264 preset name. "medium" when absent.
    #[serde(default)]
    pub preset: Option<String>,
    /// Output width; the timeline's when absent.
    #[serde(default)]
    pub width: Option<u32>,
    /// Output height; the timeline's when absent.
    #[serde(default)]
    pub height: Option<u32>,
    /// Frame rate numerator; the timeline's when absent.
    #[serde(default)]
    pub rate_num: Option<i64>,
    /// Frame rate denominator; the timeline's when absent.
    #[serde(default)]
    pub rate_den: Option<i64>,
    /// "h264", "hevc" or "av1". H.264 when absent.
    #[serde(default)]
    pub codec: Option<String>,
    /// Ten bits a channel. Eight when absent.
    #[serde(default)]
    pub ten_bit: Option<bool>,
}

/// What a request hands back. Serialised as the payload alone: the variant
/// is implied by the method, so a caller that sent `project.get` reads an
/// editor view and nothing wraps it.
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum Reply {
    /// [`Request::Version`].
    Version(VersionInfo),
    /// The project after a change, or as it stands.
    View(Box<EditorView>),
    /// [`Request::ProjectList`].
    Projects(Vec<ProjectInfo>),
    /// [`Request::ProjectDocument`].
    Document(Value),
    /// [`Request::MediaProbe`].
    Media(MediaSummary),
    /// [`Request::CatalogueList`].
    Packages(Vec<PackageInfo>),
    /// [`Request::TemplateList`].
    Templates(Vec<TemplateInfo>),
    /// [`Request::TemplateSave`].
    Template(TemplateInfo),
    /// [`Request::ExportRun`]: the job that is now running.
    Started(Started),
    /// [`Request::PreviewFrame`] with an output: the file written.
    Written(Written),
    /// [`Request::PreviewFrame`] without one: the picture itself.
    Picture(Picture),
    /// Methods with nothing to say beyond having worked.
    Done(Done),
}

/// The build and the contract it speaks.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// [`API_VERSION`].
    pub api_version: String,
    /// The Concat build.
    pub concat: String,
    /// Where this machine keeps recents, templates and models.
    pub dirs: Dirs,
    /// What this build serves, as names a caller tests for membership:
    /// what the API itself provides in every build, `events`, and what
    /// only some builds do, `gpu` when frames composite on one; then what
    /// the transport around it adds, `json-rpc`, `unix-socket` and `grpc`
    /// for each of the socket transports listening. A name absent from
    /// the list is not served, whatever the build; a name added later is
    /// not a version bump, so a caller ignores names it does not know.
    pub capabilities: Vec<String>,
}

/// The app's directories, as paths.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dirs {
    /// Small state: recents, settings, the template library.
    pub config: String,
    /// Large state: downloaded models, painted titles.
    pub data: String,
}

impl From<&AppDirs> for Dirs {
    fn from(dirs: &AppDirs) -> Self {
        Dirs {
            config: dirs.config.to_string_lossy().into_owned(),
            data: dirs.data.to_string_lossy().into_owned(),
        }
    }
}

/// A file the API wrote.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    /// The file's path.
    pub path: String,
    /// Its picture size.
    pub width: u32,
    /// Its picture size.
    pub height: u32,
}

/// A job the API has begun. Every event about it carries `job`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    /// The job's name, unique for the life of the API: "j1", "j2", ...
    pub job: String,
    /// The project it works on.
    pub path: String,
    /// The file it will write.
    pub output: String,
}

/// A frame handed back without touching the disk.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Picture {
    /// Its picture size.
    pub width: u32,
    /// Its picture size.
    pub height: u32,
    /// The PNG, base64 in the standard alphabet with padding.
    pub png: String,
}

/// The empty reply, an object so every reply is one.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Done {}

/// One effect package, as a caller building a chain needs it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    /// The id a clip's chain stores, `author.name`.
    pub id: String,
    /// What the catalogue card says.
    pub name: String,
    /// "effect", "filter", "audio", "transition" or "generator".
    pub kind: String,
    /// The shelf the card sits on.
    pub category: String,
    /// A sentence about it.
    pub description: String,
    /// The parameter the simple view shows as the one slider, if any.
    pub intensity: Option<String>,
    /// The knobs, in the order the inspector shows them.
    pub params: Vec<ParamInfo>,
}

/// One parameter of a package.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    /// The key a chain entry's `params` stores it under.
    pub key: String,
    /// What the control is labelled.
    pub label: String,
    /// "float", "int", "bool", "enum", "color" or "point".
    #[serde(rename = "type")]
    pub kind: String,
    /// Lowest value.
    pub min: f64,
    /// Highest value.
    pub max: f64,
    /// The value an untouched control means.
    pub default: f64,
    /// Slider increment; 0 means continuous.
    pub step: f64,
    /// Displayed after the number.
    pub unit: String,
    /// Whether the control can carry keyframes.
    pub animate: bool,
    /// For "enum": the values the document may hold.
    pub values: Vec<f64>,
    /// For "enum": what each value is called, in `values` order.
    pub labels: Vec<String>,
}

/// Why a request has no reply, as a program reads it. The JSON-RPC number
/// each maps to is [`ErrorCode::number`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The line was not JSON.
    Parse,
    /// The JSON was not a request: an unknown method, a missing or
    /// ill-typed field, a value out of range.
    Invalid,
    /// The project named is not open; open it first.
    NotOpen,
    /// A job, template or package named does not exist.
    NotFound,
    /// The edit layer said no. The message is the sentence the window
    /// would show, and the state is as it was.
    Refused,
    /// A one-at-a-time job is already running.
    Busy,
    /// The job was stopped by [`Request::ExportCancel`].
    Cancelled,
    /// The transport wanted a token and did not get the right one. Never
    /// raised by the API itself.
    Unauthorized,
    /// Everything else: a file that could not be read or written, a decode
    /// that failed, a model that did not download.
    Failed,
}

impl ErrorCode {
    /// The code's JSON-RPC 2.0 number: the standard ones for the standard
    /// meanings, and server-defined ones (-32000 downwards) for the rest.
    pub fn number(self) -> i64 {
        match self {
            ErrorCode::Parse => -32700,
            ErrorCode::Invalid => -32600,
            ErrorCode::Failed => -32000,
            ErrorCode::NotOpen => -32001,
            ErrorCode::NotFound => -32002,
            ErrorCode::Refused => -32003,
            ErrorCode::Busy => -32004,
            ErrorCode::Cancelled => -32005,
            ErrorCode::Unauthorized => -32006,
        }
    }
}

/// A refusal: what kind, and the sentence a person would be shown.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// What kind of refusal.
    pub code: ErrorCode,
    /// The sentence.
    pub message: String,
}

impl ApiError {
    /// An error of `code` saying `message`.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> ApiError {
        ApiError {
            code,
            message: message.into(),
        }
    }

    /// [`ErrorCode::Invalid`].
    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Invalid, message)
    }

    /// [`ErrorCode::Failed`].
    pub fn failed(message: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Failed, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// A request's outcome. `{"result": ...}` when it worked, `{"error":
/// {"code": ..., "message": ...}}` when it did not.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)]
pub enum Response {
    /// The reply.
    Result(Reply),
    /// Why there is none.
    Error(ApiError),
}

impl From<Result<Reply, ApiError>> for Response {
    fn from(result: Result<Reply, ApiError>) -> Self {
        match result {
            Ok(reply) => Response::Result(reply),
            Err(error) => Response::Error(error),
        }
    }
}

/// Something that happened to a job. A transport forwards these to every
/// caller as they come; each names its job and its project, so a caller
/// keeps the ones it asked for and ignores the rest.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all_fields = "camelCase")]
pub enum Event {
    /// A cutout's masks are being found ahead of an export.
    #[serde(rename = "cutout.progress")]
    CutoutProgress {
        /// The job.
        job: String,
        /// The project folder.
        path: String,
        /// The media being analysed.
        media_id: String,
        /// True while a model downloads, false while it runs.
        fetching: bool,
        /// How far along, `0..=1`.
        fraction: f32,
    },
    /// The export moved on.
    #[serde(rename = "export.progress")]
    ExportProgress {
        /// The job.
        job: String,
        /// The project folder.
        path: String,
        /// Frames done.
        frame: i64,
        /// Frames in total.
        total: i64,
        /// "video", "audio" or "mux".
        stage: String,
    },
    /// The export wrote its file. The job is over.
    #[serde(rename = "export.done")]
    ExportDone {
        /// The job.
        job: String,
        /// The project folder.
        path: String,
        /// The file written.
        output: String,
        /// Its picture size.
        width: u32,
        /// Its picture size.
        height: u32,
    },
    /// The export did not write its file. The job is over.
    #[serde(rename = "export.failed")]
    ExportFailed {
        /// The job.
        job: String,
        /// The project folder.
        path: String,
        /// Why: [`ErrorCode::Cancelled`] when asked to stop, otherwise what
        /// went wrong.
        error: ApiError,
    },
}

impl Event {
    /// [`Event::ExportProgress`] for one report from the renderer.
    pub fn progress(job: &str, path: &str, progress: Progress) -> Event {
        Event::ExportProgress {
            job: job.to_owned(),
            path: path.to_owned(),
            frame: progress.frame,
            total: progress.total,
            stage: progress.stage.to_owned(),
        }
    }

    /// The job this event is about.
    pub fn job(&self) -> &str {
        match self {
            Event::CutoutProgress { job, .. }
            | Event::ExportProgress { job, .. }
            | Event::ExportDone { job, .. }
            | Event::ExportFailed { job, .. } => job,
        }
    }
}
