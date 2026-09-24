# Methods

**In one line:** every method the API answers, what it takes, what it gives
back.

## Cheat sheet

| Method | Does | Replies with |
|---|---|---|
| [`version`](#version) | what this build serves | *VersionInfo* |
| [`project.create`](#projectcreate) | new project folder, opened | *EditorView* |
| [`project.open`](#projectopen) | open a folder | *EditorView* |
| [`project.close`](#projectclose) | close, optionally save | `{}` |
| [`project.list`](#projectlist) | recent projects | *ProjectInfo*[] |
| [`project.get`](#projectget) | current state | *EditorView* |
| [`project.document`](#projectdocument) | the `concat.json` a save writes | JSON object |
| [`project.save`](#projectsave) | write to disk, optionally rename | `{}` |
| [`project.setVideo`](#projectsetvideo) | frame size and rate (undoable) | *EditorView* |
| [`edit.apply`](#editapply) | one edit command | *EditorView* |
| [`edit.undo`](#editundo) | step back | *EditorView* |
| [`edit.redo`](#editredo) | step forward | *EditorView* |
| [`media.probe`](#mediaprobe) | what is in a file | *MediaSummary* |
| [`media.import`](#mediaimport) | probe + add to bin | *EditorView* |
| [`catalogue.list`](#cataloguelist) | effect packages and their params | *PackageInfo*[] |
| [`template.list`](#templatelist) | the template library | *TemplateInfo*[] |
| [`template.instantiate`](#templateinstantiate) | project from a template | *EditorView* |
| [`template.save`](#templatesave) | project into a template | *TemplateInfo* |
| [`export.run`](#exportrun) | render to a file, as a job | *Started* |
| [`export.cancel`](#exportcancel) | stop a job | `{}` |
| [`preview.frame`](#previewframe) | one frame as PNG | *Written* or *Picture* |

Types in *italics* are in [Types](types.md).

## How to read this page

- Examples are **bare requests**, the shape `concat-cli api` takes on
  stdin. On a socket or over gRPC the same fields go in `params` inside
  the envelope. See [JSON-RPC](../transports/json-rpc.md).
- Replies are shown as the `result` payload alone.
- *optional* parameters may be left out; the default is stated.
- `path` is always the project **folder**, and it must be open first, or
  the reply is `notOpen`.

---

## `version`

What this build serves. **Call it first.**

**Parameters:** none.

```json
{"method":"version"}
```

**Reply**, a *VersionInfo*:

```json
{
  "apiVersion": "0.2",
  "concat": "0.2.4",
  "dirs": {
    "config": "/Users/ada/Library/Application Support/app.concat.editor",
    "data": "/Users/ada/Library/Application Support/app.concat.editor"
  },
  "capabilities": ["events", "gpu", "json-rpc"]
}
```

The capability names and the bump rules are in the
[overview](overview.md#versioning-and-capabilities).

---

## `project.create`

Creates a project folder and opens it.

| Parameter | Type | Meaning |
|---|---|---|
| `location` | string | The directory the project folder is made in |
| `name` | string | The project's name. The folder is named after it; characters a filesystem refuses become `-` |
| `video` | *VideoSettings* | *optional*. Frame and rate. **Default:** 1920×1080 at 30 fps |

```json
{"method":"project.create","location":"/edits","name":"Reel","video":{"width":1080,"height":1920,"rateNum":30,"rateDen":1}}
```

**Reply:** an *EditorView* of the new project.

Good to know:

- A fresh project has one timeline `TL1` with four tracks `T1`–`T4`.
- It is added to the recents list, like one the window made.
- A folder that already holds a project is refused (`failed`).
- Over a socket, `location` must lie under one of the server's write
  roots, or the request is `refused`; see the JSON-RPC transport's
  [Security](../transports/json-rpc.md#security). A folder the window has
  open is `refused` too.

---

## `project.open`

Opens a project folder.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |

```json
{"method":"project.open","path":"/edits/Reel"}
```

**Reply:** an *EditorView*.

Good to know:

- Opening a folder already open returns its state as it stands, edits and
  all. The history is untouched.

---

## `project.close`

Closes an open project.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `save` | bool | *optional*. Write the document first. **Default:** `false` |

```json
{"method":"project.close","path":"/edits/Reel","save":true}
```

**Reply:** `{}`.

> [!WARNING]
> Without `save: true`, unsaved edits are dropped.

A running export is not affected; it holds its own copy of what it needs.

---

## `project.list`

The projects this machine opened most recently, newest first.

**Parameters:** none.

**Reply:** an array of *ProjectInfo*.

```json
[{"path":"/edits/Reel","name":"Reel","width":1920,"height":1080,"rateNum":30,"rateDen":1,"openedAt":1790002385144}]
```

This is the launch screen's list. It names projects whether or not they
are open now.

---

## `project.get`

The state of an open project.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |

**Reply:** an *EditorView*.

---

## `project.document`

The document exactly as a save writes it: the contents of `concat.json`.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |

**Reply:** the document, a JSON object.

> [!TIP]
> To inspect the project as data, prefer `project.get`. Its `project`
> field is the same model without the flat `tracks` / `clips` / `video`
> mirror that older builds read.

---

## `project.save`

Writes the document to the project folder.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `name` | string | *optional*. A new name for the project |

**Reply:** `{}`.

---

## `project.setVideo`

Sets the active timeline's frame and rate, as an **undoable** edit.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `video` | *VideoSettings* | The frame and rate |

```json
{"method":"project.setVideo","path":"/edits/Reel","video":{"width":3840,"height":2160,"rateNum":60,"rateDen":1}}
```

**Reply:** an *EditorView*.

A zero dimension or rate is `refused`.

---

## `edit.apply`

Applies **one edit command** and records one undo step.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `command` | *Command* | The edit: `{"op": "…", …fields}`. All of them are in [Edit commands](edits.md) |

```json
{"method":"edit.apply","path":"/edits/Reel","command":{"op":"addClip","mediaId":"m1","trackId":"T1","start":2.5}}
```

**Reply:** an *EditorView* of the project after the edit.

Good to know:

- `createdId` in the reply is the id of what the command made: a clip
  `c…`, track `t…`, timeline `tl…` or media `m…`. For a `batch` it is the
  last id minted inside.
- A **refusal** is a `refused` error. Its message is the sentence the
  window would show. Nothing changed.
- An edit that names something no longer there is, for most commands, a
  **tolerated no-op**: unchanged view, no undo step.
- Several commands as one undo step: wrap them in `{"op": "batch",
  "commands": [...]}`.

---

## `edit.undo`

Steps the history back one edit.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |

**Reply:** an *EditorView*. With nothing to undo, it is the unchanged view.
`canUndo` and `canRedo` say where the history stands.

---

## `edit.redo`

Steps the history forward one edit.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |

**Reply:** an *EditorView*.

---

## `media.probe`

What is inside a media file. Touches no project.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The file |

```json
{"method":"media.probe","path":"/footage/take1.mp4"}
```

**Reply:** a *MediaSummary*.

```json
{
  "path": "/footage/take1.mp4",
  "duration": 6.121029,
  "kind": "video",
  "video": {"index":0,"codec":"h264","width":854,"height":480,"frameRate":30.0,"frameRateFraction":"30/1"},
  "audio": {"index":1,"codec":"aac","sampleRate":48000,"channels":2,"title":"","language":"und"},
  "audioTracks": [
    {"index":1,"codec":"aac","sampleRate":48000,"channels":2,"title":"","language":"und"},
    {"index":2,"codec":"aac","sampleRate":48000,"channels":2,"title":"","language":"und"}
  ]
}
```

A file that cannot be read or decoded is `failed`.

---

## `media.import`

Probes a file and adds it to the project's bin. What dropping a file on
the window does.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `file` | string | The file to import |

```json
{"method":"media.import","path":"/edits/Reel","file":"/footage/take1.mp4"}
```

**Reply:** an *EditorView*. `createdId` is the new media id (`m1`, `m2`, …).

Good to know:

- A path already in the bin is a no-op, and `createdId` is absent.
- This is `media.probe` + the `addMedia` command, as one request.
- The bin item's fields are in [Types → MediaItem](types.md#mediaitem).

---

## `catalogue.list`

Every effect package the build knows, with its parameters. Enough to build
a valid effect chain without reading a manifest.

| Parameter | Type | Meaning |
|---|---|---|
| `kind` | string | *optional*. One of `effect`, `filter`, `audio`, `transition`, `generator`. **Default:** all kinds. Any other word is `invalid` |

```json
{"method":"catalogue.list","kind":"effect"}
```

**Reply:** an array of *PackageInfo*, in id order.

```json
[{
  "id": "concat.gaussian-blur",
  "name": "Gaussian Blur",
  "kind": "effect",
  "category": "Blur",
  "description": "A soft, even blur.",
  "intensity": "radius",
  "params": [
    {"key":"radius","label":"Radius","type":"float","min":1.0,"max":50.0,"default":10.0,"step":1.0,"unit":"px","animate":false,"values":[],"labels":[]}
  ]
}]
```

How to use it:

- `id` is what a clip's chain stores: in `videoEffects`, `filters`, or a
  `transitionIn`.
- each param's `key` is what the chain entry's `params` object stores it
  under. See [Types → AppliedFilter](types.md#appliedfilter).

---

## `template.list`

The template library.

**Parameters:** none.

**Reply:** an array of *TemplateInfo*, each naming its slots.

What a template is: a bundle folder under `templates/` in the config
directory, holding `template.json` (a project document whose placeholder
media have blank paths), `assets/` (media and fonts that are part of the
design) and `poster.jpg`.

---

## `template.instantiate`

Makes a project from a template with every slot filled, and opens it.

| Parameter | Type | Meaning |
|---|---|---|
| `template` | string | The bundle folder, as `template.list` gave it |
| `location` | string | The directory the project folder is made in |
| `name` | string | The project's name |
| `fills` | array of `{"mediaId", "file"}` | The file for each slot. `mediaId` is the slot's id from `template.list` |

```json
{"method":"template.instantiate","template":"/Users/ada/Library/Application Support/app.concat.editor/templates/Intro","location":"/edits","name":"My intro","fills":[{"mediaId":"m1","file":"/footage/logo.png"},{"mediaId":"m2","file":"/footage/take1.mp4"}]}
```

**Reply:** an *EditorView* of the new project.

Good to know:

- **Every slot must be filled.** A set that leaves one empty makes nothing.
- Every file is probed first, so a bad path refuses the whole request and
  leaves no folder behind.
- Over a socket, `location` must lie under one of the server's write
  roots, or the request is `refused`.

---

## `template.save`

Packs an open project into a new template bundle in the library.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `name` | string | The template's name |

**Reply:** the new bundle's *TemplateInfo*.

---

## `export.run`

Renders the active timeline to a file, as a **job**. Exactly what the
window's Export sheet does: cutouts analysed, titles painted, then the
frame loop and the mix.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `output` | string | The file to write. The container follows the extension; `.mp4` is the usual choice |
| `crf` | integer | *optional*. Constant rate factor; lower is better and bigger. **Default:** 20 |
| `preset` | string | *optional*. x264 preset name, `ultrafast` … `veryslow`. **Default:** `medium` |
| `width` | integer | *optional*. **Default:** the timeline's |
| `height` | integer | *optional*. **Default:** the timeline's |
| `rateNum` | integer | *optional*. Frame rate numerator. **Default:** the timeline's |
| `rateDen` | integer | *optional*. Frame rate denominator. **Default:** the timeline's |
| `codec` | string | *optional*. `h264`, `hevc` or `av1`. **Default:** `h264`. Anything else is `invalid` |
| `tenBit` | bool | *optional*. Ten bits a channel. **Default:** `false` |
| `colorRange` | string | *optional*. `limited` (16-235, what every player and YouTube expect) or `full` (0-255, for screen content bound for a PC player). The file is tagged and converted to match. **Default:** `limited`. Anything else is `invalid` |

```json
{"method":"export.run","path":"/edits/Reel","output":"/edits/reel.mp4","crf":18,"codec":"hevc"}
```

**Reply**, a *Started*, at once:

```json
{"job":"j1","path":"/edits/Reel","output":"/edits/reel.mp4"}
```

**Then events**, until the job ends:

```jsonl
{"event":"cutout.progress","job":"j1","path":"/edits/Reel","mediaId":"m3","fetching":true,"fraction":0.4}
{"event":"export.progress","job":"j1","path":"/edits/Reel","frame":30,"total":150,"stage":"video"}
{"event":"export.progress","job":"j1","path":"/edits/Reel","frame":150,"total":150,"stage":"mux"}
{"event":"export.done","job":"j1","path":"/edits/Reel","output":"/edits/reel.mp4","width":1920,"height":1080}
```

or

```json
{"event":"export.failed","job":"j1","path":"/edits/Reel","error":{"code":"failed","message":"…"}}
```

Rules:

- **One export at a time.** A second is refused with `busy`. From the
  window's Remote page the slot is the window's own, so an export begun
  in the Export sheet counts.
- An empty timeline is `refused`.
- Over a socket, `output` must lie under one of the server's write roots,
  or the request is `refused`.
- `width` and `height` are at most 8192 a side, the frame rate at most
  240 a second, `crf` at most 63: anything larger is `invalid`.
- `cutout.progress` only appears for clips with an automatic cutout whose
  masks are not cached yet. `fetching` is true while the model downloads.
- Event shapes are in [Types → Events](types.md#events). The job model is
  in the [overview](overview.md#jobs-and-events).

---

## `export.cancel`

Stops a running export at its next frame.

| Parameter | Type | Meaning |
|---|---|---|
| `job` | string | The job `export.run` named |

**Reply:** `{}`. The job then ends with `export.failed` whose `error.code`
is `cancelled`.

A job that is not running (finished, or never existed) is `notFound`.

---

## `preview.frame`

Composites the true frame at one instant, titles and effects included, as
a PNG.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `time` | number | The timeline instant, in seconds |
| `output` | string | *optional*. The file to write; folders above it are created. **Absent:** the picture comes back inline |
| `width` | integer | *optional*. Frame width |
| `height` | integer | *optional*. Frame height |

```json
{"method":"preview.frame","path":"/edits/Reel","time":1.5,"output":"/edits/frames/1.5.png","width":640,"height":360}
```

**Reply with `output`**, a *Written*:

```json
{"path":"/edits/frames/1.5.png","width":640,"height":360}
```

**Reply without**, a *Picture*:

```json
{"width":640,"height":360,"png":"iVBORw0KGgo…"}
```

Rules:

- `width` and `height` count only **together**. Give both or neither;
  neither means the timeline's size.
- Zero for either is `invalid`, and so is anything over 8192 a side.
- Over a socket, `output` must lie under one of the server's write roots,
  or the request is `refused`.
- The PNG is RGBA, 8 bits a channel, base64 in the standard alphabet with
  padding.
