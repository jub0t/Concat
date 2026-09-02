// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The UI's view of the engine-owned edit.
 *
 * The engine (`wolfcut-project`, via the `editor_*` commands) is the model of
 * record: every mutation is an [`EditorCommand`] sent through
 * `lib/engine.ts`, and the state that comes back is the truth. What lives
 * here is everything that is legitimately the UI's:
 *
 * - the TypeScript mirror of the engine's types (field for field - serde on
 *   the other side writes exactly these names),
 * - read-only selectors over that state,
 * - the *gesture echo*: during a drag the UI previews the edit locally with
 *   the same arithmetic the engine will apply, then commits one command on
 *   release. One command per gesture is what keeps undo meaning "undo the
 *   drag", not "undo one pixel of it".
 *
 * No mutation of project state happens in TypeScript outside the echo, and
 * the echo never survives past its commit.
 */

import type { MsgKey } from "./i18n";
import type { Clip } from "./generated/Clip";
import type { ClipPatch } from "./generated/ClipPatch";
import type { Command as EditorCommand } from "./generated/Command";
import type { CustomFont as EngineFont } from "./generated/CustomFont";
import type { EditorView as EngineEditorView } from "./generated/EditorView";
import type { MediaItem } from "./generated/MediaItem";
import type { Project } from "./generated/Project";
import type { Timeline as TimelineData } from "./generated/Timeline";
import type { Track } from "./generated/Track";

// ── the engine's types, generated ────────────────────────────────────────────
// These used to be hand-mirrored interfaces the compiler could not check
// against serde. They are now generated from the Rust source of truth by
// ts-rs (see ./generated/README.md) and re-exported here under the names the
// UI has always used, so a wire change on the Rust side is a type error on
// this one. `EditorCommand` is the engine's `Command`; `TimelineData` its
// `Timeline`; `EditorSettings` the host's `SettingsView`.

export type { MediaKind } from "./generated/MediaKind";
export type { ClipKind } from "./generated/ClipKind";
export type { MediaItem } from "./generated/MediaItem";
export type { Track } from "./generated/Track";
export type { Clip } from "./generated/Clip";
export type { Timeline as TimelineData } from "./generated/Timeline";
export type { SettingsView as EditorSettings } from "./generated/SettingsView";
export type { ClipPatch } from "./generated/ClipPatch";
export type { ClipMove } from "./generated/ClipMove";
export type { NewMedia } from "./generated/NewMedia";
export type { Command as EditorCommand } from "./generated/Command";

/** A timeline's identity, for the tab strip. */
export type TimelineMeta = Pick<TimelineData, "id" | "name">;

/** Transform limits, shared with the preview gizmo. The engine clamps too. */
export const MIN_SCALE = 0.05;
export const MAX_SCALE = 8;

/** The engine's font entry plus the one field that is legitimately the
 * UI's - whether loading the face failed on this machine. */
export type CustomFont = EngineFont & {
  /** UI-only: a load attempt failed. Never sent to or from the engine. */
  missing?: boolean;
};

/** The engine's project, with fonts widened to carry the UI-only
 * `missing` flag. Everything else is exactly what serde wrote. */
export type EditorProject = Omit<Project, "fonts"> & { fonts: CustomFont[] };

/** What every editor call returns: the authoritative state. */
export type EditorView = Omit<EngineEditorView, "project"> & { project: EditorProject };

// ── selectors ────────────────────────────────────────────────────────────────

export function activeTimeline(project: EditorProject): TimelineData {
  return (
    project.timelines.find((timeline) => timeline.id === project.activeTimelineId) ??
    project.timelines[0]
  );
}

export function findClip(project: EditorProject, clipId: string): Clip | null {
  return activeTimeline(project).clips.find((clip) => clip.id === clipId) ?? null;
}

export function findTrack(project: EditorProject, trackId: string): Track | null {
  return activeTimeline(project).tracks.find((track) => track.id === trackId) ?? null;
}

export function findMedia(project: EditorProject, mediaId: string): MediaItem | null {
  return project.media.find((item) => item.id === mediaId) ?? null;
}

/** Where the last clip of the active timeline ends. */
export function projectDuration(project: EditorProject): number {
  return activeTimeline(project).clips.reduce(
    (end, clip) => Math.max(end, clip.start + clip.duration),
    0,
  );
}

/** Clips under the playhead on tracks that are not disabled. */
export function clipsAt(project: EditorProject, time: number): Clip[] {
  const timeline = activeTimeline(project);
  return timeline.clips.filter((clip) => {
    const track = timeline.tracks.find((candidate) => candidate.id === clip.trackId);
    if (!track) return false;
    if (clip.kind !== "audio" && !track.visible) return false;
    if (clip.kind === "audio" && track.muted) return false;
    return time >= clip.start && time < clip.start + clip.duration;
  });
}

