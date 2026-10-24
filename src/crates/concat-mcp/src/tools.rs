// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The tools: which API method or edit command each one is, and what an
//! agent is told about it.
//!
//! Every tool is a [`concat_api::Request`] in the end. Most are one method
//! or one edit command as it is, their arguments that request's fields and
//! their schema cut from the API's ([`crate::schema`]); a handful compose,
//! as captions are a batch of titles and waiting on an export is asking its
//! status until it ends, and say so. None decides what an edit means.
//!
//! Names are stable: an agent's instructions and a user's allow-list name
//! them. A tool is added, never renamed; a removed one would be a version
//! bump of this server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use concat_api::{ApiError, ErrorCode, Request};
use serde_json::{Map, Value, json};

use crate::backend::Backend;
use crate::schema::Schemas;
use crate::summary;

/// The longest preview side handed back when the agent names no size: a
/// frame an agent can look at without a 4K PNG in its context.
const PREVIEW_SIDE: f64 = 1280.0;
/// How often a wait asks how an export stands.
const POLL: Duration = Duration::from_millis(250);
/// How long `export_wait` waits when told nothing, and at most.
const WAIT_DEFAULT: f64 = 60.0;
const WAIT_LONGEST: f64 = 600.0;
/// How many stages an export has, for one progress figure across them.
const STAGES: usize = 5;

/// Where a stage comes in an export, from 0. A job's status names the
/// stage the exporter reports - "rendering" (or "rendering in software",
/// going again after the hardware encoder stopped), "mixing audio",
/// "muxing" - and the API's documentation names them "video", "audio" and "mux"; both
/// are known here, and one this does not know counts as the first.
fn stage_place(stage: &str) -> usize {
    match stage {
        "cutout" => 0,
        "stabilize" => 1,
        "video" | "rendering" | "rendering in software" => 2,
        "audio" | "mixing audio" => 3,
        "mux" | "muxing" => 4,
        _ => 0,
    }
}

/// What a tool does with its arguments.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    /// An API method, its arguments the method's params.
    Method {
        /// The method's name, `area.verb`.
        method: &'static str,
        /// What the reply is shown as.
        reply: Reply,
    },
    /// One edit command on a project: `edit.apply` with the command's
    /// fields as arguments, beside the project's `path`.
    Command {
        /// The command's `op`.
        op: &'static str,
    },
    /// `version`, with how this server reaches Concat added.
    Version,
    /// `project.get`, summarised.
    TimelineRead,
    /// Titles at the given times, as one batch.
    Captions,
    /// `export.status` until the job ends or the time is up.
    ExportWait,
    /// `updateClip` with only a mask.
    Mask,
    /// `preview.frame`, as an image.
    PreviewFrame,
}

/// How a reply is shown to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// As the API sent it.
    Raw,
    /// An editor view, as [`summary::view`].
    Summary,
}

/// One tool.
#[derive(Clone, Debug)]
pub struct Tool {
    /// The name an agent calls it by.
    pub name: &'static str,
    /// The name a person is shown.
    pub title: &'static str,
    /// What the agent is told: what it does, its units, its ranges.
    pub description: &'static str,
    /// Changes nothing.
    pub read_only: bool,
    /// May remove or overwrite something.
    pub destructive: bool,
    /// Calling it twice is calling it once.
    pub idempotent: bool,
    /// What it does.
    pub action: Action,
}

const fn method(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    method: &'static str,
    reply: Reply,
) -> Tool {
    Tool {
        name,
        title,
        description,
        read_only: false,
        destructive: false,
        idempotent: false,
        action: Action::Method { method, reply },
    }
}

const fn command(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    op: &'static str,
) -> Tool {
    Tool {
        name,
        title,
        description,
        read_only: false,
        destructive: false,
        idempotent: false,
        action: Action::Command { op },
    }
}

impl Tool {
    const fn reading(mut self) -> Tool {
        self.read_only = true;
        self.idempotent = true;
        self
    }

    const fn destroying(mut self) -> Tool {
        self.destructive = true;
        self
    }

    const fn idempotent(mut self) -> Tool {
        self.idempotent = true;
        self
    }
}

