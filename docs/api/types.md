# Types

**In one line:** the JSON shapes that requests carry and replies return.

**On this page:** [Replies](#replies) · [The project model](#the-project-model)
· [Media](#media) · [Catalogue](#catalogue) · [Templates](#templates) ·
[Events](#events) · [Errors](#errors)

Every field is camelCase. A field marked *optional* may be absent from a
reply; a request may leave it out.

---

## Replies

### VersionInfo

Reply to `version`.

| Field | Type | Meaning |
|---|---|---|
| `apiVersion` | string | The contract, e.g. `"0.2"` |
| `concat` | string | The build's version |
| `dirs` | `{config, data}` | The app's directories, as paths |
| `capabilities` | string[] | See [overview](overview.md#versioning-and-capabilities) |

### EditorView

Reply to every method that opens or changes a project.

| Field | Type | Meaning |
|---|---|---|
| `project` | *Project* | The whole project |
| `canUndo` | bool | There is something to undo |
| `canRedo` | bool | There is something to redo |
| `settings` | `{name, width, height, rateNum, rateDen}` | The project's name and the active timeline's frame and rate |
| `createdId` | string | *optional*. The id the command just minted |

```json
{
  "project": { "…": "see Project" },
  "canUndo": true,
  "canRedo": false,
  "settings": {"name":"Reel","width":1920,"height":1080,"rateNum":30,"rateDen":1},
  "createdId": "c2"
}
```

### ProjectInfo

One entry of `project.list`.

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The project folder |
| `name` | string | The name in the title bar |
| `width`, `height` | integer | Output size |
| `rateNum`, `rateDen` | integer | Frame rate as a fraction |
| `openedAt` | integer | Milliseconds since the epoch |

### Started

Reply to `export.run`.

| Field | Type | Meaning |
|---|---|---|
| `job` | string | `"j1"`, `"j2"`, … Every event about this job carries it |
| `path` | string | The project folder |
| `output` | string | The file it will write |

### Written

Reply to `preview.frame` with `output`.

| Field | Type |
|---|---|
| `path` | string, the file written |
| `width`, `height` | integer |

### Picture

Reply to `preview.frame` without `output`.

| Field | Type |
|---|---|
| `width`, `height` | integer |
| `png` | string, the PNG as base64 (standard alphabet, padded) |

### Done

`{}`. The reply of methods with nothing to say beyond having worked.

---

## The project model

What `project.get` returns as `project`, and what `project.document` writes
to `concat.json` (plus a compatibility mirror; see
[`project.document`](methods.md#projectdocument)).

### Project

| Field | Type | Meaning |
|---|---|---|
| `media` | *MediaItem*[] | The bin, shared by all timelines |
| `fonts` | `{family, path}`[] | Fonts added from disk |
| `timelines` | *Timeline*[] | In tab order. Always at least one |
| `activeTimelineId` | string | The timeline commands act on |

### Timeline

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `"TL1"` for the first, `"tl…"` after |
| `name` | string | The tab label |
| `video` | *VideoSettings* | This timeline's frame and rate |
| `tracks` | *Track*[] | Top to bottom. Never empty |
| `clips` | *Clip*[] | In insertion order, **not** time order; sort by `start` yourself |

### VideoSettings

| Field | Type | Meaning |
|---|---|---|
| `width`, `height` | integer | Frame size in pixels |
| `rateNum`, `rateDen` | integer | Frame rate as a fraction: 30 fps is `30/1`, 29.97 is `30000/1001` |

### Track

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `"T1"`–`"T4"` on the first timeline, `"t…"` after |
| `visible` | bool | Video on this track reaches the composite |
| `muted` | bool | Audio on this track is silent |

### Clip

A clip as `project.get` returns it. Fields with a default are left out of
the JSON when they hold it.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `"c…"`. Survives splits (the head keeps it) and merges |
| `trackId` | string | The lane |
| `mediaId` | string | Empty for a text or layer clip |
| `name` | string | Display label |
| `kind` | string | `video`, `audio`, `image`, `text`, `layer` |
| `start` | number | Seconds from the start of the timeline |
| `duration` | number | Seconds on the timeline. Never below 1/60 |
| `sourceStart` | number | In-point: seconds into the media |
| `volume` | number | Linear gain, 1 is unity |
| `fadeIn`, `fadeOut` | number | Audio ramp seconds |
| `scale` | number | Multiplier over the fitted size |
| `offsetX`, `offsetY` | number | Fractions of frame width / height from centred |
| `rotation` | number | Degrees, clockwise |
| `stretchX`, `stretchY` | number | *optional*, default 1 |
| `opacity` | number | 0..1 |
| `speed` | number | Playback rate; the curve's mean when a curve is set |
| `speedCurve` | *SpeedPoint*[] | *optional* |
| `reverse` | bool | *optional* |
| `keys` | *ClipKey*[] | *optional*. User keyframes, sorted by property then `at` |
| `flipH`, `flipV` | bool | *optional* |
| `blend` | string | *optional*. `multiply`, `screen`, `add`, `lighten`, `darken`; absent is normal |
| `crop` | *Crop* | *optional* |
| `cutout` | *Cutout* | *optional* |
| `preservePitch` | bool | Keep pitch when speed ≠ 1 |
| `filters` | *AppliedFilter*[] | Audio chain, in order |
| `videoEffects` | *AppliedFilter*[] | Video chain, in order |
| `audioStream` | integer | *optional*. Which audio stream of the media plays |
| `muted` | bool | *optional*. True when the sound is detached |
| `detachedFrom` | string | *optional*. On a detached audio clip: the video it came from |
| `transitionIn` | *Transition* | *optional* |
| `text` | *TextStyle* | *optional*. Present on a text clip |

A clip fresh from `addClip`:

```json
{"id":"c2","trackId":"T1","mediaId":"m1","name":"take1.mp4","kind":"video","start":0.0,"duration":6.12,"sourceStart":0.0,"volume":1.0,"fadeIn":0.0,"fadeOut":0.0,"scale":1.0,"offsetX":0.0,"offsetY":0.0,"rotation":0.0,"opacity":1.0,"speed":1.0,"preservePitch":true,"filters":[],"videoEffects":[]}
```

### MediaItem

One entry of the bin. The probe's findings are stored, so a project opens
meaningfully even when the file is missing.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `"m…"`, never re-issued |
| `path` | string | Absolute path. Also the duplicate check |
| `name` | string | Display name, normally the file's basename |
| `duration` | number | *optional*. Seconds, when the container says |
| `kind` | string | `video`, `audio`, `image` |
| `width`, `height` | integer | *optional* |
| `frameRate` | number | *optional*. Decimal, for display |
| `frameRateFraction` | string | *optional*. Exact, e.g. `"30000/1001"` |
| `videoCodec`, `audioCodec` | string | *optional* |
| `hasAudio` | bool | The file carries sound |
| `audioTracks` | *AudioTrack*[] | *optional*. Every audio stream, in file order |
| `placeholder` | bool | *optional*. True for a template slot |
| `colorRange` | string | *optional*. `limited` or `full`: the levels the picture is read as, over the file's own tag. Absent means "as tagged". Set with `setMediaColorRange` |
| `origin` | string | *optional*. `speech` for a file the speech sheet read aloud. Absent for an import. The bin shelves a file with an origin under Generated, not with the imports |

### AudioTrack

| Field | Type |
|---|---|
| `index` | integer, the stream's index in the file |
| `codec` | string |
| `channels` | integer |
| `sampleRate` | integer |
| `title` | string, *optional* |
| `language` | string, *optional* |

### AppliedFilter

One link of an effect chain, in `filters` or `videoEffects`.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | A package id from `catalogue.list`, e.g. `"concat.gaussian-blur"` |
| `params` | object | Parameter key → number. Missing keys mean the package's defaults |
| `enabled` | bool | *optional*, default `true`. False bypasses without losing settings |
| `keys` | object | *optional*. Parameter key → *ParamKey*[] (`{at, value, ease}`) for animated parameters |

```json
{"id":"concat.gaussian-blur","params":{"radius":4},"enabled":true}
```

Every parameter is stored as a number, whatever its `type` in the
catalogue: a `bool` is 0 or 1, an `enum` is one of its `values`, a `color`
is packed RGBA, a `point` is two keys `<key>.x` and `<key>.y` in 0..1.

### Transition

| Field | Type | Meaning |
|---|---|---|
| `id` | string | A transition package id, e.g. `"concat.dissolve"`, `"concat.push"`, `"concat.clock-wipe"` |
| `duration` | number | Seconds, default 1 |

### TextStyle

A title's styling. Sizes are **fractions of the frame**, so a title made at
1080p exports right at 4K. In a command you send only the fields you
change; the rest take these defaults.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `content` | string | `"Your text"` | The words, newlines included |
| `fontFamily` | string | `"\"Cabinet Grotesk\""` | CSS-style family name, quoted where needed. May name an added font |
| `fontSize` | number | 0.09 | Cap height as a fraction of frame height |
| `fontWeight` | number | 700 | 100..=900 |
| `italic` | bool | false | |
| `color` | string | `"#ffffff"` | CSS hex |
| `align` | string | `"center"` | `left`, `center`, `right`. Also which point of the block the clip's position pins |
| `opacity` | number | 1 | 0..=1, multiplied with the clip's |
| `strokeWidth` | number | 0 | Outline as a fraction of frame height; 0 is none |
| `strokeColor` | string | `"#000000"` | |
| `shadow` | bool | true | A drop shadow |
| `background` | string | `""` | A plate colour behind the text; empty is none |
| `lineHeight` | number | 1.2 | Multiple of the font size; floored at 0.5 |
| `tracking` | number | 0 | Extra letter spacing, in frame-height fractions |
| `maxWidth` | number | 0 | Wrap width as a fraction of frame width; 0 is no wrap |
| `maxHeight` | number | 0 | Box height as a fraction of frame height; 0 is the words' own |

### Crop

Fractions of the source taken off each edge: `{left, top, right, bottom}`,
each 0..=0.9, with at least a tenth of the picture left.

### Cutout

| Field | Type | Meaning |
|---|---|---|
| `mode` | string | `auto` (the model's mask) or `custom` (the mask plus strokes) |
| `subject` | string | *optional*, default `auto`. `person`, `object`, or `auto` (person if found, else object) |
| `feather` | number | Edge softness as a fraction of picture width |
| `strokes` | *Stroke*[] | *optional*. Corrections, in order |

### Stroke

| Field | Type | Meaning |
|---|---|---|
| `tool` | string | `smartBrush`, `brush`, `smartEraser`, `eraser` |
| `size` | number | Diameter as a fraction of picture width |
| `points` | `[x, y]`[] | Fractions of the source picture, `[0, 0]` top-left |
| `at` | number | *optional*. Source second the stroke was painted at |

### ClipKey

| Field | Type | Meaning |
|---|---|---|
| `property` | string | `scale`, `offsetX`, `offsetY`, `rotation`, `opacity`, `volume` |
| `at` | number | 0..=1 of the clip's length |
| `value` | number | In the property's own units |
| `ease` | `[x1, y1, x2, y2]` | A cubic bezier; default linear `[0, 0, 1, 1]` |

### SpeedPoint

`{at, speed}`: `at` a fraction 0..=1 of the clip, `speed` source seconds
per timeline second there.

### NewMedia

A probed file, as `addMedia`, `fillSlot`, `replaceClipMedia` and
`freezeFrame` take it. The same fields as *MediaItem* minus `id` and
`placeholder`: `path`, `name`, `duration`, `kind`, `width`, `height`,
`frameRate`, `frameRateFraction`, `videoCodec`, `audioCodec`, `hasAudio`,
`audioTracks`, and `origin` (*optional*; leave it out for an import).

> [!TIP]
> Build one from a `media.probe` reply: copy `path`, `duration`, `kind`
> and `audioTracks`; take `width`, `height`, `frameRate`,
> `frameRateFraction` and `videoCodec` from its `video`; `audioCodec`
> from its `audio`; `hasAudio` is whether `audio` is present; `name` is
> the file's basename. Or skip all that with `media.import`.

---

## Media

### MediaSummary

Reply to `media.probe`.

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The file, as given |
| `duration` | number | *optional*. Container duration in seconds |
| `kind` | string | `video`, `audio`, `image` |
| `video` | *VideoStreamInfo* | *optional*. The first video stream |
| `audio` | *AudioStreamInfo* | *optional*. The first audio stream, what a clip plays unless it names another |
| `audioTracks` | *AudioStreamInfo*[] | Every audio stream, in file order |

**VideoStreamInfo:** `index`, `codec`, `width`, `height`, `frameRate`
(decimal), `frameRateFraction` (exact, e.g. `"30/1"`), and `colorRange`
(`limited` or `full`, present only when the file says; a file that says
nothing is played as limited).

**AudioStreamInfo:** `index`, `codec`, `sampleRate`, `channels`, `title`,
`language`.

---

## Catalogue

### PackageInfo

One entry of `catalogue.list`.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `author.name`, e.g. `"concat.gaussian-blur"`. What a chain stores |
| `name` | string | The catalogue card's title |
| `kind` | string | `effect`, `filter`, `audio`, `transition`, `generator` |
| `category` | string | The shelf the card sits on |
| `description` | string | One sentence |
| `intensity` | string | *optional*. The parameter the simple view shows as its one slider |
| `params` | *ParamInfo*[] | In inspector order |

### ParamInfo

| Field | Type | Meaning |
|---|---|---|
| `key` | string | What `params` stores it under |
| `label` | string | The control's label |
| `type` | string | `float`, `int`, `bool`, `enum`, `color`, `point` |
| `min`, `max` | number | The range |
| `default` | number | The value an untouched control means |
| `step` | number | Slider increment; 0 is continuous |
| `unit` | string | Shown after the number |
| `animate` | bool | Can carry keyframes |
| `values` | number[] | For `enum`: the values the document may hold |
| `labels` | string[] | For `enum`: the name of each value, in `values` order |

---

## Templates

### TemplateInfo

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The bundle folder. What `template.instantiate` takes |
| `name` | string | |
| `width`, `height` | integer | Output size |
| `rateNum`, `rateDen` | integer | Frame rate |
| `slots` | *SlotInfo*[] | In the order they first appear on the timeline |
| `hasPoster` | bool | The bundle carries `poster.jpg` |

### SlotInfo

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The placeholder's id. What a fill names |
| `name` | string | As the creator labelled it |
| `kind` | string | `video`, `audio`, `image`: what the slot wants |
| `seconds` | number | Timeline seconds the slot covers |

---

## Events

Every event carries `job` and `path`. As a bare object the tag is `event`;
in the JSON-RPC envelope the tag becomes the notification's `method` and
the rest its `params`.

### `cutout.progress`

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The media being analysed |
| `fetching` | bool | True while a model downloads, false while it runs |
| `fraction` | number | 0..=1 |

### `export.progress`

| Field | Type | Meaning |
|---|---|---|
| `frame` | integer | Frames done |
| `total` | integer | Frames in total |
| `stage` | string | `video`, `audio`, `mux` |

### `export.done`

| Field | Type |
|---|---|
| `output` | string, the file written |
| `width`, `height` | integer |

### `export.failed`

| Field | Type |
|---|---|
| `error` | *ApiError*; `code` is `cancelled` when `export.cancel` stopped it |

---

## Errors

### ApiError

| Field | Type | Meaning |
|---|---|---|
| `code` | string | One of the codes in the [overview](overview.md#errors) |
| `message` | string | The sentence a person would be shown |

In the JSON-RPC envelope: `{"code": <number>, "message": "…", "data": {"code": "<name>"}}`.
