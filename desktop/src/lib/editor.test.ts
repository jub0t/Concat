// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The gesture echo's arithmetic against the engine's.
 *
 * `lib/editor.ts` deliberately duplicates a handful of the engine's rules so
 * a drag can preview locally before its one command commits. That mirror is
 * the architecture's soft spot: if the two drift, the preview lies and the
 * commit "jumps". Every expectation here is copied from the corresponding
 * test or clamp in `engine/crates/wolfcut-project/src/commands.rs` - when one
 * of these fails, read the engine first and fix whichever side moved.
 */
import { describe, expect, test } from "vitest";

import {
  snapTime,
  speedPatch,
  transformPatch,
  trimPatch,
  whyNotMerge,
  type Clip,
  type EditorProject,
} from "./editor";

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
    media: [],
    fonts: [],
    timelines: [
      {
        id: "TL1",
        name: "Timeline 1",
        tracks: [{ id: "T1", name: "Track 1", visible: true, muted: false }],
        clips,
      },
    ],
    activeTimelineId: "TL1",
  };
}

const MIN_CLIP_DURATION = 1 / 60;

describe("trimPatch mirrors TrimClip", () => {
  test("an end trim grows and shrinks with the duration floor", () => {
    expect(trimPatch(clip(), "end", 2)).toEqual({ duration: 12 });
    expect(trimPatch(clip(), "end", -20)).toEqual({ duration: MIN_CLIP_DURATION });
  });

  test("a head trim moves the in-point with speed", () => {
    // The engine's a_head_trim_moves_the_in_point_with_speed: 10s of source
    // at 2x occupies 5s; trimming 1s off the head covers 2 source seconds.
    const fast = clip({ duration: 5, speed: 2 });
    expect(trimPatch(fast, "start", 1)).toEqual({
      start: 1,
      duration: 4,
      sourceStart: 2,
    });
  });

  test("a head trim clamps at the timeline's start", () => {
    const placed = clip({ start: 1 });
    // Dragging 3s left only travels the 1s that exists before zero.
    expect(trimPatch(placed, "start", -3)).toEqual({
      start: 0,
      duration: 11,
      sourceStart: 0,
    });
  });

  test("a head trim cannot consume the whole clip", () => {
    const patch = trimPatch(clip(), "start", 99);
    expect(patch.duration).toBeCloseTo(MIN_CLIP_DURATION, 10);
    expect(patch.start).toBeCloseTo(10 - MIN_CLIP_DURATION, 10);
  });
});

describe("speedPatch mirrors SetClipSpeed", () => {
  test("the source covered is held constant", () => {
    // 10 timeline seconds at 1x cover 10 source seconds; at 2x they fit in 5.
    expect(speedPatch(clip(), 2)).toEqual({ speed: 2, duration: 5 });
    expect(speedPatch(clip({ duration: 5, speed: 2 }), 0.5)).toEqual({
      speed: 0.5,
      duration: 20,
    });
  });

  test("speed clamps to the engine range", () => {
    expect(speedPatch(clip(), 100).speed).toBe(16);
    expect(speedPatch(clip(), 0).speed).toBe(0.0625);
  });
});

describe("transformPatch mirrors SetClipTransform", () => {
  test("scale and offsets clamp to the engine bounds", () => {
    expect(transformPatch({ scale: 100 })).toEqual({ scale: 8 });
    expect(transformPatch({ scale: 0 })).toEqual({ scale: 0.05 });
    expect(transformPatch({ offsetX: 5, offsetY: -5 })).toEqual({ offsetX: 3, offsetY: -3 });
  });

  test("rotation wraps into (-180, 180], never accumulating turns", () => {
    expect(transformPatch({ rotation: 190 })).toEqual({ rotation: -170 });
    expect(transformPatch({ rotation: -190 })).toEqual({ rotation: 170 });
    expect(transformPatch({ rotation: 540 })).toEqual({ rotation: 180 });
    // The engine maps the -180 boundary to 180; the echo must agree.
    expect(transformPatch({ rotation: -180 })).toEqual({ rotation: 180 });
  });
});

describe("whyNotMerge phrases the engine's refusals", () => {
  // The keys' en.json values are the engine's own refusal sentences: the
  // engine re-validates on the command and its error becomes the toast, so
  // in English the tooltip and the toast must match. Other locales translate
  // the tooltip while engine errors stay English.
  const halves = [
    clip({ id: "a", duration: 4 }),
    clip({ id: "b", start: 4, duration: 6, sourceStart: 4 }),
  ];

  test("source-continuous neighbours merge", () => {
    expect(whyNotMerge(project(halves), ["a", "b"])).toBeNull();
  });

  test("each refusal names its reason", () => {
    expect(whyNotMerge(project(halves), ["a"])).toBe("timeline.mergeNeedTwo");
    expect(
      whyNotMerge(project([halves[0], { ...halves[1], trackId: "T2" }]), ["a", "b"]),
    ).toBe("timeline.mergeSameTrack");
    expect(
      whyNotMerge(project([halves[0], { ...halves[1], mediaId: "m2" }]), ["a", "b"]),
    ).toBe("timeline.mergeSameFile");
    expect(
      whyNotMerge(project([halves[0], { ...halves[1], speed: 2 }]), ["a", "b"]),
    ).toBe("timeline.mergeSameSpeed");
    expect(
      whyNotMerge(project([halves[0], { ...halves[1], start: 5 }]), ["a", "b"]),
    ).toBe("timeline.mergeMustTouch");
    expect(
      whyNotMerge(project([halves[0], { ...halves[1], sourceStart: 0 }]), ["a", "b"]),
    ).toBe("timeline.mergeOutOfOrder");
  });
});

describe("snapTime", () => {
  test("snaps to clip edges and the playhead within the threshold", () => {
    const timeline = project([clip({ start: 2, duration: 3 })]);
    const options = { threshold: 0.25, playhead: 9 };
    expect(snapTime(timeline, 2.1, options)).toBe(2);
    expect(snapTime(timeline, 4.9, options)).toBe(5);
    expect(snapTime(timeline, 8.8, options)).toBe(9);
    expect(snapTime(timeline, 3.5, options)).toBe(3.5);
  });

  test("a clip does not snap to itself", () => {
    const timeline = project([clip({ id: "me", start: 2, duration: 3 })]);
    expect(snapTime(timeline, 2.1, { threshold: 0.25, playhead: 99, exclude: "me" })).toBe(2.1);
  });
});