/// Every tool, in the order `tools/list` gives them.
pub fn catalogue() -> Vec<Tool> {
    use Reply::{Raw, Summary};
    vec![
        Tool {
            name: "concat_version",
            title: "Concat version",
            description: "What this Concat speaks: the API version, the build, its capabilities, and how this MCP server reaches it (`mcp.mode`: \"window\" when it goes through the open Concat window's server, \"headless\" when it runs the editor engine itself). Call it first if unsure what is available.",
            read_only: true,
            destructive: false,
            idempotent: true,
            action: Action::Version,
        },
        method(
            "project_list",
            "Recent projects",
            "The projects this machine opened most recently, newest first: each one's folder `path`, name, frame size and rate. A project is a folder; every other project tool names it by that `path`.",
            "project.list",
            Raw,
        )
        .reading(),
        method(
            "project_create",
            "Create a project",
            "Creates a project folder named `name` inside the existing directory `location`, and opens it. `video` sets the frame and rate (1920×1080 at 30 fps when absent). Returns the project summary; its folder is `location/name`, which is the `path` later tools take. Refuses a folder that already holds a project.",
            "project.create",
            Summary,
        ),
        method(
            "project_open",
            "Open a project",
            "Opens a project folder so it can be read and edited. Opening one already open returns it as it stands, unsaved edits included. Refused when the Concat window has that project open. Returns the project summary.",
            "project.open",
            Summary,
        )
        .idempotent(),
        method(
            "project_save",
            "Save a project",
            "Writes an open project's document to its folder; edits are only in memory until this is called. `name` renames the project as it saves.",
            "project.save",
            Raw,
        )
        .idempotent(),
        method(
            "project_close",
            "Close a project",
            "Closes an open project. With `save: true` it is saved first; otherwise unsaved edits are dropped.",
            "project.close",
            Raw,
        )
        .destroying(),
        Tool {
            name: "timeline_read",
            title: "Read the timeline",
            description: "An open project's active timeline as a compact summary: frame size and fps, the lanes (`tracks`, top to bottom, with their ids), every clip in time order with its id, kind, track, start, duration and end in seconds and whatever is not at its default (speed, volume, opacity, transform, effects, keyframed properties, text), and the media bin with each item's id. Read this to learn the ids other tools need.",
            read_only: true,
            destructive: false,
            idempotent: true,
            action: Action::TimelineRead,
        },
        method(
            "project_get",
            "Get the whole project",
            "An open project's full state: every timeline, clip field, media item and setting, as the editor holds it. Large; `timeline_read` is usually enough.",
            "project.get",
            Raw,
        )
        .reading(),
        method(
            "project_set_video",
            "Set frame size and rate",
            "Sets the active timeline's frame (width, height in pixels) and rate (rateNum/rateDen frames a second, e.g. 30000/1001 for 29.97) as one undoable edit.",
            "project.setVideo",
            Summary,
        ),
        method(
            "media_probe",
            "Probe a media file",
            "Reports what is inside a video, audio or image file - duration in seconds, streams, frame size and rate, codecs - without changing any project.",
            "media.probe",
            Raw,
        )
        .reading(),
        method(
            "media_import",
            "Import media",
            "Adds a file (an absolute path) to an open project's media bin, as dropping it on the window does. The new item's id is the summary's `createdId`; place it on the timeline with `clip_add`. Importing a path already in the bin changes nothing.",
            "media.import",
            Summary,
        )
        .idempotent(),
        method(
            "edit_apply",
            "Apply an edit",
            "Applies any one edit command to an open project, as one undoable step. `command` is an object tagged by `op` - every edit the window can make is one, with the same limits and refusals; the typed tools below are the common ones. `{\"op\": \"batch\", \"commands\": [...]}` applies several commands as one atomic, undoable step: all land or none does, and `createdId` is the last id minted inside. A refusal comes back as the sentence the window would show, with nothing changed. Returns the project summary; a command that creates something puts its id in `createdId`.",
            "edit.apply",
            Summary,
        ),
        method(
            "edit_undo",
            "Undo",
            "Steps an open project's history back one edit (a batch is one edit).",
            "edit.undo",
            Summary,
        ),
        method(
            "edit_redo",
            "Redo",
            "Steps an open project's history forward one edit.",
            "edit.redo",
            Summary,
        ),
        command(
            "clip_add",
            "Place a clip",
            "Places a clip of a media item (`mediaId`, from the bin) on a lane (`trackId`, from `timeline_read`) starting at `start` seconds. Its length is the media's (5 s for a still). `ripple: true` pushes later clips on that lane right instead of overlapping them. The new clip's id is `createdId`.",
            "addClip",
        ),
        command(
            "clips_add_sequence",
            "Place clips end to end",
            "Lays several media items (`mediaIds`, from the bin, in order) end to end starting at `start` seconds, each its media's length (5 s for a still), in one undoable step. With no `trackId` they go on the lowest lane free for the whole run; on a named lane, `ripple: true` makes room as a drop does. `createdId` is the last clip's id; `timeline_read` lists the rest.",
            "addClipsInSequence",
        ),
        command(
            "clip_trim",
            "Trim a clip",
            "Moves one edge of a clip by `delta` seconds of timeline: `edge` \"start\" with a positive delta shortens it from the head (the picture does not slide), \"end\" with a positive delta lengthens it. A clip is never shorter than 1/60 s or longer than its media allows. `ripple: true` closes or opens the gap behind it on that lane.",
            "trimClip",
        ),
        command(
            "clips_split",
            "Split clips",
            "Cuts each named clip in two at `time` seconds on the timeline. The first part keeps the id; the second gets a new one. Clips the time does not fall inside are left alone.",
            "splitClips",
        ),
        command(
            "clips_move",
            "Move clips",
            "Moves clips in one undoable step: each to a new `start` (seconds, at least 0) and `trackId`. Unknown ids are skipped.",
            "moveClips",
        ),
        command(
            "clips_remove",
            "Delete clips",
            "Deletes clips from the active timeline. `ripple: true` closes the gaps they leave on their lanes; false leaves the gaps.",
            "removeClips",
        )
        .destroying(),
        command(
            "clip_update",
            "Change a clip",
            "Changes only the fields given in `patch`: volume (gain, 1 is unchanged, may exceed 1), fadeIn/fadeOut (seconds), opacity (0-1), muted, flipH/flipV, blend, crop (fractions off each edge, null to remove), videoEffects and filters (the whole chain: a list of {id, params}; ids from `effects_list`), transitionIn ({id, duration} on the cut into the clip, null to remove), text (a title's style, null to remove), mask (see `clip_mask`), background (the clip's own background, null to follow the timeline's) and audioStream.",
            "updateClip",
        ),
        Tool {
            name: "clip_mask",
            title: "Mask a clip",
            description: "Keeps part of a clip's picture with a shape, or with `mask: null` takes the mask off. `shape` is linear, mirror, circle, rectangle, heart or star; `x`/`y` move its centre as fractions of the picture's width/height (positive right/down); `width`/`height` are fractions of the picture's shorter side; `rotation` in degrees; `roundness` (rectangle corners) and `feather` (edge softness) 0-1; `invert` keeps the outside. Fields left out take the default circle's. Animate it with `keyframe_set` on maskX, maskY, maskWidth, maskHeight, maskRotation, maskRoundness or maskFeather.",
            read_only: false,
            destructive: false,
            idempotent: true,
            action: Action::Mask,
        },
        command(
            "clip_stabilize",
            "Stabilize a clip",
            "Takes the camera's shake out of a video clip, or with `stabilize: null` stops. `smoothness` 0-1 (low keeps pans and removes jitter, high leaves only the slowest moves), `maxZoom` (at least 1) is how far the picture may be enlarged to hide the moving edges, `tripod: true` locks the frame entirely. The shake is measured when the clip is previewed or exported.",
            "setClipStabilize",
        ),
        command(
            "clip_frame",
            "Fit or fill the frame",
            "Sizes a picture to the frame in one step: `framing` \"fit\" shows all of it (bars where the shapes differ), \"fill\" covers the frame and crops the overhang. Where the clip's scale is keyframed, the key goes at `at` (a fraction of the clip, 0-1).",
            "frameClip",
        ),
        command(
            "timeline_background",
            "Set the background",
            "Sets what shows behind pictures that do not cover the frame, on a whole timeline (`timelineId` from `timeline_read`): `{\"kind\": \"none\"}` (black), `{\"kind\": \"blur\", \"amount\": 0-1}` (the clip itself, enlarged and blurred), `{\"kind\": \"color\", \"color\": \"#rrggbb\"}` or `{\"kind\": \"image\", \"mediaId\": ...}`. One clip's own background is `clip_update`'s `background`.",
            "setTimelineBackground",
        ),
        command(
            "clip_transform",
            "Place, scale or rotate a clip",
            "Sets a picture's placement; only the fields given change. scale 0.05-8 (1 fits the frame), offsetX/offsetY as fractions of the frame's width/height from the centre (positive is right/down, within ±3), rotation in degrees, stretchX/stretchY 0.1-10.",
            "setClipTransform",
        ),
        command(
            "clip_speed",
            "Change a clip's speed",
            "Plays a clip faster or slower: `speed` from 0.0625 to 16 (2 is double speed). The same stretch of source is kept, so the clip's length on the timeline changes.",
            "setClipSpeed",
        ),
        command(
            "title_add",
            "Add a title",
            "Places a title (text over the picture) at `start` seconds for `duration` seconds (4 s when absent). `style` sets the words and look - `content`, `fontFamily`, `fontSize` (fraction of frame height, 0.09 by default), `fontWeight` (100-900), `color` (\"#rrggbb\"), `align`, `background`, `strokeWidth`... - and needs only the fields that differ from the default. `offsetY` places it vertically as a fraction of frame height from the centre (positive is down). With no `trackId` and `above: true` it lands on a lane above the picture. The new clip's id is `createdId`.",
            "addTextClip",
        ),
        Tool {
            name: "captions_add",
            title: "Add captions",
            description: "Places a run of captions in one undoable step: one title per entry, from `start` to `end` seconds, each on a lane above the picture. `style` applies to all of them (its `content` is ignored); without one they look like the window's captions - Hanken Grotesk, weight 600, 0.04 of the frame high. `offsetY` is where they sit, as a fraction of frame height from the centre: 0.35 (the default) is near the bottom, 0 the middle, -0.35 the top.",
            read_only: false,
            destructive: false,
            idempotent: false,
            action: Action::Captions,
        },
        command(
            "keyframe_set",
            "Set a keyframe",
            "Animates a clip: puts a key on `property` (scale, offsetX, offsetY, rotation, opacity, volume, or a mask's maskX, maskY, maskWidth, maskHeight, maskRotation, maskRoundness, maskFeather) at `at`, a fraction of the clip's length from 0 (its start) to 1 (its end), holding `value` in the property's own units - the number the inspector shows. A key already within a hair of `at` is replaced. `ease` is how the value is approached: [x1, y1, x2, y2] like a CSS cubic-bezier, linear when absent.",
            "setClipKey",
        ),
        command(
            "keyframe_clear",
            "Remove a keyframe",
            "Removes the key on `property` at `at` (a fraction of the clip's length), if there is one.",
            "clearClipKey",
        )
        .destroying(),
        command(
            "effect_layer_add",
            "Add an effect layer",
            "Places an effect or look as a layer over the timeline: from `start` for `duration` seconds (5 s when absent), it treats everything beneath it. `effectId` is a package id from `effects_list`; the layer's opacity (via `clip_update`) is how strongly it applies. To put an effect on one clip instead, set that clip's `videoEffects` with `clip_update`. With `sound: true` and an \"audio\" package it is a sound layer (kind \"soundLayer\"): its chain, in the clip's `filters`, treats the mix of everything audible for its span, its opacity the wet/dry and its fades the ramps.",
            "addLayerClip",
        ),
        command(
            "effect_key_set",
            "Animate an effect parameter",
            "Puts a key on one parameter (`key`, from `effects_list`) of one effect in a clip's chain (`entry`, its index in the clip's videoEffects, from 0) at `at` (a fraction of the clip's length, 0-1), holding `value` in the parameter's own units. Keep the value within the range `effects_list` gives.",
            "setEffectKey",
        ),
        command(
            "track_add",
            "Add a lane",
            "Adds a lane to the active timeline: on top, or with `bottom: true` under every other. Its id is `createdId`.",
            "addTrack",
        ),
        method(
            "effects_list",
            "List effects",
            "Every effect package this build has - id, name, kind (\"effect\", \"filter\", \"audio\", \"transition\" or \"generator\"), category and each parameter's key, range, default and unit - so an effect chain, a layer or a transition can be built correctly. `kind` narrows the list.",
            "catalogue.list",
            Raw,
        )
        .reading(),
        method(
            "template_list",
            "List templates",
            "The template library: each template's folder, name and the slots (placeholder media ids) a project made from it must fill.",
            "template.list",
            Raw,
        )
        .reading(),
        method(
            "template_instantiate",
            "Make a project from a template",
            "Makes a project from a template - `template` (its folder), `location` and `name` as for `project_create` - with every slot filled by a file (`fills`: [{mediaId, file}]). Opens it and returns its summary.",
            "template.instantiate",
            Summary,
        ),
        method(
            "export_start",
            "Start an export",
            "Starts rendering an open project's active timeline to `output` (an .mp4 path), as the window's Export does. Returns at once with a `job` id; follow it with `export_wait` or `export_status`. Optional: crf (quality, lower is better, 20 by default), preset (x264 speed, \"medium\"), width/height, rateNum/rateDen, codec (\"h264\", \"hevc\" or \"av1\"), tenBit, colorRange, hdr. One export runs at a time; another is refused as busy.",
            "export.run",
            Raw,
        ),
        method(
            "export_status",
            "Export status",
            "How an export job stands, at once: `state` (\"running\", \"done\", \"cancelled\" or \"failed\"), `stage` (\"cutout\", \"stabilize\", \"rendering\", \"mixing audio\" or \"muxing\"), frames done of total, `fraction` of the stage (0-1), and once done the file and its size, or the error.",
            "export.status",
            Raw,
        )
        .reading(),
        Tool {
            name: "export_wait",
            title: "Wait for an export",
            description: "Waits for an export job to end, up to `timeoutSeconds` (60 by default, 600 at most), reporting progress as it goes, and returns its status as `export_status` does. If it is still running when the time is up, call it again.",
            read_only: true,
            destructive: false,
            idempotent: true,
            action: Action::ExportWait,
        },
        method(
            "export_cancel",
            "Cancel an export",
            "Stops a running export job at its next frame; it then ends as \"cancelled\".",
            "export.cancel",
            Raw,
        )
        .destroying(),
        Tool {
            name: "preview_frame",
            title: "Preview a frame",
            description: "Renders the true composited frame at `time` seconds on an open project's timeline and returns it as an image to look at - titles, effects and transforms included. Without `width` and `height` it fits within 1280 pixels; give both for another size (8192 at most). With `output` (a .png path) it is written there instead, replacing any file already at that path.",
            // Not read-only: with `output` it writes a file, over one that
            // is there. A client that trusts the hint to skip asking would
            // let an agent overwrite anything under home.
            read_only: false,
            destructive: true,
            idempotent: true,
            action: Action::PreviewFrame,
        },
    ]
}

