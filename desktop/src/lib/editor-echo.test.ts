// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The echo's commit: accumulated drag patches into engine commands.
 *
 * `commandsForEcho` is the moment the preview becomes real. Every drag in the
 * app funnels through it on release, so a wrong translation here does not
 * mislead a preview - it commits a wrong edit to the document. These tests pin
 * the exact command list for each echo shape. When one fails, decide which
 * side moved: the echo builders in `lib/editor.ts` (trimPatch/speedPatch/
 * transformPatch write the patches this function reads) or the command
 * vocabulary in `engine/crates/wolfcut-project/src/commands.rs`.
 */
import { describe, expect, test } from "vitest";

import {
  commandsForEcho,
  speedPatch,
  transformPatch,
  trimPatch,
  withEcho,
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

describe("commandsForEcho: moves", () => {
  test("a start change alone is one move on the same track", () => {
    expect(commandsForEcho(clip(), { start: 3 })).toEqual([
      { op: "moveClips", moves: [{ clipId: "c1", start: 3, trackId: "T1" }] },
    ]);
  });

  test("a track change alone is one move keeping the start", () => {
    expect(commandsForEcho(clip({ start: 2 }), { trackId: "T2" })).toEqual([
      { op: "moveClips", moves: [{ clipId: "c1", start: 2, trackId: "T2" }] },
    ]);
  });

  test("start and track together stay one move", () => {
    expect(commandsForEcho(clip(), { start: 5, trackId: "T3" })).toEqual([
      { op: "moveClips", moves: [{ clipId: "c1", start: 5, trackId: "T3" }] },
    ]);
  });

  test("a trackId echoed back unchanged commits nothing", () => {
    // Dropping a clip back onto its own track at its own start is not an
    // edit: the same-value guard start and duration already have. It used to
    // commit a do-nothing MoveClips and burn an empty undo step.
    expect(commandsForEcho(clip(), { trackId: "T1" })).toEqual([]);
  });

  test("a track change combined with a trim commits both", () => {
    // A busy panel's mixed accumulation: the head trim commits first (the
    // engine places the start), then the move carries the track - it used to
    // be silently lost in the trim's else-branch.
    expect(
      commandsForEcho(clip({ start: 2, duration: 4 }), { start: 3, duration: 3, trackId: "T2" }),
    ).toEqual([
      { op: "trimClip", clipId: "c1", edge: "start", delta: 1 },
      { op: "moveClips", moves: [{ clipId: "c1", start: 3, trackId: "T2" }] },
    ]);
  });
});

describe("commandsForEcho: trims", () => {
  test("a head trim becomes one start-edge TrimClip carrying the travel", () => {
    const base = clip();
    // trimPatch writes start+duration+sourceStart; the commit sends only the
    // start delta and trusts the engine to redo the same arithmetic.
    const patch = trimPatch(base, "start", 1);
    expect(patch).toEqual({ start: 1, duration: 9, sourceStart: 1 });
    expect(commandsForEcho(base, patch)).toEqual([
      { op: "trimClip", clipId: "c1", edge: "start", delta: 1 },
    ]);
  });

  test("a clamped head trim commits the applied travel, not the request", () => {
    const base = clip({ start: 1 });
    const patch = trimPatch(base, "start", -3); // only 1s exists before zero
    expect(commandsForEcho(base, patch)).toEqual([
      { op: "trimClip", clipId: "c1", edge: "start", delta: -1 },
    ]);
  });

  test("a head trim at speed keeps the delta in timeline seconds", () => {
    const base = clip({ duration: 5, speed: 2 });
    const patch = trimPatch(base, "start", 1); // sourceStart moves 2, start moves 1
    expect(commandsForEcho(base, patch)).toEqual([
      { op: "trimClip", clipId: "c1", edge: "start", delta: 1 },
    ]);
  });

  test("an end trim becomes one end-edge TrimClip with the duration delta", () => {
    expect(commandsForEcho(clip(), trimPatch(clip(), "end", 2))).toEqual([
      { op: "trimClip", clipId: "c1", edge: "end", delta: 2 },
    ]);
    expect(commandsForEcho(clip(), { duration: 7 })).toEqual([
      { op: "trimClip", clipId: "c1", edge: "end", delta: -3 },
    ]);
  });

  test("sourceStart never rides in a command", () => {
    // The engine owns that arithmetic; if a sourceStart ever leaks into the
    // wire format the two sides can disagree about the in-point.
    const commands = commandsForEcho(clip(), trimPatch(clip(), "start", 2));
    expect(JSON.stringify(commands)).not.toContain("sourceStart");
  });
});

describe("commandsForEcho: speed", () => {
  test("a speed change is SetClipSpeed alone - the engine re-derives duration", () => {
    const base = clip();
    const patch = speedPatch(base, 2); // writes speed AND duration into the echo
    expect(patch).toEqual({ speed: 2, duration: 5 });
    expect(commandsForEcho(base, patch)).toEqual([
      { op: "setClipSpeed", clipId: "c1", speed: 2 },
    ]);
  });
});

describe("commandsForEcho: transforms", () => {
  test("only the echoed transform fields ride the command", () => {
    expect(commandsForEcho(clip(), { scale: 1.5, rotation: 90 })).toEqual([
      { op: "setClipTransform", clipId: "c1", scale: 1.5, rotation: 90 },
    ]);
    expect(commandsForEcho(clip(), transformPatch({ offsetX: 0.25 }))).toEqual([
      { op: "setClipTransform", clipId: "c1", offsetX: 0.25 },
    ]);
  });
});

describe("commandsForEcho: the update patch", () => {
  test("plain fields collect into one UpdateClip", () => {
    expect(
      commandsForEcho(clip(), {
        name: "renamed",
        volume: 0.5,
        fadeIn: 0.2,
        fadeOut: 0.3,
        opacity: 0.8,
        preservePitch: false,
        filters: [{ id: "bass", params: { gain: 6 } }],
        videoEffects: [{ id: "sepia", params: {} }],
      }),
    ).toEqual([
      {
        op: "updateClip",
        clipId: "c1",
        patch: {
          name: "renamed",
          volume: 0.5,
          fadeIn: 0.2,
          fadeOut: 0.3,
          opacity: 0.8,
          preservePitch: false,
          filters: [{ id: "bass", params: { gain: 6 } }],
          videoEffects: [{ id: "sepia", params: {} }],
        },
      },
    ]);
  });

  test("a present-but-undefined transitionIn or text means clear, as null", () => {
    // The wire keeps the engine's double-option distinction: absent leaves the
    // field alone, null clears it. The echo spells "clear" as a present
    // undefined, and the commit must translate that to null, never drop it.
    expect(commandsForEcho(clip(), { transitionIn: undefined })).toEqual([
      { op: "updateClip", clipId: "c1", patch: { transitionIn: null } },
    ]);
    expect(commandsForEcho(clip(), { text: undefined })).toEqual([
      { op: "updateClip", clipId: "c1", patch: { text: null } },
    ]);
  });

  test("a real transition value passes through", () => {
    expect(
      commandsForEcho(clip(), { transitionIn: { id: "cross-fade", duration: 1 } }),
    ).toEqual([
      {
        op: "updateClip",
        clipId: "c1",
        patch: { transitionIn: { id: "cross-fade", duration: 1 } },
      },
    ]);
  });
});

describe("commandsForEcho: mixed accumulations and no-ops", () => {
  test("a busy panel commits in the documented order: speed, transform, move, patch", () => {
    const commands = commandsForEcho(clip(), {
      volume: 0.7,
      start: 4,
      scale: 2,
      speed: 2,
      duration: 5, // speedPatch wrote this; it must NOT read as a trim
    });
    expect(commands).toEqual([
      { op: "setClipSpeed", clipId: "c1", speed: 2 },
      { op: "setClipTransform", clipId: "c1", scale: 2 },
      { op: "moveClips", moves: [{ clipId: "c1", start: 4, trackId: "T1" }] },
      { op: "updateClip", clipId: "c1", patch: { volume: 0.7 } },
    ]);
  });

  test("an empty echo commits nothing", () => {
    expect(commandsForEcho(clip(), {})).toEqual([]);
  });

  test("echoed values equal to the base commit nothing", () => {
    // A drag that ends where it began, or a trim clamped to zero travel.
    expect(commandsForEcho(clip({ start: 2 }), { start: 2 })).toEqual([]);
    expect(commandsForEcho(clip(), { duration: 10 })).toEqual([]);
    const pinned = clip(); // at 0: dragging the head left cannot travel at all
    expect(commandsForEcho(pinned, trimPatch(pinned, "start", -1))).toEqual([]);
  });
});

describe("withEcho", () => {
  const project = (clips: Clip[]): EditorProject => ({
    media: [],
    fonts: [],
    timelines: [
      {
        id: "TL1",
        name: "Timeline 1",
        tracks: [{ id: "T1", name: "Track 1", visible: true, muted: false }],
        clips,
      },
      { id: "TL2", name: "Timeline 2", tracks: [], clips: [clip({ id: "other" })] },
    ],
    activeTimelineId: "TL1",
  });

  test("patches land only on the active timeline's clips", () => {
    const p = project([clip()]);
    const echoed = withEcho(p, { c1: { start: 5 }, other: { start: 9 } });
    expect(echoed.timelines[0].clips[0].start).toBe(5);
    // A clip with the same id on another timeline is untouched.
    expect(echoed.timelines[1].clips[0].start).toBe(0);
  });

  test("no echo returns the project untouched, same reference", () => {
    const p = project([clip()]);
    expect(withEcho(p, null)).toBe(p);
    expect(withEcho(p, {})).toBe(p);
  });
});
