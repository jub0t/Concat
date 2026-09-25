# Edit commands

**In one line:** every edit the window can make, as a JSON object you send
through `edit.apply`.

```json
{"method":"edit.apply","path":"/edits/Reel","command":{"op":"trimClip","clipId":"c2","edge":"end","delta":-1.5}}
```

A command is `{"op": "<name>", …fields}`. Fields are camelCase. The reply
is an *EditorView* ([Types](types.md#editorview)) of the project after the
edit; `createdId` names what the command made, if anything.

## Cheat sheet

| Op | Does | Makes |
|---|---|---|
| **Media** | | |
| [`addMedia`](#addmedia) | file into the bin | `m…` |
| [`removeMedia`](#removemedia) | bin item and every clip of it, out | |
| [`updateMediaPath`](#updatemediapath) | relink a file | |
| [`setMediaColorRange`](#setmediacolorrange) | say whether a file is really limited or full range | |
| [`replaceClipMedia`](#replaceclipmedia) | point a clip at another file | `m…` if new |
| **Placing clips** | | |
| [`addClip`](#addclip) | media on a named track | `c…` |
| [`addClipAtFirstFree`](#addclipatfirstfree) | media on the first free track | `c…` |
| [`addTextClip`](#addtextclip) | a title | `c…` |
| [`addLayerClip`](#addlayerclip) | an effect layer over everything beneath | `c…` |
| [`freezeFrame`](#freezeframe) | a held still, cut into a clip | `c…` |
| **Moving and cutting** | | |
| [`moveClips`](#moveclips) | reposition any number of clips | |
| [`trimClip`](#trimclip) | drag one edge | |
| [`splitClips`](#splitclips) | cut at a time | `c…` (the tail) |
| [`mergeClips`](#mergeclips) | rejoin split pieces | |
| [`removeClips`](#removeclips) | delete | |
| **Changing a clip** | | |
| [`updateClip`](#updateclip) | patch: name, volume, fades, opacity, effects, transition, text, crop… | |
| [`setClipTransform`](#setcliptransform) | scale, offset, rotation, stretch | |
| [`setClipSpeed`](#setclipspeed) | playback rate | |
| [`setClipSpeedCurve`](#setclipspeedcurve) | speed over time | |
| [`setClipKey`](#setclipkey) · [`clearClipKey`](#clearclipkey) · [`clearClipKeys`](#clearclipkeys) | keyframes on scale, offset, rotation, opacity, volume | |
| [`setEffectKey`](#seteffectkey) · [`clearEffectKey`](#cleareffectkey) · [`clearEffectKeys`](#cleareffectkeys) | keyframes on an effect parameter | |
| [`setClipCutout`](#setclipcutout) · [`addCutoutStroke`](#addcutoutstroke) | background removal and brush corrections | |
| [`detachAudio`](#detachaudio) · [`reattachAudio`](#reattachaudio) | a video's sound as its own clip, and back | `c…` |
| **Tracks** | | |
| [`addTrack`](#addtrack) | a new lane | `t…` |
| [`removeTrack`](#removetrack) | a lane and its clips, out | |
| [`setTrackFlag`](#settrackflag) | visible / muted | |
| **Timelines** | | |
| [`addTimeline`](#addtimeline) | a new timeline, made active | `tl…` |
| [`removeTimeline`](#removetimeline) · [`renameTimeline`](#renametimeline) · [`selectTimeline`](#selecttimeline) · [`moveTimeline`](#movetimeline) · [`setTimelineVideo`](#settimelinevideo) | manage timelines | |
| **Templates and fonts** | | |
| [`setMediaPlaceholder`](#setmediaplaceholder) · [`fillSlot`](#fillslot) | template slots | |
| [`addFont`](#addfont) · [`removeFont`](#removefont) | fonts for titles | |
| **Grouping** | | |
| [`batch`](#batch) | several commands as one undo step | last id inside |

## Rules that apply to every command

- **Ids.** Clips are `c…`, tracks `t…` (the first timeline's starter lanes
  are `T1`–`T4`), timelines `tl…` (the first is `TL1`), media `m…`. Ids
  come from the *EditorView* replies.
- **Unknown ids** are usually a tolerated no-op: the request succeeds,
  nothing changes, no undo step. Where a command errs instead, its entry
  says so.
- **Times** are seconds on the timeline. A start below 0 lands at 0.
- **Numbers are clamped**, not refused, into the range each field states.
  A NaN or infinite number is refused: "A number in that edit is not finite."
- **A refusal** is a `refused` error whose message is the exact sentence
  the window would show. The project is as it was.
- Commands act on the **active timeline**, except where the entry says
  "all timelines".

## Refusal sentences

| Sentence | Raised by |
|---|---|
| That media is no longer in the bin. | a clip-placing command whose media is gone |
| That track no longer exists. | a clip-placing command whose track is gone |
| There are no tracks. | first-free placement on a timeline with no tracks |
| A timeline needs at least one track. | `removeTrack` on the last track |
| A project needs at least one timeline. | `removeTimeline` on the last timeline |
| That template slot no longer exists. | `fillSlot` with an unknown id |
| That media is not a template slot. | `fillSlot` on ordinary media |
| A number in that edit is not finite. | any command carrying NaN or infinity |
| *(one of several sentences)* | `mergeClips`, saying why the pieces cannot be rejoined |

---

## Media

### `addMedia`

Imports a probed file into the bin, minting an `m` id.

| Field | Type | Meaning |
|---|---|---|
| `item` | *NewMedia* | The file as `media.probe` describes it (see [Types](types.md#newmedia)) |

> [!TIP]
> You rarely need this. `media.import` probes and adds in one request.
> Use `addMedia` when you already have the probe, or inside a `batch`.

A path already in the bin is a no-op that mints nothing.

### `removeMedia`

Removes a bin item and **every clip referencing it, on all timelines**.

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The bin item. Unknown id: no-op |

### `updateMediaPath`

Relinks a bin item to a new path on disk.

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The bin item. Unknown id: no-op |
| `newPath` | string | The new absolute path |

### `setMediaColorRange`

Says what levels a media file's picture really spans, over whatever the
file claims. The fix for a washed-out or crushed picture.

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The bin item. Unknown id: no-op |
| `range` | `"limited"`, `"full"` or `null` | `limited` is 16-235, `full` is 0-255. `null` goes back to reading the file's own tag |

```json
{"op":"setMediaColorRange","mediaId":"m1","range":"full"}
```

When to use which:

- A screen recording that plays **grey where it should be black** is a
  full-range file tagged nothing. Set `full`.
- A file whose **shadows are crushed** and highlights clipped is a
  video-range file tagged full. Set `limited`.
- `media.probe` reports what the file claims as `video.colorRange`.

Reaches every clip of the media, on every timeline, in the monitor and
the export alike. Stored in the document; absent means "as tagged".

### `replaceClipMedia`

Points a clip at another file, adding it to the bin first if it is not
there. The clip keeps its length, looks and name, and its in-point unless
`sourceStart` moves it: an enhanced copy stands in frame for frame, a
reversed span starts at its own zero. The bin keeps the original.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | The clip. Unknown id: no-op |
| `item` | *NewMedia* | The probed file |
| `sourceStart` | number | *optional*. A new in-point in the copy, in seconds |

---

## Placing clips

### `addClip`

Places a clip of a bin item on a named track.

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The bin item. Gone: **refused** |
| `trackId` | string | The lane. Gone: **refused** |
| `start` | number | Seconds; floored at 0 |
| `ripple` | bool | *optional*, default `false`. When the drop would overlap, shift every clip at or after `start` right by the new clip's length |

Duration comes from the media: its own length for video and audio, 5 s
for a still or a file with no reported length.

```json
{"op":"addClip","mediaId":"m1","trackId":"T1","start":2.5}
```

### `addClipAtFirstFree`

`addClip` without naming a lane: lands on the lowest track with nothing in
the clip's span, or on the bottom track (overlapping) rather than refusing.

| Field | Type | Meaning |
|---|---|---|
| `mediaId` | string | The bin item. Gone: **refused** |
| `start` | number | Seconds; floored at 0 |

### `addTextClip`

Places a title: a clip with no media behind it, named after the text's
first line.

| Field | Type | Meaning |
|---|---|---|
| `trackId` | string | *optional*. The lane. Absent picks a free track; a vanished track is **refused** |
| `above` | bool | *optional*, default `false`. With no `trackId`: land on the first free lane *above* the highest occupied one, minting a lane at the top if needed. What the editor does for its own titles and captions |
| `start` | number | Seconds; floored at 0 |
| `style` | *TextStyle* | *optional*. Only the fields you set; the rest are the window's defaults. `{"content": "Hello"}` is enough |
| `duration` | number | *optional*, default 4 s |
| `offsetY` | number | *optional*. Vertical placement as a fraction of frame height, clamped to ±3. Lower thirds are made of this |

```json
{"op":"addTextClip","start":1,"duration":3,"style":{"content":"Chapter one","fontSize":0.12,"color":"#ffcc00"},"offsetY":0.35}
```

### `addLayerClip`

Places a layer: an effect over a span of the timeline that treats
everything beneath it. The chain starts as the one package at its
defaults; the clip's `opacity` is how hard it is applied.

| Field | Type | Meaning |
|---|---|---|
| `trackId` | string | *optional*. Absent picks the first free track |
| `start` | number | Seconds; floored at 0 |
| `duration` | number | *optional*, default 5 s |
| `effectId` | string | A package id from `catalogue.list`, e.g. `concat.warm` |
| `name` | string | What the lane calls it |

### `freezeFrame`

Splits a clip at `time`, inserts a held still, and ripples later clips on
that track by the hold's length. `createdId` is the freeze clip.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | A picture clip under the playhead |
| `time` | number | Must fall strictly inside the clip |
| `duration` | number | *optional*, default 1 s; floored at 1/60 s |
| `still` | *NewMedia* | The probed still (a JPEG of the frame). **Required for video**; an image clip may omit it and reuse its media |

Audio and text clips: no-op.

---

## Moving and cutting

### `moveClips`

Repositions any number of clips as one undo step.

| Field | Type | Meaning |
|---|---|---|
| `moves` | array of `{clipId, start, trackId}` | Where each clip goes. `start` floors at 0. Unknown `clipId`: that move is skipped. Unknown `trackId`: moved in time, kept on its track |

```json
{"op":"moveClips","moves":[{"clipId":"c2","start":0,"trackId":"T1"},{"clipId":"c3","start":6.1,"trackId":"T1"}]}
```

### `trimClip`

Drags one edge of a clip.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `edge` | `"start"` or `"end"` | Which edge |
| `delta` | number | Signed timeline seconds. Positive drags the head right (shortening) or the tail right (lengthening) |
| `ripple` | bool | *optional*, default `false`. Close the lane up behind the trim: later clips on the track move by the change in length |

How the edges differ:

- **`start`** moves the in-point with the edge (scaled by speed), so the
  remaining frames stay where they were.
- **`end`** only lengthens or shortens.
- Either edge stops at the 1/60 s minimum duration.

### `splitClips`

Cuts each named clip in two at one time.

| Field | Type | Meaning |
|---|---|---|
| `clipIds` | string[] | The clips under the playhead |
| `time` | number | The cut point, in timeline seconds |

- The head keeps the id and the transition; the tail is minted fresh and
  stays source-continuous.
- A clip the time misses, or grazes within 1/60 s of an edge, is skipped.

### `mergeClips`

Rejoins split pieces into the earliest piece, which keeps its id.

| Field | Type | Meaning |
|---|---|---|
| `clipIds` | string[] | The pieces, any order |

**Refused** unless the pieces sit on one track, come from one file at one
speed, touch within a microsecond, and are in source order. The message
says which condition failed.

### `removeClips`

Deletes clips from the active timeline.

| Field | Type | Meaning |
|---|---|---|
| `clipIds` | string[] | Unknown ids are ignored |
| `ripple` | bool | *optional*, default `false`. Leave no gap: on each touched track, later clips move left by the removed spans before them. Untouched tracks stay put |

---

## Changing a clip

### `updateClip`

Applies a patch: only the fields present change.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `patch` | *ClipPatch* | The fields to change |

*ClipPatch* fields (all optional):

| Field | Type | Clamp / meaning |
|---|---|---|
| `name` | string | Taken verbatim |
| `volume` | number | Floored at 0; **not** capped at 1 |
| `fadeIn`, `fadeOut` | number | Seconds, floored at 0 |
| `opacity` | number | 0..=1 |
| `preservePitch` | bool | Keep voices at pitch when speed ≠ 1 |
| `muted` | bool | Silence the clip's own sound |
| `flipH`, `flipV` | bool | Mirror |
| `blend` | string | `normal` (or empty), `multiply`, `screen`, `add`, `lighten`, `darken` |
| `filters` | *AppliedFilter*[] | **Replaces** the whole audio chain |
| `videoEffects` | *AppliedFilter*[] | **Replaces** the whole video chain |
| `crop` | *Crop* or `null` | See below |
| `transitionIn` | *Transition* or `null` | The transition on the cut into the clip |
| `text` | *TextStyle* or `null` | Title styling; also renames the clip after its first line |
| `audioStream` | integer or `null` | Which of the media's audio streams to play; `null` means the file's first |

> [!IMPORTANT]
> `crop`, `transitionIn`, `text` and `audioStream` have three states:
> **absent** leaves the value alone, **`null`** clears it, a **value**
> replaces it.

```json
{"op":"updateClip","clipId":"c2","patch":{"volume":0.5,"fadeIn":0.5,"videoEffects":[{"id":"concat.gaussian-blur","params":{"radius":4}}],"transitionIn":{"id":"concat.dissolve","duration":0.75}}}
```

### `setClipTransform`

Places the picture. Send only the fields you moved.

| Field | Type | Clamp |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `scale` | number | 0.05..=8 |
| `offsetX` | number | Fraction of frame width, ±3 |
| `offsetY` | number | Fraction of frame height, ±3 |
| `rotation` | number | Degrees, wrapped into (-180, 180] |
| `stretchX`, `stretchY` | number | 0.1..=10, a multiplier beyond `scale` |

### `setClipSpeed`

Changes the playback rate while holding the source covered constant: the
timeline length is what stretches.

| Field | Type | Clamp |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `speed` | number | 0.0625..=16 |

### `setClipSpeedCurve`

Speed that changes over the clip.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | The clip |
| `curve` | *SpeedPoint*[] or `null` | Points of `{at, speed}`, `at` a fraction 0..=1 of the clip. `null` returns to a constant rate at the current mean |

### `setClipKey`

Puts a keyframe on one property at one point of the clip.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | The clip |
| `property` | `"scale"`, `"offsetX"`, `"offsetY"`, `"rotation"`, `"opacity"`, `"volume"` | Which property |
| `at` | number | Where in the clip, 0..=1 |
| `value` | number | The value there, in the property's own units |
| `ease` | `[x1, y1, x2, y2]` | *optional*, default linear. A CSS-style cubic bezier; the names `"linear"`, `"in"`, `"out"`, `"inOut"` are accepted too |

A key within 0.002 of `at` on the same property is replaced.

### `clearClipKey`

Removes the key at `at` on one property, if there is one.

| Field | Type |
|---|---|
| `clipId` | string |
| `property` | as above |
| `at` | number, 0..=1 |

### `clearClipKeys`

Removes every key on one property.

| Field | Type |
|---|---|
| `clipId` | string |
| `property` | as above |

### `setEffectKey`

A keyframe on one parameter of one link in the clip's video chain.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | The clip |
| `entry` | integer | Index into `videoEffects`. Out of range: no-op |
| `key` | string | The parameter's key from `catalogue.list` |
| `at` | number | 0..=1 |
| `value` | number | The parameter's own value; keep it within the package's range yourself |
| `ease` | `[x1, y1, x2, y2]` | *optional*, default linear |

### `clearEffectKey`

Removes the key at `at` on one parameter of one link.

| Field | Type |
|---|---|
| `clipId` | string |
| `entry` | integer |
| `key` | string |
| `at` | number |

### `clearEffectKeys`

Removes every key on one parameter of one link.

| Field | Type |
|---|---|
| `clipId` | string |
| `entry` | integer |
| `key` | string |

### `setClipCutout`

Sets or clears a picture's cutout: the mask that removes its background.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `cutout` | *Cutout* or `null` | `{"mode": "auto"}` is the usual start; `null` takes it off |

*Cutout* fields: `mode` (`auto` or `custom`), `subject` (`auto`, `person`,
`object`; default `auto`), `feather` (edge softness as a fraction of the
picture's width), `strokes` (corrections, see next).

Masks are found at export time; `export.run` reports it as
`cutout.progress`.

### `addCutoutStroke`

Paints one brush stroke onto a clip's cutout. A clip with no cutout gets a
custom one; an automatic cutout becomes custom.

| Field | Type | Meaning |
|---|---|---|
| `clipId` | string | Unknown id: no-op |
| `stroke` | *Stroke* | `tool` (`smartBrush`, `brush`, `smartEraser`, `eraser`), `size` (diameter as a fraction of picture width), `points` (`[[x, y], …]` as fractions of the source picture), `at` (*optional* source second the stroke was painted at) |

A stroke with no points is a no-op.

### `detachAudio`

Pulls a video clip's sound out into its own audio clip on a free lane,
muting the video and moving its audio filters to the sound.

| Field | Type |
|---|---|
| `clipId` | string, the video clip |

No-op unless the clip is an unmuted video whose media has audio and is not
already detached.

### `reattachAudio`

Undoes a detach: deletes the detached sound, unmutes the video, hands the
filters back.

| Field | Type |
|---|---|
| `clipId` | string, the video **or** its detached sound |

---

## Tracks

### `addTrack`

Appends a lane, named after the highest "Track N" in use. No fields.

### `removeTrack`

Deletes a lane and every clip on it.

| Field | Type |
|---|---|
| `trackId` | string |

**Refused** on the last track.

### `setTrackFlag`

Flips one of a track's two toggles.

| Field | Type | Meaning |
|---|---|---|
| `trackId` | string | Unknown id: no-op |
| `flag` | `"visible"` or `"muted"` | Which toggle |
| `value` | bool | The new setting |

---

## Timelines

### `addTimeline`

Adds a fresh timeline with four lanes and makes it active. No fields.

### `removeTimeline`

Deletes a timeline; the active tab moves to a neighbour if it was this one.

| Field | Type |
|---|---|
| `timelineId` | string. Unknown id: no-op |

**Refused** on the last timeline.

### `setTimelineVideo`

Sets a timeline's output frame and rate.

| Field | Type |
|---|---|
| `timelineId` | string. Unknown id: no-op |
| `video` | *VideoSettings* |

A zero dimension or rate is a no-op, not clamped.

### `renameTimeline`

| Field | Type |
|---|---|
| `timelineId` | string |
| `name` | string, trimmed. Whitespace-only is ignored |

### `selectTimeline`

Switches which timeline later commands act on.

| Field | Type |
|---|---|
| `timelineId` | string. Unknown id: selection stays |

### `moveTimeline`

Moves a timeline tab to a new position.

| Field | Type |
|---|---|
| `timelineId` | string |
| `index` | integer, 0-based, counted with the tab already removed; clamped to the end |

---

## Templates and fonts

### `setMediaPlaceholder`

Marks a bin item as a template slot, or back to ordinary media.

| Field | Type |
|---|---|
| `mediaId` | string. Unknown id: no-op |
| `placeholder` | bool |

### `fillSlot`

Swaps a file into a template slot in place, on all timelines. The slot
keeps its id, so clips keep working.

| Field | Type |
|---|---|
| `mediaId` | string, a placeholder |
| `item` | *NewMedia* |

**Refused** if the id is unknown or the item is not a placeholder.
`template.instantiate` does this for every slot for you.

### `addFont`

Registers a font file for titles.

| Field | Type |
|---|---|
| `family` | string, the name titles refer to |
| `path` | string, the font file |

A path already registered is a no-op.

### `removeFont`

| Field | Type |
|---|---|
| `family` | string |

Clips keep the family name; the face returns when the file does.

---

## Grouping

### `batch`

Several commands as one atomic edit and one undo step.

| Field | Type |
|---|---|
| `commands` | *Command*[], applied in order. Nesting is allowed |

- Applied to a staged copy; committed only if **every** command succeeds.
- `createdId` in the reply is the last id minted inside.

```json
{"op":"batch","commands":[
  {"op":"addTrack"},
  {"op":"addTextClip","start":0,"duration":2,"style":{"content":"One"}},
  {"op":"addTextClip","start":2,"duration":2,"style":{"content":"Two"}}
]}
```

> [!NOTE]
> A batch cannot refer to an id it mints, because the id is not known
> until the batch is applied. Use `addClipAtFirstFree` / `addTextClip`
> with `duration` rather than add-then-trim.