/// What a tool call came to: MCP content, and whether it is an error the
/// agent should read and correct.
pub struct Outcome {
    /// The content blocks.
    pub content: Vec<Value>,
    /// The same result as JSON, for a client that reads it structured.
    pub structured: Option<Value>,
    /// A tool execution error.
    pub is_error: bool,
}

impl Outcome {
    fn json(value: Value) -> Outcome {
        let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
        Outcome {
            content: vec![json!({ "type": "text", "text": text })],
            structured: value.is_object().then_some(value),
            is_error: false,
        }
    }

    fn error(error: &ApiError) -> Outcome {
        let code = serde_json::to_value(error.code)
            .ok()
            .and_then(|code| code.as_str().map(str::to_owned))
            .unwrap_or_default();
        Outcome {
            content: vec![
                json!({ "type": "text", "text": format!("{} (code: {code})", error.message) }),
            ],
            structured: Some(json!({ "error": { "code": code, "message": error.message } })),
            is_error: true,
        }
    }
}

/// What a call may use besides its arguments.
pub struct Context<'a> {
    /// Where requests go.
    pub backend: &'a mut dyn Backend,
    /// Set when the client cancels the call.
    pub cancelled: &'a AtomicBool,
    /// Reports progress, when the client asked for it: done so far, out
    /// of a total, and a sentence.
    pub progress: Option<&'a dyn Fn(f64, f64, String)>,
}

