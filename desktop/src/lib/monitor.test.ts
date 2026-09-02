// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The monitor's derivations, including the one that mirrors the exporter.
 *
 * `previewGhostAt`'s handle clamp is a deliberate twin of the host's
 * `resolve_transitions` (export.rs): no source handle, shorter dissolve.
 * If these expectations fail, read the exporter first and fix whichever
 * side moved - a preview ghost that disagrees with the render is exactly
 * the bug class this suite exists to keep out.
 */
import { describe, expect, test } from "vitest";

import type { Clip, EditorProject } from "./editor";
import {
  previewGhostAt,
  previewSourceAt,
  previewVeilAt,
  textOverlaysAt,
} from "./monitor";

function clip(overrides: Partial<Clip> = {}): Clip {
  return {
    id: "c1",
    trackId: "T1",
    mediaId: "m1",
    name: "a.mp4",
    kind: "video",
    start: 0,
    duration: 10,
    sourceStart: 0,
    volume: 1,
    fadeIn: 0,
    fadeOut: 0,
    scale: 1,
    offsetX: 0,
    offsetY: 0,
    rotation: 0,
    opacity: 1,
    speed: 1,
    preservePitch: true,
    filters: [],
    videoEffects: [],
    ...overrides,
  };
}

function project(clips: Clip[]): EditorProject {
  return {
    media: [
      {
        id: "m1",
        path: "/a.mp4",
        name: "a.mp4",
        duration: 30,
        kind: "video",
        width: 1920,
        height: 1080,
        frameRate: 30,
        frameRateFraction: "30/1",
        videoCodec: "h264",
        audioCodec: "aac",
        hasAudio: true,
      },
    ],
    fonts: [],
    timelines: [
      {
        id: "TL1",
        name: "Timeline 1",
        tracks: [
          { id: "T1", name: "Track 1", visible: true, muted: false },
          { id: "T2", name: "Track 2", visible: true, muted: false },
        ],
        clips,
      },
    ],
    activeTimelineId: "TL1",
  };
}

const timeline = (p: EditorProject) => p.timelines[0];

describe("previewSourceAt", () => {
  test("the top-most visual clip wins and maps into its source", () => {
    const p = project([
      clip({ id: "under", trackId: "T1" }),
      clip({ id: "over", trackId: "T2", sourceStart: 5, speed: 2 }),
    ]);
    const source = previewSourceAt(p, timeline(p), 3);
    expect(source?.clipId).toBe("over");
    // 3s into the clip at 2x from a 5s in-point: the affine map.
    expect(source?.time).toBe(11);
    expect(source?.speed).toBe(2);
  });

  test("a gap shows nothing", () => {
    const p = project([clip({ start: 5 })]);
    expect(previewSourceAt(p, timeline(p), 1)).toBeNull();
  });
});

describe("previewGhostAt mirrors the exporter's handle clamp", () => {
  const dissolve = (incoming: Partial<Clip>) =>
    project([
      clip({ id: "out", duration: 4 }),
      clip({
        id: "in",
        start: 4,
        duration: 4,
        transitionIn: { id: "cross-fade", duration: 1 },
        ...incoming,
      }),
    ]);

  test("inside the window the pre-roll fades in", () => {
    const p = dissolve({ sourceStart: 2 });
    const ghost = previewGhostAt(p, timeline(p), 3.5);
    expect(ghost?.clipId).toBe("in");
    // Half a second before the cut: half faded, showing the handle.
    expect(ghost?.opacity).toBeCloseTo(0.5, 10);
    expect(ghost?.time).toBeCloseTo(1.5, 10);
  });

  test("no handle, shorter dissolve - exactly like resolve_transitions", () => {
    // Only 0.25s of source exists before the in-point, so the window is
    // 0.25s: at 0.5s before the cut there is no ghost yet.
    const p = dissolve({ sourceStart: 0.25 });
    expect(previewGhostAt(p, timeline(p), 3.5)).toBeNull();
    expect(previewGhostAt(p, timeline(p), 3.9)?.opacity).toBeCloseTo(0.6, 10);
  });

  test("a still has an infinite handle", () => {
    const p = dissolve({ kind: "image", sourceStart: 0 });
    expect(previewGhostAt(p, timeline(p), 3.5)).not.toBeNull();
  });

  test("a cut edited apart orphans the transition", () => {
    const p = project([
      clip({ id: "out", duration: 3 }), // ends at 3, the cut is at 4
      clip({
        id: "in",
        start: 4,
        duration: 4,
        sourceStart: 2,
        transitionIn: { id: "cross-fade", duration: 1 },
      }),
    ]);
    expect(previewGhostAt(p, timeline(p), 3.5)).toBeNull();
  });
});

describe("previewVeilAt", () => {
  test("washes in to the cut and out after it, peaking at one", () => {
    const p = project([
      clip({ id: "out", duration: 4 }),
      clip({
        id: "in",
        start: 4,
        duration: 4,
        sourceStart: 1,
        transitionIn: { id: "fade-black", duration: 2 },
      }),
    ]);
    expect(previewVeilAt(p, timeline(p), 2.9)).toBeNull();
    expect(previewVeilAt(p, timeline(p), 3.5)?.opacity).toBeCloseTo(0.5, 10);
    expect(previewVeilAt(p, timeline(p), 4)?.opacity).toBe(1);
    expect(previewVeilAt(p, timeline(p), 4.5)?.opacity).toBeCloseTo(0.5, 10);
    expect(previewVeilAt(p, timeline(p), 5.1)).toBeNull();
  });
});

describe("textOverlaysAt", () => {
  test("only visible text at the playhead, bottom-most first", () => {
    const style = { content: "hi" } as never;
    const p = project([
      clip({ id: "t2", trackId: "T2", kind: "text", text: style }),
      clip({ id: "t1", trackId: "T1", kind: "text", text: style }),
      clip({ id: "late", trackId: "T1", kind: "text", text: style, start: 20 }),
    ]);
    const overlays = textOverlaysAt(p, timeline(p), 1);
    expect(overlays.map((overlay) => overlay.clipId)).toEqual(["t1", "t2"]);
  });
});