/** The audio clips holding sound detached from this video clip, if any. */
export function detachedAudioOf(project: EditorProject, videoClipId: string): Clip[] {
  return activeTimeline(project).clips.filter((clip) => clip.detachedFrom === videoClipId);
}

/**
 * The clip that ends exactly where this one starts, on the same track - what
 * a transition needs to exist. Whole-frame tolerance, matching the engine's
 * exporter-side adjacency test.
 */
export function precedingClip(project: EditorProject, clipId: string): Clip | null {
  const timeline = activeTimeline(project);
  const clip = timeline.clips.find((candidate) => candidate.id === clipId);
  if (!clip) return null;
  return (
    timeline.clips.find(
      (other) =>
        other.id !== clip.id &&
        other.trackId === clip.trackId &&
        other.kind !== "audio" &&
        Math.abs(other.start + other.duration - clip.start) < 1 / 60,
    ) ?? null
  );
}

/** How many clips a timeline holds. For the delete prompt. */
export function timelineClipCount(project: EditorProject, timelineId: string): number {
  return project.timelines.find((timeline) => timeline.id === timelineId)?.clips.length ?? 0;
}

const JOIN_EPSILON = 1e-6;

/**
 * Why these clips cannot be merged, or null if they can - the UI's read-only
 * twin of the engine's own check, for the disabled button's tooltip. The
 * engine re-validates on the command; this only phrases the reason early.
 */
export function whyNotMerge(project: EditorProject, clipIds: readonly string[]): MsgKey | null {
  if (clipIds.length < 2) return "timeline.mergeNeedTwo";
  const timeline = activeTimeline(project);
  const clips = clipIds.flatMap((id) => {
    const clip = timeline.clips.find((candidate) => candidate.id === id);
    return clip ? [clip] : [];
  });
  if (clips.length < 2) return "timeline.mergeNeedTwo";
  if (clips.some((clip) => clip.trackId !== clips[0].trackId)) {
    return "timeline.mergeSameTrack";
  }
  if (clips.some((clip) => clip.mediaId !== clips[0].mediaId)) {
    return "timeline.mergeSameFile";
  }
  if (clips.some((clip) => clip.speed !== clips[0].speed)) {
    return "timeline.mergeSameSpeed";
  }
  const ordered = [...clips].sort((left, right) => left.start - right.start);
  for (let index = 1; index < ordered.length; index += 1) {
    const previous = ordered[index - 1];
    const current = ordered[index];
    if (Math.abs(current.start - (previous.start + previous.duration)) > JOIN_EPSILON) {
      return "timeline.mergeMustTouch";
    }
    if (
      Math.abs(
        current.sourceStart - (previous.sourceStart + previous.duration * previous.speed),
      ) > JOIN_EPSILON
    ) {
      return "timeline.mergeOutOfOrder";
    }
  }
  return null;
}

/**
 * The nearest interesting time to `time`, within `threshold` seconds. Pure
 * view logic: snapping decides what the gesture *proposes*, the engine only
 * ever sees the snapped result.
 */
export function snapTime(
  project: EditorProject,
  time: number,
  { threshold, playhead, exclude }: { threshold: number; playhead: number; exclude?: string },
): number {
  const targets = [0, playhead];
  for (const clip of activeTimeline(project).clips) {
    if (clip.id === exclude) continue;
    targets.push(clip.start, clip.start + clip.duration);
  }
  let best = time;
  let bestDistance = threshold;
  for (const target of targets) {
    const distance = Math.abs(target - time);
    if (distance < bestDistance) {
      best = target;
      bestDistance = distance;
    }
  }
  return best;
}

// ── the gesture echo ─────────────────────────────────────────────────────────

/** Transient per-clip patches shown during a gesture, keyed by clip id. */
export type Echo = Record<string, Partial<Clip>>;

/** The project as the monitor and timeline should draw it mid-gesture. */
export function withEcho(project: EditorProject, echo: Echo | null): EditorProject {
  if (!echo || Object.keys(echo).length === 0) return project;
  return {
    ...project,
    timelines: project.timelines.map((timeline) =>
      timeline.id === project.activeTimelineId
        ? {
            ...timeline,
            clips: timeline.clips.map((clip) =>
              echo[clip.id] ? { ...clip, ...echo[clip.id] } : clip,
            ),
          }
        : timeline,
    ),
  };
}

const MIN_CLIP_DURATION = 1 / 60;

/** The trim arithmetic, identical to the engine's, for the live echo. */
export function trimPatch(clip: Clip, edge: "start" | "end", delta: number): Partial<Clip> {
  if (edge === "end") {
    return { duration: Math.max(MIN_CLIP_DURATION, clip.duration + delta) };
  }
  const shift = Math.min(delta, clip.duration - MIN_CLIP_DURATION);
  const start = Math.max(0, clip.start + shift);
  const applied = start - clip.start;
  return {
    start,
    duration: clip.duration - applied,
    sourceStart: Math.max(0, clip.sourceStart + applied * clip.speed),
  };
}