/// The tools with their schemas, made once.
pub struct Toolbox {
    tools: Vec<(Tool, Value)>,
    schemas: Schemas,
}

impl Toolbox {
    /// Every tool in [`catalogue`], with the schema the API gives it.
    /// Panics on a tool whose method or command the API does not have: a
    /// test builds this, so the panic is a failed build, not a user's.
    pub fn new() -> Toolbox {
        let schemas = Schemas::generate();
        let tools = catalogue()
            .into_iter()
            .map(|tool| {
                let schema = input_schema(&schemas, &tool)
                    .unwrap_or_else(|| panic!("the API has nothing for the {} tool", tool.name));
                (tool, schema)
            })
            .collect();
        Toolbox { tools, schemas }
    }

    /// `tools/list`'s list.
    pub fn list(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|(tool, schema)| {
                json!({
                    "name": tool.name,
                    "title": tool.title,
                    "description": tool.description,
                    "inputSchema": schema,
                    "annotations": {
                        "title": tool.title,
                        "readOnlyHint": tool.read_only,
                        "destructiveHint": tool.destructive,
                        "idempotentHint": tool.idempotent,
                        "openWorldHint": false,
                    },
                })
            })
            .collect()
    }

    /// The tool called `name`, with its schema.
    pub fn find(&self, name: &str) -> Option<&(Tool, Value)> {
        self.tools.iter().find(|(tool, _)| tool.name == name)
    }

    /// The schemas tools are cut from.
    pub fn schemas(&self) -> &Schemas {
        &self.schemas
    }

    /// Runs the tool `name`; `None` when there is no such tool.
    pub fn call(&self, name: &str, arguments: Value, context: Context<'_>) -> Option<Outcome> {
        let (tool, _) = self.find(name)?;
        let arguments = match arguments {
            Value::Null => Value::Object(Map::new()),
            other => other,
        };
        Some(match run(tool, arguments, context) {
            Ok(outcome) => outcome,
            Err(error) => Outcome::error(&error),
        })
    }
}

