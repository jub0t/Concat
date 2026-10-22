#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Detect a CapCut draft folder and convert it to a Concat project (concat.json).

    python3 scripts/capcut_to_concat.py <folder>            # writes <folder>/concat.json
    python3 scripts/capcut_to_concat.py <folder> --out DIR  # writes DIR/concat.json instead
    python3 scripts/capcut_to_concat.py <folder> --dry-run  # converts, prints, writes nothing

A CapCut draft is a folder holding `draft_content.json`. A Concat project is a
folder holding `concat.json`. This script reads the first and writes the
second; it never modifies or deletes any CapCut file.

The mapping is documented in CAPCUT_IMPORT.md. This file is also the test
oracle for the Rust importer in crates/concat-host/src/capcut.rs: both follow
the same rules, so a change to one is a change to the other.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import re
import sys
import tempfile
from pathlib import Path

log = logging.getLogger("capcut2concat")

US = 1_000_000.0  # CapCut times are microseconds; Concat's are seconds.

# Ranges Concat holds every clip number to (concat-project model::ranges).
TOUCH = 1e-6
MIN_CLIP = 1.0 / 60.0
MIN_SPEED, MAX_SPEED = 0.0625, 16.0
MIN_SCALE, MAX_SCALE = 0.05, 8.0
MIN_STRETCH, MAX_STRETCH = 0.1, 10.0
MAX_OFFSET = 3.0
MIN_TRANSITION = 0.1

# CapCut effect name (lower case) -> Concat catalogue id. Only pairs that were
# checked against the catalogue are listed; anything else is skipped with a warning.
EFFECT_MAP = {"blur": "concat.gaussian-blur"}
# A CapCut transition Concat has no package for becomes a dissolve of the same length.
FALLBACK_TRANSITION = "concat.dissolve"
# CapCut's text size unit is unverified. 15 is CapCut's default title size and
# Concat's default is 0.09 of the frame height, so scale linearly around that.
TEXT_SIZE_UNIT = 0.09 / 15.0

DRAFT_FILE = "draft_content.json"
META_FILE = "draft_meta_info.json"
MANIFESTS = ("concat.json", "wolfcut.json")


# ---------------------------------------------------------------- detection


def is_concat_project(folder: Path) -> bool:
    return any((folder / name).is_file() for name in MANIFESTS)


def is_capcut_draft(folder: Path) -> bool:
    return (folder / DRAFT_FILE).is_file()


# ----------------------------------------------------------------- helpers


def clamp(value: float, low: float, high: float) -> float:
    return max(low, min(high, value))


def seconds(us) -> float:
    return float(us or 0) / US


def wrap_rotation(degrees: float) -> float:
    wrapped = ((degrees % 360.0) + 540.0) % 360.0 - 180.0
    return 180.0 if wrapped == -180.0 else wrapped


def fps_to_rate(fps: float) -> tuple[int, int]:
    for num in (24000, 30000, 60000):  # NTSC rates are fractions, not decimals
        if abs(fps - num / 1001.0) < 0.01:
            return num, 1001
    if abs(fps - round(fps)) < 1e-6:
        return int(round(fps)), 1
    return int(round(fps * 1000)), 1000


def text_of(content: str) -> str:
    """The words of a CapCut text material.

    CapCut 1.5.x stores `<size=15><font ...>[words]</font></size>`; other
    builds store a JSON object with a `text` field. Both are read.
    """
    stripped = content.strip()
    if stripped.startswith("{"):
        try:
            text = json.loads(stripped).get("text")
            if isinstance(text, str):
                return text.strip()
        except ValueError:
            pass
    plain = re.sub(r"<[^>]*>", "", content)
    opened, closed = plain.find("["), plain.rfind("]")
    if 0 <= opened < closed:
        plain = plain[opened + 1 : closed]
    return plain.strip()


# ------------------------------------------------------------------ convert


