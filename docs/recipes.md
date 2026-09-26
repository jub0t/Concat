# Recipes

**In one line:** copy-paste scripts for the things people most often want
from the API.

Each recipe is shown two ways where it makes sense: as **lines** for
`concat-cli api` (paste into a `.jsonl` file, or type one at a time), and
as **Python** against a socket using the `Concat` class from
[JSON-RPC → Python client](transports/json-rpc.md#a-minimal-python-client).

**Recipes:** [Rough cut from a folder](#1-rough-cut-from-a-folder-of-clips)
· [Export and show progress](#2-export-and-show-progress) ·
[A title card](#3-a-title-card) · [A look on every clip](#4-a-look-on-every-clip)
· [Contact sheet of frames](#5-a-contact-sheet-of-frames) ·
[A project from a template](#6-a-project-from-a-template) ·
[Vertical cut of a landscape edit](#7-a-vertical-cut-of-a-landscape-edit) ·
[Undo, redo, save](#8-undo-redo-save) · [Inspect a project](#9-inspect-a-project-without-changing-it)

Run the CLI from the `src/` folder: `cargo run -p concat-cli -- api < file.jsonl`.

---

## 1. Rough cut from a folder of clips

Import every file and lay them end to end on the first track.

**Lines** (two files shown; repeat the pattern):

```jsonl
{"jsonrpc":"2.0","id":1,"method":"project.create","params":{"location":"/edits","name":"Rough cut"}}
{"jsonrpc":"2.0","id":2,"method":"media.import","params":{"path":"/edits/Rough cut","file":"/footage/a.mp4"}}
{"jsonrpc":"2.0","id":3,"method":"media.import","params":{"path":"/edits/Rough cut","file":"/footage/b.mp4"}}
{"jsonrpc":"2.0","id":4,"method":"edit.apply","params":{"path":"/edits/Rough cut","command":{"op":"addClip","mediaId":"m1","trackId":"T1","start":0}}}
{"jsonrpc":"2.0","id":5,"method":"project.get","params":{"path":"/edits/Rough cut"}}
```

Read the clip's `duration` from the `project.get` reply, then place the
next clip at that `start`. Python does the bookkeeping:

```python
import glob, os
c = Concat(token=TOKEN)
folder = "/edits/Rough cut"
c.call("project.create", location="/edits", name="Rough cut")

at = 0.0
for file in sorted(glob.glob("/footage/*.mp4")):
    view = c.call("media.import", path=folder, file=file)
    media_id = view["createdId"]
    view = c.call("edit.apply", path=folder,
                  command={"op": "addClip", "mediaId": media_id, "trackId": "T1", "start": at})
    clip = next(cl for cl in view["project"]["timelines"][0]["clips"] if cl["id"] == view["createdId"])
    at += clip["duration"]

c.call("project.save", path=folder)
```

> [!TIP]
> `media.import` on a file already in the bin is a no-op and returns no
> `createdId`. Look the id up in `project.media` by `path` if you re-run.

---

## 2. Export and show progress

**Lines:**

```jsonl
{"jsonrpc":"2.0","id":1,"method":"project.open","params":{"path":"/edits/Reel"}}
{"jsonrpc":"2.0","id":2,"method":"export.run","params":{"path":"/edits/Reel","output":"/edits/reel.mp4","crf":18,"preset":"slow","codec":"h264"}}
```

The CLI keeps running until the job ends, printing each event.

**Python**, with a progress line:

```python
def show(event):
    p = event["params"]
    if event["method"] == "export.progress":
        print(f'\r{p["stage"]:5} {p["frame"]}/{p["total"]}', end="")
    elif event["method"] == "cutout.progress":
        print(f'\rcutout {p["mediaId"]} {"fetching" if p["fetching"] else "analysing"} {p["fraction"]:.0%}', end="")

c = Concat(token=TOKEN, on_event=show)
c.call("project.open", path="/edits/Reel")
started = c.call("export.run", path="/edits/Reel", output="/edits/reel.mp4", crf=18)
end = c.wait_for_job(started["job"])
print()
if end["method"] == "export.done":
    print("wrote", end["params"]["output"])
else:
    print("failed:", end["params"]["error"]["message"])
```

To stop it from another connection:

```json
{"jsonrpc":"2.0","id":9,"method":"export.cancel","params":{"job":"j1"}}
```

The job then ends with `export.failed` and `error.code` = `cancelled`.

---

## 3. A title card

A three-second title on its own lane above the footage, then a lower
third.

```jsonl
{"jsonrpc":"2.0","id":1,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addTextClip","above":true,"start":0,"duration":3,"style":{"content":"Summer 2026","fontSize":0.14}}}}
{"jsonrpc":"2.0","id":2,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addTextClip","above":true,"start":5,"duration":4,"offsetY":0.35,"style":{"content":"Ada Lovelace\nEngine designer","fontSize":0.05,"align":"left","background":"#00000099","maxWidth":0.5}}}}
```

Only the style fields you set change; the rest are the window's defaults
(white, bold, centred, with a shadow). All of them are in
[Types → TextStyle](api/types.md#textstyle).

Change a title later with a patch. Absent fields stay, `null` clears:

```json
{"op":"updateClip","clipId":"c7","patch":{"text":{"content":"Summer 2026","color":"#ffcc00"}}}
```

---

## 4. A look on every clip

Find a filter, then set it on each video clip.

```python
looks = c.call("catalogue.list", kind="filter")
print([p["id"] for p in looks])            # e.g. concat.warm, concat.film, concat.mono …

view = c.call("project.get", path="/edits/Reel")
timeline = next(t for t in view["project"]["timelines"] if t["id"] == view["project"]["activeTimelineId"])
commands = [
    {"op": "updateClip", "clipId": clip["id"],
     "patch": {"videoEffects": clip["videoEffects"] + [{"id": "concat.film", "params": {}}]}}
    for clip in timeline["clips"] if clip["kind"] == "video"
]
c.call("edit.apply", path="/edits/Reel", command={"op": "batch", "commands": commands})
```

Notes:

- `videoEffects` in a patch **replaces** the chain, so append to what the
  clip has.
- Empty `params` means the package's defaults. The keys and ranges are in
  the `catalogue.list` reply.
- One `batch` means one undo step for the whole pass.

The same over everything at once, as a layer instead of per clip:

```json
{"op":"addLayerClip","start":0,"duration":60,"effectId":"concat.film","name":"Film look"}
```

---

## 5. A contact sheet of frames

One PNG a second, to a folder.

```python
view = c.call("project.open", path="/edits/Reel")
timeline = next(t for t in view["project"]["timelines"] if t["id"] == view["project"]["activeTimelineId"])
length = max((cl["start"] + cl["duration"] for cl in timeline["clips"]), default=0)

t = 0.0
while t < length:
    c.call("preview.frame", path="/edits/Reel", time=t, output=f"/edits/frames/{t:06.1f}.png", width=320, height=180)
    t += 1.0
```

Or one frame straight into memory:

```python
import base64
pic = c.call("preview.frame", path="/edits/Reel", time=2.5)
png_bytes = base64.b64decode(pic["png"])
```

---

## 6. A project from a template

```python
templates = c.call("template.list")
intro = next(t for t in templates if t["name"] == "Intro")
for slot in intro["slots"]:
    print(slot["mediaId"], slot["name"], slot["kind"], f'{slot["seconds"]:.1f}s')

fills = [
    {"mediaId": intro["slots"][0]["mediaId"], "file": "/footage/logo.png"},
    {"mediaId": intro["slots"][1]["mediaId"], "file": "/footage/take1.mp4"},
]
view = c.call("template.instantiate", template=intro["path"], location="/edits", name="My intro", fills=fills)
```

Every slot must be filled, and every file is probed before anything is
made. To make a template from a project you have open:

```json
{"jsonrpc":"2.0","id":1,"method":"template.save","params":{"path":"/edits/Reel","name":"Reel template"}}
```

---

## 7. A vertical cut of a landscape edit

A second timeline in the same project, 1080×1920, with the same clips
re-framed.

```jsonl
{"jsonrpc":"2.0","id":1,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addTimeline"}}}
{"jsonrpc":"2.0","id":2,"method":"project.setVideo","params":{"path":"/edits/Reel","video":{"width":1080,"height":1920,"rateNum":30,"rateDen":1}}}
{"jsonrpc":"2.0","id":3,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addClipAtFirstFree","mediaId":"m1","start":0}}}
{"jsonrpc":"2.0","id":4,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"setClipTransform","clipId":"c9","scale":1.8,"offsetX":-0.1}}}
{"jsonrpc":"2.0","id":5,"method":"export.run","params":{"path":"/edits/Reel","output":"/edits/reel-vertical.mp4"}}
```

`addTimeline` makes the new timeline active, so the `project.setVideo`
and the edits that follow land on it. Switch back with
`{"op":"selectTimeline","timelineId":"TL1"}`. The clip id `c9` above is
whatever `createdId` came back from the add.

---

## 8. Undo, redo, save

```jsonl
{"jsonrpc":"2.0","id":1,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"removeClips","clipIds":["c2"],"ripple":true}}}
{"jsonrpc":"2.0","id":2,"method":"edit.undo","params":{"path":"/edits/Reel"}}
{"jsonrpc":"2.0","id":3,"method":"edit.redo","params":{"path":"/edits/Reel"}}
{"jsonrpc":"2.0","id":4,"method":"project.save","params":{"path":"/edits/Reel"}}
{"jsonrpc":"2.0","id":5,"method":"project.close","params":{"path":"/edits/Reel"}}
```

Every reply's `canUndo` / `canRedo` tells you where the history stands.
A tolerated no-op (an unknown id, a value already held) records no undo
step.

---

## 9. Inspect a project without changing it

```sh
cd src
cargo run -p concat-cli -- api '{"method":"project.open","path":"/edits/Reel"}' | python3 -c '
import json, sys
view = json.loads(sys.stdin.readline())["result"]
p = view["project"]
print(view["settings"])
for t in p["timelines"]:
    print(t["id"], t["name"], f'{t["video"]["width"]}x{t["video"]["height"]}')
    for clip in sorted(t["clips"], key=lambda c: c["start"]):
        print(f'  {clip["id"]:5} {clip["kind"]:5} {clip["start"]:7.2f} +{clip["duration"]:6.2f}  {clip["name"]}')
'
```

Opening does not change the project document; nothing in the folder is
written unless you call `project.save`. The only side effect is the
recents list, which moves the project to the front.