impl Default for Toolbox {
    fn default() -> Self {
        Toolbox::new()
    }
}

/// The schema a tool's arguments follow.
fn input_schema(schemas: &Schemas, tool: &Tool) -> Option<Value> {
    match tool.action {
        Action::Method { method, .. } => schemas.method(method),
        Action::Command { op } => schemas.command(op),
        Action::Version => schemas.method("version"),
        Action::TimelineRead => schemas.method("project.get"),
        Action::ExportWait => {
            let mut schema = schemas.method("export.status")?;
            schema["properties"]["timeoutSeconds"] = json!({
                "type": "number",
                "minimum": 0,
                "maximum": WAIT_LONGEST,
                "default": WAIT_DEFAULT,
                "description": "How long to wait, in seconds.",
            });
            Some(schema)
        }
        Action::PreviewFrame => schemas.method("preview.frame"),
        Action::Mask => {
            let update = schemas.command("updateClip")?;
            let mut mask = schemas.def("ClipMask")?;
            mask["type"] = json!(["object", "null"]);
            mask["description"] = json!("The shape and its numbers, or null to take the mask off.");
            Some(json!({
                "type": "object",
                "properties": {
                    "path": update.pointer("/properties/path")?.clone(),
                    "clipId": update.pointer("/properties/clipId")?.clone(),
                    "mask": mask,
                },
                "required": ["path", "clipId", "mask"],
            }))
        }
        Action::Captions => {
            let path = schemas
                .method("project.get")?
                .pointer("/properties/path")?
                .clone();
            let mut style = schemas.def("TextStyle")?;
            style["description"] = json!(
                "The look every caption shares, as a title's style; only the fields that differ from the default are needed. Its content is replaced by each caption's text."
            );
            Some(json!({
                "type": "object",
                "properties": {
                    "path": path,
                    "captions": {
                        "type": "array",
                        "minItems": 1,
                        "description": "The captions, each shown from start to end.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "start": { "type": "number", "minimum": 0, "description": "When it appears, in seconds on the timeline." },
                                "end": { "type": "number", "minimum": 0, "description": "When it goes, in seconds on the timeline; after start." },
                                "text": { "type": "string", "description": "The words. A newline breaks the line." },
                            },
                            "required": ["start", "end", "text"],
                        },
                    },
                    "style": style,
                    "offsetY": {
                        "type": "number",
                        "default": 0.35,
                        "minimum": -3,
                        "maximum": 3,
                        "description": "Vertical placement as a fraction of frame height from the centre, positive down.",
                    },
                },
                "required": ["path", "captions"],
            }))
        }
    }
}

/// A request from a method's arguments.
fn request(method: &str, mut arguments: Value) -> Result<Request, ApiError> {
    let Some(fields) = arguments.as_object_mut() else {
        return Err(ApiError::invalid("the arguments are an object"));
    };
    fields.insert("method".to_owned(), json!(method));
    serde_json::from_value(arguments)
        .map_err(|error| ApiError::invalid(format!("invalid arguments: {error}")))
}

/// `edit.apply` of one command, from the command's arguments and the
/// project's `path` beside them.
fn edit(op: &str, mut arguments: Value) -> Result<Request, ApiError> {
    let Some(fields) = arguments.as_object_mut() else {
        return Err(ApiError::invalid("the arguments are an object"));
    };
    let path = fields
        .remove("path")
        .ok_or_else(|| ApiError::invalid("invalid arguments: missing field `path`"))?;
    fields.insert("op".to_owned(), json!(op));
    request("edit.apply", json!({ "path": path, "command": arguments }))
}

fn run(tool: &Tool, arguments: Value, context: Context<'_>) -> Result<Outcome, ApiError> {
    let backend = context.backend;
    match tool.action {
        Action::Method { method, reply } => {
            let result = backend.call(request(method, arguments)?)?;
            Ok(Outcome::json(match reply {
                Reply::Raw => result,
                Reply::Summary => summary::view(&result),
            }))
        }
        Action::Command { op } => {
            let result = backend.call(edit(op, arguments)?)?;
            Ok(Outcome::json(summary::view(&result)))
        }
        Action::Version => {
            let mut result = backend.call(request("version", arguments)?)?;
            result["mcp"] = backend.describe();
            Ok(Outcome::json(result))
        }
        Action::TimelineRead => {
            let result = backend.call(request("project.get", arguments)?)?;
            Ok(Outcome::json(summary::view(&result)))
        }
        Action::Captions => {
            let result = backend.call(captions(arguments)?)?;
            Ok(Outcome::json(summary::view(&result)))
        }
        Action::ExportWait => export_wait(arguments, backend, context.cancelled, context.progress),
        Action::PreviewFrame => preview(arguments, backend),
        Action::Mask => {
            let result = backend.call(mask(arguments)?)?;
            Ok(Outcome::json(summary::view(&result)))
        }
    }
}