def convert(draft: dict, meta: dict | None, fallback_name: str):
    """Returns (concat document, warnings)."""
    warnings: list[str] = []

    def warn(message: str) -> None:
        warnings.append(message)
        log.warning(message)

    materials = draft.get("materials", {})
    by_id: dict[str, tuple[str, dict]] = {}
    for kind, entries in materials.items():
        if isinstance(entries, list):
            for entry in entries:
                if isinstance(entry, dict) and "id" in entry:
                    by_id[entry["id"]] = (kind, entry)

    canvas = draft.get("canvas_config", {})
    width = int(canvas.get("width") or 1920)
    height = int(canvas.get("height") or 1080)
    rate_num, rate_den = fps_to_rate(float(draft.get("fps") or 30.0))
    name = (meta or {}).get("draft_name") or draft.get("name") or fallback_name

    # ---- media bin: one item per distinct file, like Concat's own duplicate check
    media: list[dict] = []
    media_of: dict[str, str] = {}  # capcut material id -> concat media id
    by_path: dict[str, str] = {}

    def add_media(material: dict, kind: str) -> None:
        path = (material.get("path") or "").replace("\\", "/")
        key = path or material["id"]
        if key not in by_path:
            item = {
                "id": f"m{len(media) + 1}",
                "path": path,
                "name": material.get("material_name") or material.get("name") or Path(path).name or key,
                "kind": kind,
                "hasAudio": bool(material.get("has_audio", kind == "audio")),
            }
            if kind != "image" and material.get("duration"):
                item["duration"] = seconds(material["duration"])
            if kind != "audio" and material.get("width") and material.get("height"):
                item["width"], item["height"] = int(material["width"]), int(material["height"])
            if not path:
                warn(f"media '{item['name']}' has no file path; relink it in Concat")
            media.append(item)
            by_path[key] = item["id"]
        media_of[material["id"]] = by_path[key]

    for material in materials.get("videos", []):
        add_media(material, "image" if material.get("type") == "photo" else "video")
    for material in materials.get("audios", []):
        add_media(material, "audio")

    # ---- tracks -> lanes. CapCut's first track is the bottom of the stack and
    # so is Concat's first lane (Timeline::background_lane), so order is kept.
    lanes: list[list[dict]] = []  # each lane: clips in time order
    for index, track in enumerate(draft.get("tracks", [])):
        kind = track.get("type")
        if kind not in ("video", "audio", "text", "effect"):
            warn(f"track {index + 1} of type '{kind}' is not supported; skipped "
                 f"({len(track.get('segments', []))} segment(s))")
            continue
        segments = sorted(track.get("segments", []), key=lambda s: s["target_timerange"]["start"])
        clips: list[dict] = []
        for segment in segments:
            clip = segment_to_clip(segment, kind, by_id, media_of, warn)
            if clip is not None:
                clips.append(clip)
        apply_transitions(clips, warn)
        lanes.extend(place_without_overlap(clips, warn))
    if not lanes:
        raise ValueError("the draft has no track this importer can read")

    # ---- ids, last, once the order is final
    tracks, flat_clips = [], []
    for number, lane in enumerate(lanes, start=1):
        track_id = f"T{number}"
        tracks.append({"id": track_id, "visible": True, "muted": False})
        for clip in lane:
            clip["trackId"] = track_id
            clip["id"] = f"c{len(flat_clips) + 1}"
            for private in [key for key in clip if key.startswith("_")]:
                del clip[private]
            flat_clips.append(clip)

    video = {"width": width, "height": height, "rateNum": rate_num, "rateDen": rate_den}
    timeline = {"id": "TL1", "name": "Timeline 1", "video": video, "tracks": tracks, "clips": flat_clips}
    document = {
        "concat": "0.1.0",
        "version": 1,
        "name": name,
        "video": video,
        "media": media,
        "tracks": tracks,
        "clips": flat_clips,
        "fonts": [],
        "timelines": [timeline],
        "activeTimelineId": "TL1",
    }
    return document, warnings