/** The speed arithmetic, identical to the engine's, for the live echo. */
export function speedPatch(clip: Clip, speed: number): Partial<Clip> {
  const next = Math.max(0.0625, Math.min(16, speed));
  const sourceCovered = clip.duration * clip.speed;
  return { speed: next, duration: Math.max(MIN_CLIP_DURATION, sourceCovered / next) };
}

/** The transform clamps, identical to the engine's, for the live echo. */
export function transformPatch(
  patch: Partial<Pick<Clip, "scale" | "offsetX" | "offsetY" | "rotation">>,
): Partial<Clip> {
  const clamped: Partial<Clip> = {};
  if (patch.scale !== undefined) clamped.scale = Math.max(0.05, Math.min(8, patch.scale));
  if (patch.offsetX !== undefined) clamped.offsetX = Math.max(-3, Math.min(3, patch.offsetX));
  if (patch.offsetY !== undefined) clamped.offsetY = Math.max(-3, Math.min(3, patch.offsetY));
  if (patch.rotation !== undefined) {
    const wrapped = ((patch.rotation % 360) + 540) % 360 - 180;
    clamped.rotation = wrapped === -180 ? 180 : wrapped;
  }
  return clamped;
}

/**
 * Turns one clip's accumulated echo into the command(s) that make it real.
 *
 * A gesture touches one family of fields, so this usually yields exactly one
 * command; the ordering (speed, then transform, then move/trim, then the
 * patch) only matters for mixed accumulations from a busy panel.
 */
export function commandsForEcho(base: Clip, patch: Partial<Clip>): EditorCommand[] {
  const commands: EditorCommand[] = [];
  const has = (key: keyof Clip) => key in patch;

  if (has("speed") && patch.speed !== undefined) {
    commands.push({ op: "setClipSpeed", clipId: base.id, speed: patch.speed });
  }

  if (has("scale") || has("offsetX") || has("offsetY") || has("rotation")) {
    commands.push({
      op: "setClipTransform",
      clipId: base.id,
      ...(has("scale") ? { scale: patch.scale } : {}),
      ...(has("offsetX") ? { offsetX: patch.offsetX } : {}),
      ...(has("offsetY") ? { offsetY: patch.offsetY } : {}),
      ...(has("rotation") ? { rotation: patch.rotation } : {}),
    });
  }

  const durationChanged =
    !has("speed") && has("duration") && patch.duration !== undefined
      ? patch.duration !== base.duration
      : false;
  const startChanged = has("start") && patch.start !== undefined && patch.start !== base.start;
  // Same-value guard, like start: an echoed-but-unchanged track must not
  // commit a do-nothing MoveClips, which would burn an empty undo step.
  const trackChanged =
    has("trackId") && patch.trackId !== undefined && patch.trackId !== base.trackId;

  if (durationChanged && startChanged) {
    commands.push({
      op: "trimClip",
      clipId: base.id,
      edge: "start",
      delta: (patch.start ?? base.start) - base.start,
    });
  } else if (durationChanged) {
    commands.push({
      op: "trimClip",
      clipId: base.id,
      edge: "end",
      delta: (patch.duration ?? base.duration) - base.duration,
    });
  }

  // A move commits whatever the trims above did not. Not an else-branch: an
  // echo holding both a trim and a track change (a busy panel's mixed
  // accumulation) used to commit the trim and silently lose the move. After
  // a head trim the engine has already placed the start, so the move's start
  // - the same value - is idempotent and only the track genuinely applies.
  if (trackChanged || (startChanged && !durationChanged)) {
    commands.push({
      op: "moveClips",
      moves: [
        {
          clipId: base.id,
          start: patch.start ?? base.start,
          trackId: patch.trackId ?? base.trackId,
        },
      ],
    });
  }

  const update: ClipPatch = {};
  if (has("name") && patch.name !== undefined) update.name = patch.name;
  if (has("volume") && patch.volume !== undefined) update.volume = patch.volume;
  if (has("fadeIn") && patch.fadeIn !== undefined) update.fadeIn = patch.fadeIn;
  if (has("fadeOut") && patch.fadeOut !== undefined) update.fadeOut = patch.fadeOut;
  if (has("opacity") && patch.opacity !== undefined) update.opacity = patch.opacity;
  if (has("preservePitch") && patch.preservePitch !== undefined) {
    update.preservePitch = patch.preservePitch;
  }
  if (has("filters") && patch.filters !== undefined) update.filters = patch.filters;
  if (has("videoEffects") && patch.videoEffects !== undefined) {
    update.videoEffects = patch.videoEffects;
  }
  // Key present with undefined means "clear" for the two clearable fields.
  if (has("transitionIn")) update.transitionIn = patch.transitionIn ?? null;
  if (has("text")) update.text = patch.text ?? null;
  if (Object.keys(update).length > 0) {
    commands.push({ op: "updateClip", clipId: base.id, patch: update });
  }

  return commands;
}