/// `clip_mask`: a clip patch with only its mask.
fn mask(mut arguments: Value) -> Result<Request, ApiError> {
    let invalid = |why: &str| ApiError::invalid(format!("invalid arguments: {why}"));
    let fields = arguments
        .as_object_mut()
        .ok_or_else(|| invalid("the arguments are an object"))?;
    let path = fields
        .remove("path")
        .ok_or_else(|| invalid("missing field `path`"))?;
    let clip = fields
        .remove("clipId")
        .ok_or_else(|| invalid("missing field `clipId`"))?;
    let mask = fields
        .remove("mask")
        .ok_or_else(|| invalid("missing field `mask`"))?;
    if let Some(unknown) = fields.keys().next() {
        return Err(invalid(&format!("unknown field `{unknown}`")));
    }
    edit(
        "updateClip",
        json!({ "path": path, "clipId": clip, "patch": { "mask": mask } }),
    )
}

/// `captions_add`: one batch of titles.
fn captions(mut arguments: Value) -> Result<Request, ApiError> {
    let invalid = |why: &str| ApiError::invalid(format!("invalid arguments: {why}"));
    let fields = arguments
        .as_object_mut()
        .ok_or_else(|| invalid("the arguments are an object"))?;
    let path = fields
        .remove("path")
        .ok_or_else(|| invalid("missing field `path`"))?;
    let captions = match fields.remove("captions") {
        Some(Value::Array(captions)) if !captions.is_empty() => captions,
        _ => return Err(invalid("`captions` is a list of {start, end, text}")),
    };
    let offset = match fields.remove("offsetY") {
        None | Some(Value::Null) => 0.35,
        Some(value) => value
            .as_f64()
            .ok_or_else(|| invalid("`offsetY` is a number"))?,
    };
    let style = match fields.remove("style") {
        None | Some(Value::Null) => json!({
            "fontFamily": "Hanken Grotesk",
            "fontSize": 0.04,
            "fontWeight": 600,
        }),
        Some(Value::Object(style)) => Value::Object(style),
        Some(_) => return Err(invalid("`style` is an object")),
    };
    if let Some(unknown) = fields.keys().next() {
        return Err(invalid(&format!("unknown field `{unknown}`")));
    }
    let mut commands = Vec::with_capacity(captions.len());
    for (index, caption) in captions.iter().enumerate() {
        let number = |key: &str| caption.get(key).and_then(Value::as_f64);
        let (Some(start), Some(end), Some(text)) = (
            number("start"),
            number("end"),
            caption.get("text").and_then(Value::as_str),
        ) else {
            return Err(invalid(&format!(
                "caption {index} needs a start, an end and a text"
            )));
        };
        if !(start >= 0.0 && end > start) {
            return Err(invalid(&format!(
                "caption {index} ends at {end} s, not after its start at {start} s"
            )));
        }
        let mut style = style.clone();
        style["content"] = json!(text);
        commands.push(json!({
            "op": "addTextClip",
            "trackId": null,
            "above": true,
            "start": start,
            "duration": end - start,
            "style": style,
            "offsetY": offset,
        }));
    }
    request(
        "edit.apply",
        json!({ "path": path, "command": { "op": "batch", "commands": commands } }),
    )
}