def placement(segment: dict, warn) -> dict:
    """Timing, speed and transform shared by every segment kind."""
    target, source = segment["target_timerange"], segment.get("source_timerange") or {}
    speed = float(segment.get("speed") or 1.0)
    if not MIN_SPEED <= speed <= MAX_SPEED:
        clamped = clamp(speed, MIN_SPEED, MAX_SPEED)
        warn(f"speed {speed:g}x is outside Concat's {MIN_SPEED:g}-{MAX_SPEED:g}x range; using {clamped:g}x")
        speed = clamped
    clip = segment.get("clip") or {}
    scale = clip.get("scale") or {}
    transform = clip.get("transform") or {}
    flip = clip.get("flip") or {}
    scale_x = clamp(float(scale.get("x", 1.0)), MIN_SCALE, MAX_SCALE)
    scale_y = clamp(float(scale.get("y", 1.0)), MIN_SCALE, MAX_SCALE)
    out = {
        "start": max(0.0, seconds(target["start"])),
        "duration": max(MIN_CLIP, seconds(target["duration"])),
        "sourceStart": max(0.0, seconds(source.get("start", 0))),
        "speed": speed,
        "volume": max(0.0, float(segment.get("volume", 1.0))),
        "opacity": clamp(float(clip.get("alpha", 1.0)), 0.0, 1.0),
        "scale": scale_x,
        # CapCut: units of half the canvas, y up. Concat: frame fractions, y down.
        "offsetX": clamp(float(transform.get("x", 0.0)) / 2.0, -MAX_OFFSET, MAX_OFFSET),
        "offsetY": clamp(-float(transform.get("y", 0.0)) / 2.0, -MAX_OFFSET, MAX_OFFSET),
        "rotation": wrap_rotation(float(clip.get("rotation", 0.0))),
    }
    if abs(scale_y - scale_x) > 1e-6:
        out["stretchY"] = clamp(scale_y / scale_x, MIN_STRETCH, MAX_STRETCH)
    if flip.get("horizontal"):
        out["flipH"] = True
    if flip.get("vertical"):
        out["flipV"] = True
    return out


def segment_to_clip(segment, track_kind, by_id, media_of, warn):
    material_kind, material = by_id.get(segment.get("material_id"), (None, None))
    if material is None:
        warn("a segment points at a material that is not in the draft; skipped")
        return None
    clip = placement(segment, warn)

    if track_kind in ("video", "audio"):
        media_id = media_of.get(material["id"])
        if media_id is None:
            warn(f"segment of '{material.get('material_name') or material.get('name')}' has no media; skipped")
            return None
        is_image = material.get("type") == "photo"
        clip.update(
            kind="audio" if track_kind == "audio" else ("image" if is_image else "video"),
            mediaId=media_id,
            name=material.get("material_name") or material.get("name") or "clip",
        )
        if is_image:
            clip["sourceStart"], clip["speed"] = 0.0, 1.0
    elif track_kind == "text":
        words = text_of(material.get("content") or "")
        if not words:
            warn("a text segment has no readable text; skipped")
            return None
        style = {"content": words}
        if material.get("text_color"):
            style["color"] = str(material["text_color"]).lower()
        if material.get("font_size"):
            style["fontSize"] = float(material["font_size"]) * TEXT_SIZE_UNIT
        clip.update(kind="text", mediaId="", name=words.splitlines()[0][:40], text=style)
    else:  # effect
        effect = EFFECT_MAP.get(str(material.get("name", "")).lower())
        if effect is None:
            warn(f"effect '{material.get('name')}' has no Concat equivalent; skipped")
            return None
        clip.update(kind="layer", mediaId="", name=material["name"],
                    videoEffects=[{"id": effect, "params": {}, "enabled": True}])
        clip["speed"] = 1.0

    # CapCut hangs a transition on the segment it leaves; Concat on the clip it enters.
    for ref in segment.get("extra_material_refs", []):
        found_kind, found = by_id.get(ref, (None, None))
        if found_kind == "transitions":
            clip["_transition"] = found
    return clip


