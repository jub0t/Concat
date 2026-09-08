// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What crosses the API boundary: a [`Request`] in, a [`Response`] out,
//! and [`Event`]s along the way.
//!
//! All three are serde types with a fixed JSON shape, and that shape is the
//! contract: a transport is free to carry it over stdin, a socket or a
//! function call, but never to reinterpret it. Requests are tagged by
//! `method`, events by `event`, and a response is `{"result": ...}` or
//! `{"error": "..."}`. Fields are camelCase, matching the document and the
//! command layer, so a caller learns one spelling.
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
pub const API_VERSION: &str = "0.1";

/// What a caller asks for. One variant per method, named `area.verb`.
///
/// Every method that touches a project names it by its folder, the same
/// path [`Request::ProjectOpen`] took; the API keeps one session per folder
/// and a method on a folder that is not open is an error, not a silent
/// open, so a caller always knows what it is editing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase")]
pub enum Request {
    /// The API's version and the build behind it.
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
    /// the window's Export does: titles painted, cutouts analysed, then the
    /// frame loop and the mix. Blocks until the file is written, reporting
    /// through [`Event`]s.
    #[serde(rename = "export.run")]
    ExportRun {
        /// The project folder.
        path: String,
        /// How to render it.
        #[serde(flatten)]
        spec: ExportSpec,
    },
    /// Stops the export that is running, if one is. Meaningful only from a
    /// transport that can speak while a dispatch blocks.
    #[serde(rename = "export.cancel")]
    ExportCancel,

    /// Composites the true frame at one instant and writes it as a PNG.
    #[serde(rename = "preview.frame")]
    PreviewFrame {
        /// The project folder.
        path: String,
        /// The timeline instant, in seconds.
        time: f64,
        /// The file to write.
        output: String,
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
    /// [`Request::ExportRun`] and [`Request::PreviewFrame`]: the file
    /// written.
    Written(Written),
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

/// A request's outcome. `{"result": ...}` when it worked, `{"error":
/// "..."}` with the sentence a person would be shown when it did not.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Response {
    /// The reply.
    Result(Reply),
    /// Why there is none.
    Error(String),
}

impl From<Result<Reply, String>> for Response {
    fn from(result: Result<Reply, String>) -> Self {
        match result {
            Ok(reply) => Response::Result(reply),
            Err(error) => Response::Error(error),
        }
    }
}

/// Something that happened while a request ran. A transport forwards
/// these as they come; a caller that does not care ignores them.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all_fields = "camelCase")]
pub enum Event {
    /// The export moved on.
    #[serde(rename = "export.progress")]
    ExportProgress {
        /// Frames done.
        frame: i64,
        /// Frames in total.
        total: i64,
        /// "rendering", "mixing audio" or "muxing".
        stage: String,
    },
    /// A cutout's masks are being found ahead of an export.
    #[serde(rename = "cutout.progress")]
    CutoutProgress {
        /// The media being analysed.
        media_id: String,
        /// True while a model downloads, false while it runs.
        fetching: bool,
        /// How far along, `0..=1`.
        fraction: f32,
    },
}

impl From<Progress> for Event {
    fn from(progress: Progress) -> Self {
        Event::ExportProgress {
            frame: progress.frame,
            total: progress.total,
            stage: progress.stage.to_owned(),
        }
    }
}
