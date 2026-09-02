// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Pure derivations of what the monitor and the exporter see.
 *
 * Everything here answers one question - given the project and a playhead,
 * what is on screen, what ghosts in, what washes over, what exports - with
 * no React and no IO, which is what makes the cross-fade pre-roll arithmetic
 * and the exporter flattening testable in microseconds. App.tsx memoises
 * these; nothing else derives them a second way.
 */

import {
  clipsAt,
  findMedia,
  findTrack,
  precedingClip,
  type EditorProject,
  type TimelineData,
} from "./editor";
import type { TextStyle } from "./text";

/** The clip the element preview shows: top-most visual clip at the playhead. */
export interface PreviewSource {
  clipId: string;
  path: string;
  /** Where in the source file the playhead sits, in seconds. */
  time: number;
  /** Playback rate, 1 being normal - the element must run at this rate or it
      drifts from the transport and stutters on every corrective seek. */
  speed: number;
  /** A still is shown as an image; there is nothing to seek or play. */
  isStill: boolean;
}

/** A title to draw over the picture, already positioned. */
export interface TextOverlay {
  clipId: string;
  style: TextStyle;
  /** Offset from centred, as a fraction of the frame. */
  offsetX: number;
  offsetY: number;
}

/** A title flattened for the export dialog's rasteriser. */
export interface ExportTitle {
  clipId: string;
  style: TextStyle;
  /** Offset from centred, as a fraction of the frame. */
  offsetX: number;
  offsetY: number;
  start: number;
  duration: number;
  /** Index into the track stack, zero being bottom-most. */
  track: number;
}

/** A cross-fade in progress: the incoming clip's pre-roll, faded in. */
export interface PreviewGhost {
  clipId: string;
  path: string;
  time: number;
  speed: number;
  opacity: number;
}

/** A fade-to-colour transition washing over the playhead. */
export interface PreviewVeil {
  color: string;
  opacity: number;
}

const depthIn = (timeline: TimelineData) => {
  return (trackId: string) => timeline.tracks.findIndex((track) => track.id === trackId);
};

/** Text clips at the playhead on visible tracks, bottom-most first. */
export function textOverlaysAt(
  project: EditorProject,
  timeline: TimelineData,
  playhead: number,
): TextOverlay[] {
  const depth = depthIn(timeline);
  return clipsAt(project, playhead)
    .filter((clip) => clip.kind === "text" && clip.text !== undefined)
    .filter((clip) => findTrack(project, clip.trackId)?.visible !== false)
    .sort((a, b) => depth(a.trackId) - depth(b.trackId))
    .map((clip) => ({
      clipId: clip.id,
      style: clip.text!,
      offsetX: clip.offsetX,
      offsetY: clip.offsetY,
    }));
}

/** The top-most visual clip under the playhead, mapped into its source. */
export function previewSourceAt(
  project: EditorProject,
  timeline: TimelineData,
  playhead: number,
): PreviewSource | null {
  const active = clipsAt(project, playhead).filter(
    (clip) => clip.kind !== "audio" && clip.kind !== "text",
  );
  if (active.length === 0) return null;
  const depth = depthIn(timeline);
  const top = active.reduce((best, clip) =>
    depth(clip.trackId) > depth(best.trackId) ? clip : best,
  );
  const media = findMedia(project, top.mediaId);
  if (!media) return null;
  return {
    clipId: top.id,
    path: media.path,
    time: top.sourceStart + (playhead - top.start) * top.speed,
    speed: top.speed,
    isStill: top.kind === "image",
  };
}

/**
 * The incoming half of a cross-fade under the playhead, if one is in its
 * dissolve window. The handle clamp mirrors the exporter's
 * `resolve_transitions` exactly: no handle, shorter dissolve.
 */
export function previewGhostAt(
  project: EditorProject,
  timeline: TimelineData,
  playhead: number,
): PreviewGhost | null {
  for (const clip of timeline.clips) {
    const transition = clip.transitionIn;
    if (!transition || transition.id !== "cross-fade") continue;
    if (clip.kind !== "video" && clip.kind !== "image") continue;
    if (!precedingClip(project, clip.id)) continue;
    const handle =
      clip.kind === "image" ? Infinity : clip.sourceStart / Math.max(0.0625, clip.speed);
    const d = Math.min(transition.duration, handle);
    const cut = clip.start;
    if (d <= 0 || playhead < cut - d || playhead >= cut) continue;
    const media = findMedia(project, clip.mediaId);
    if (!media) continue;
    return {
      clipId: clip.id,
      path: media.path,
      time: Math.max(0, clip.sourceStart - (cut - playhead) * clip.speed),
      speed: clip.speed,
      opacity: 1 - (cut - playhead) / d,
    };
  }
  return null;
}

/** A fade-to-colour wash over the playhead, if a cut's window covers it. */
export function previewVeilAt(
  project: EditorProject,
  timeline: TimelineData,
  playhead: number,
): PreviewVeil | null {
  for (const clip of timeline.clips) {
    const transition = clip.transitionIn;
    if (!transition) continue;
    if (transition.id !== "fade-black" && transition.id !== "fade-white") continue;
    if (!precedingClip(project, clip.id)) continue;
    const half = transition.duration / 2;
    const cut = clip.start;
    if (playhead < cut - half || playhead > cut + half) continue;
    const opacity =
      playhead <= cut ? (playhead - (cut - half)) / half : (cut + half - playhead) / half;
    return {
      color: transition.id === "fade-white" ? "#ffffff" : "#000000",
      opacity: Math.min(1, Math.max(0, opacity)),
    };
  }
  return null;
}

/** The active timeline's titles, flattened for rasterisation at export. */
export function exportTitlesOf(project: EditorProject, timeline: TimelineData): ExportTitle[] {
  return timeline.clips.flatMap((clip) => {
    if (clip.kind !== "text" || !clip.text) return [];
    const track = findTrack(project, clip.trackId);
    const index = timeline.tracks.findIndex((candidate) => candidate.id === clip.trackId);
    if (!track || !track.visible || index < 0) return [];
    return [
      {
        clipId: clip.id,
        style: clip.text,
        offsetX: clip.offsetX,
        offsetY: clip.offsetY,
        start: clip.start,
        duration: clip.duration,
        track: index,
      },
    ];
  });
}