def apply_transitions(clips: list[dict], warn) -> None:
    for left, right in zip(clips, clips[1:]):
        pending = left.get("_transition")
        if not pending:
            continue
        requested = seconds(pending.get("duration", 0)) or 1.0
        overlap = (left["start"] + left["duration"]) - right["start"]
        if overlap > TOUCH:
            # Concat centres a transition on the cut, so the cut goes in the
            # middle of CapCut's overlap and both blend windows coincide.
            cut = right["start"] + overlap / 2.0
            shift = cut - right["start"]
            left["duration"] = max(MIN_CLIP, cut - left["start"])
            right["start"] = cut
            right["duration"] = max(MIN_CLIP, right["duration"] - shift)
            right["sourceStart"] += shift * right["speed"]
        name = pending.get("name") or pending.get("effect_id") or "transition"
        warn(f"transition '{name}' has no Concat equivalent; using a dissolve")
        length = clamp(requested, MIN_TRANSITION, max(MIN_TRANSITION, min(left["duration"], right["duration"])))
        right["transitionIn"] = {"id": FALLBACK_TRANSITION, "duration": length}


def place_without_overlap(clips: list[dict], warn) -> list[list[dict]]:
    """Concat never lets one clip cover another on a lane: spill onto an extra lane."""
    lanes: list[list[dict]] = []
    for clip in clips:
        end = clip["start"] + clip["duration"]
        for lane in lanes:
            last = lane[-1]
            if clip["start"] >= last["start"] + last["duration"] - TOUCH:
                lane.append(clip)
                break
        else:
            if lanes:
                warn(f"clips overlap on one CapCut track around {clip['start']:.2f}s; moved to an extra lane")
            lanes.append([clip])
    return lanes


# ------------------------------------------------------------------- driver


def write_atomic(path: Path, document: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=".concat-", suffix=".tmp")
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as out:
            json.dump(document, out, ensure_ascii=False, indent=2)
        os.replace(temporary, path)
    except BaseException:
        os.unlink(temporary)
        raise


def convert_folder(folder: Path, out: Path | None = None, dry_run: bool = False) -> dict:
    log.info("detect capcut project try to convert...")
    try:
        draft = json.loads((folder / DRAFT_FILE).read_text(encoding="utf-8"))
    except ValueError as error:
        # Some CapCut builds encrypt the draft. That cannot be read from here.
        raise SystemExit(f"{folder / DRAFT_FILE} is not readable JSON ({error}); "
                         "it may be encrypted by this CapCut version") from error
    meta = None
    if (folder / META_FILE).is_file():
        try:
            meta = json.loads((folder / META_FILE).read_text(encoding="utf-8"))
        except ValueError:
            log.warning("%s is not readable JSON; ignored", META_FILE)
    document, warnings = convert(draft, meta, folder.name)
    target = (out or folder) / "concat.json"
    if not dry_run:
        if target.exists():
            raise SystemExit(f"{target} already exists; refusing to overwrite it")
        write_atomic(target, document)
    log.info("capcut project converted: %d media, %d lane(s), %d clip(s), %d warning(s)%s",
             len(document["media"]), len(document["tracks"]), len(document["clips"]), len(warnings),
             " (dry run, nothing written)" if dry_run else f" -> {target}")
    return document


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("folder", type=Path)
    parser.add_argument("--out", type=Path, help="write concat.json into this folder instead")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="[%(levelname)s] %(message)s")
    if is_concat_project(args.folder):
        log.info("%s is already a Concat project; nothing to convert", args.folder)
        return 0
    if not is_capcut_draft(args.folder):
        log.error("%s holds neither concat.json nor %s", args.folder, DRAFT_FILE)
        return 2
    convert_folder(args.folder, args.out, args.dry_run)
    return 0


if __name__ == "__main__":
    sys.exit(main())