/// `export_wait`: the job's status once it has ended, or when the time is
/// up, with progress reported on the way.
fn export_wait(
    mut arguments: Value,
    backend: &mut dyn Backend,
    cancelled: &AtomicBool,
    progress: Option<&dyn Fn(f64, f64, String)>,
) -> Result<Outcome, ApiError> {
    let timeout = match arguments
        .as_object_mut()
        .and_then(|fields| fields.remove("timeoutSeconds"))
    {
        None | Some(Value::Null) => WAIT_DEFAULT,
        Some(value) => value
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .ok_or_else(|| {
                ApiError::invalid("invalid arguments: `timeoutSeconds` is a number of seconds")
            })?
            .min(WAIT_LONGEST),
    };
    let status = request("export.status", arguments)?;
    let until = Instant::now() + Duration::from_secs_f64(timeout);
    let mut reported = -1.0_f64;
    loop {
        let mut current = backend.call(status.clone())?;
        let running = current["state"] == "running";
        if let Some(progress) = progress {
            let stage = current["stage"].as_str().unwrap_or_default();
            let place = stage_place(stage) as f64;
            let fraction = current["fraction"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
            let overall = if running {
                ((place + fraction) / STAGES as f64 * 100.0).min(99.9)
            } else {
                100.0
            };
            // A progress value must rise; a stage that starts over at 0
            // says nothing new until it passes where the last one was.
            if overall > reported {
                reported = overall;
                let frames = match (current["frame"].as_i64(), current["total"].as_i64()) {
                    (Some(frame), Some(total)) if total > 0 => format!(" {frame}/{total}"),
                    _ => String::new(),
                };
                let state = current["state"].as_str().unwrap_or_default();
                progress(
                    (overall * 10.0).round() / 10.0,
                    100.0,
                    format!("{state} {stage}{frames}").trim().to_owned(),
                );
            }
        }
        if !running {
            return Ok(Outcome::json(current));
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(ApiError::new(
                ErrorCode::Cancelled,
                "stopped waiting; the export goes on",
            ));
        }
        if Instant::now() >= until {
            current["note"] = json!("still running: call export_wait again, or export_status");
            return Ok(Outcome::json(current));
        }
        std::thread::sleep(POLL);
    }
}

/// `preview_frame`: the picture as an image, or the file written.
fn preview(mut arguments: Value, backend: &mut dyn Backend) -> Result<Outcome, ApiError> {
    let wants_file = arguments
        .get("output")
        .is_some_and(|output| !output.is_null());
    let sized = ["width", "height"]
        .iter()
        .all(|key| arguments.get(*key).is_some_and(|value| !value.is_null()));
    if !wants_file && !sized {
        // The timeline's own frame, fitted within PREVIEW_SIDE.
        let path = arguments.get("path").cloned().unwrap_or(Value::Null);
        let view = backend.call(request("project.get", json!({ "path": path }))?)?;
        let width = view
            .pointer("/settings/width")
            .and_then(Value::as_f64)
            .unwrap_or(1920.0);
        let height = view
            .pointer("/settings/height")
            .and_then(Value::as_f64)
            .unwrap_or(1080.0);
        let scale = (PREVIEW_SIDE / width.max(height)).min(1.0);
        if let Some(fields) = arguments.as_object_mut() {
            fields.insert(
                "width".to_owned(),
                json!(((width * scale).round() as u32).max(1)),
            );
            fields.insert(
                "height".to_owned(),
                json!(((height * scale).round() as u32).max(1)),
            );
        }
    }
    let time = arguments.get("time").cloned().unwrap_or(Value::Null);
    let result = backend.call(request("preview.frame", arguments)?)?;
    let Some(png) = result.get("png").and_then(Value::as_str) else {
        return Ok(Outcome::json(result));
    };
    Ok(Outcome {
        content: vec![
            json!({ "type": "image", "data": png, "mimeType": "image/png" }),
            json!({
                "type": "text",
                "text": format!("The frame at {time} s, {}×{}.", result["width"], result["height"]),
            }),
        ],
        structured: Some(
            json!({ "width": result["width"], "height": result["height"], "time": time }),
        ),
        is_error: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::check;

    /// One call each tool may take, as an agent would write it.
    fn samples() -> Vec<(&'static str, Value)> {
        vec![
            ("concat_version", json!({})),
            ("project_list", json!({})),
            (
                "project_create",
                json!({ "location": "/tmp", "name": "Cut", "video": { "width": 1080, "height": 1920, "rateNum": 30, "rateDen": 1 } }),
            ),
            ("project_open", json!({ "path": "/tmp/Cut" })),
            ("project_save", json!({ "path": "/tmp/Cut" })),
            ("project_close", json!({ "path": "/tmp/Cut", "save": true })),
            ("timeline_read", json!({ "path": "/tmp/Cut" })),
            ("project_get", json!({ "path": "/tmp/Cut" })),
            (
                "project_set_video",
                json!({ "path": "/tmp/Cut", "video": { "width": 1920, "height": 1080, "rateNum": 30000, "rateDen": 1001 } }),
            ),
            ("media_probe", json!({ "path": "/tmp/a.mp4" })),
            (
                "media_import",
                json!({ "path": "/tmp/Cut", "file": "/tmp/a.mp4" }),
            ),
            (
                "edit_apply",
                json!({ "path": "/tmp/Cut", "command": { "op": "addTrack" } }),
            ),
            ("edit_undo", json!({ "path": "/tmp/Cut" })),
            ("edit_redo", json!({ "path": "/tmp/Cut" })),
            (
                "clip_add",
                json!({ "path": "/tmp/Cut", "mediaId": "m1", "trackId": "t1", "start": 2.5 }),
            ),
            (
                "clip_trim",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "edge": "end", "delta": -1.0, "ripple": true }),
            ),
            (
                "clips_split",
                json!({ "path": "/tmp/Cut", "clipIds": ["c1"], "time": 3.0 }),
            ),
            (
                "clips_move",
                json!({ "path": "/tmp/Cut", "moves": [{ "clipId": "c1", "start": 0.0, "trackId": "t2" }] }),
            ),
            (
                "clips_remove",
                json!({ "path": "/tmp/Cut", "clipIds": ["c1"], "ripple": false }),
            ),
            (
                "clip_update",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "patch": { "volume": 0.5, "crop": null, "transitionIn": { "id": "cross-fade", "duration": 0.5 }, "videoEffects": [{ "id": "concat.warm", "params": { "amount": 0.4 } }] } }),
            ),
            (
                "clip_transform",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "scale": 1.2, "rotation": 15.0 }),
            ),
            (
                "clip_speed",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "speed": 0.5 }),
            ),
            (
                "title_add",
                json!({ "path": "/tmp/Cut", "start": 1.0, "duration": 3.0, "above": true, "style": { "content": "Hello", "fontSize": 0.08, "align": "left" }, "offsetY": -0.3 }),
            ),
            (
                "captions_add",
                json!({ "path": "/tmp/Cut", "captions": [{ "start": 0.0, "end": 1.5, "text": "Hi" }], "style": { "color": "#ffff00" }, "offsetY": 0.3 }),
            ),
            (
                "keyframe_set",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "property": "scale", "at": 0.0, "value": 1.0, "ease": [0.42, 0.0, 0.58, 1.0] }),
            ),
            (
                "keyframe_clear",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "property": "opacity", "at": 1.0 }),
            ),
            (
                "effect_layer_add",
                json!({ "path": "/tmp/Cut", "start": 0.0, "duration": 2.0, "effectId": "concat.warm", "name": "Warm", "trackId": null }),
            ),
            (
                "effect_key_set",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "entry": 0, "key": "amount", "at": 0.5, "value": 0.8 }),
            ),
            ("track_add", json!({ "path": "/tmp/Cut", "bottom": false })),
            (
                "clips_add_sequence",
                json!({ "path": "/tmp/Cut", "mediaIds": ["m1", "m2"], "start": 0.0 }),
            ),
            (
                "clip_mask",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "mask": { "shape": "heart", "width": 0.5, "feather": 0.1, "invert": true } }),
            ),
            (
                "clip_stabilize",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "stabilize": { "smoothness": 0.7, "maxZoom": 1.2, "tripod": false } }),
            ),
            (
                "clip_frame",
                json!({ "path": "/tmp/Cut", "clipId": "c1", "framing": "fill" }),
            ),
            (
                "timeline_background",
                json!({ "path": "/tmp/Cut", "timelineId": "tl1", "background": { "kind": "blur", "amount": 0.5 } }),
            ),
            ("effects_list", json!({ "kind": "transition" })),
            ("template_list", json!({})),
            (
                "template_instantiate",
                json!({ "template": "/t", "location": "/tmp", "name": "From", "fills": [{ "mediaId": "m1", "file": "/tmp/a.mp4" }] }),
            ),
            (
                "export_start",
                json!({ "path": "/tmp/Cut", "output": "/tmp/cut.mp4", "crf": 23, "codec": "h264" }),
            ),
            ("export_status", json!({ "job": "j1" })),
            ("export_wait", json!({ "job": "j1", "timeoutSeconds": 5 })),
            ("export_cancel", json!({ "job": "j1" })),
            (
                "preview_frame",
                json!({ "path": "/tmp/Cut", "time": 1.5, "width": 320, "height": 180 }),
            ),
        ]
    }

    #[test]
    fn every_tool_has_a_schema_and_a_sample_and_names_are_unique() {
        let toolbox = Toolbox::new();
        let listed = toolbox.list();
        let samples = samples();
        assert_eq!(listed.len(), samples.len(), "a sample for every tool");
        let mut names: Vec<&str> = listed
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed.len(), "unique names");
        for tool in &listed {
            let name = tool["name"].as_str().expect("named");
            assert!(
                name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{name}"
            );
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(
                tool["description"]
                    .as_str()
                    .is_some_and(|text| text.len() > 40),
                "{name}"
            );
        }
    }

    /// The round trip: what each tool's schema accepts is what its Rust
    /// type reads, and what the Rust type writes the schema accepts.
    #[test]
    fn every_sample_fits_its_schema_and_reads_as_its_request() {
        let toolbox = Toolbox::new();
        for (name, sample) in samples() {
            let (tool, schema) = toolbox.find(name).unwrap_or_else(|| panic!("no {name}"));
            check(schema, &sample, schema, name).unwrap_or_else(|why| panic!("{why}"));
            let built = match tool.action {
                Action::Method { method, .. } => request(method, sample.clone()),
                Action::Command { op } => edit(op, sample.clone()),
                Action::Version => request("version", sample.clone()),
                Action::TimelineRead => request("project.get", sample.clone()),
                Action::Captions => captions(sample.clone()),
                Action::ExportWait => {
                    let mut sample = sample.clone();
                    sample
                        .as_object_mut()
                        .map(|fields| fields.remove("timeoutSeconds"));
                    request("export.status", sample)
                }
                Action::PreviewFrame => request("preview.frame", sample.clone()),
                Action::Mask => mask(sample.clone()),
            };
            let built = built.unwrap_or_else(|error| panic!("{name}: {error}"));
            // Back again: the request as the API writes it fits the API's
            // own schema for requests.
            let written = serde_json::to_value(&built).expect("writes");
            let root = concat_api::request_schema();
            check(&root, &written, &root, name).unwrap_or_else(|why| panic!("{why}"));
        }
    }

    /// A backend whose export reports the statuses it is given, in order,
    /// the last for ever.
    struct Statuses(std::collections::VecDeque<Value>);

    impl Backend for Statuses {
        fn call(&mut self, _request: Request) -> Result<Value, ApiError> {
            let status = if self.0.len() > 1 {
                self.0.pop_front()
            } else {
                self.0.front().cloned()
            };
            Ok(status.expect("a status"))
        }

        fn describe(&self) -> Value {
            json!({ "mode": "test" })
        }
    }

    /// An export that mixes sound and muxes it reports the stages the
    /// exporter names - "rendering", "mixing audio", "muxing" - and the
    /// wait's progress climbs through them to the end.
    #[test]
    fn export_wait_progress_follows_the_stages_the_exporter_reports() {
        let running = |stage: &str, frame: i64, total: i64| {
            json!({ "state": "running", "stage": stage, "frame": frame, "total": total,
                    "fraction": if total > 0 { frame as f64 / total as f64 } else { 0.0 } })
        };
        let mut backend = Statuses(
            [
                running("rendering", 50, 100),
                running("mixing audio", 0, 100),
                running("muxing", 100, 100),
                json!({ "state": "done", "stage": "muxing", "frame": 100, "total": 100, "fraction": 1.0 }),
            ]
            .into(),
        );
        let seen = std::cell::RefCell::new(Vec::new());
        let report =
            |progress: f64, total: f64, _: String| seen.borrow_mut().push((progress, total));
        let cancelled = AtomicBool::new(false);
        let outcome = export_wait(
            json!({ "job": "j1" }),
            &mut backend,
            &cancelled,
            Some(&report),
        )
        .expect("waits");
        assert!(!outcome.is_error);
        assert_eq!(
            *seen.borrow(),
            vec![(50.0, 100.0), (60.0, 100.0), (99.9, 100.0), (100.0, 100.0)]
        );
    }

    #[test]
    fn a_wrong_argument_is_refused_by_the_schema_and_by_the_type_alike() {
        let toolbox = Toolbox::new();
        let (_, schema) = toolbox.find("clip_speed").expect("a tool");
        let wrong = json!({ "path": "/p", "clipId": "c1", "speed": "fast" });
        assert!(check(schema, &wrong, schema, "clip_speed").is_err());
        let error = edit("setClipSpeed", wrong).expect_err("refused");
        assert_eq!(error.code, ErrorCode::Invalid);
        let missing = request("project.open", json!({}));
        assert!(missing.expect_err("refused").message.contains("path"));
        assert!(
            captions(json!({ "path": "/p", "captions": [{ "start": 2, "end": 1, "text": "x" }] }))
                .is_err()
        );
    }
}
