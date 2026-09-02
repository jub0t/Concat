// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The engine's true frames for the monitor: the paused dwell and the
 * playback stream, one owner.
 *
 * Paused: when the playhead settles, fetch the exporter's own composite and
 * hold it over the approximation until the playhead moves. Playing: while
 * two or more visual layers sit under the playhead, stream frames against
 * the transport clock - requests are
 * quantised to the project's frame grid and issued one measured round-trip
 * ahead of the interpolated playhead, so a frame lands about when it is
 * due; each presented frame also warms the engine's cache for the instants
 * after it (desktop decision 0009). Single layers keep the smooth element
 * preview. Both paths fetch at the user's preview quality - a fraction of
 * the output frame, chosen in the monitor's footer.
 *
 * The request carries an instant and a resolution, nothing else: the engine
 * flattens its own session (engine decision 0009). The UI used to serialise
 * its whole flattened clip list into every one of these requests - per
 * scrub, per streamed frame.
 */
import { useEffect, useRef, useState } from "react";

import { clipsAt, type EditorProject } from "../lib/editor";
import { previewFrame, previewPrefetch } from "../lib/engine";

export interface EngineStill {
  bytes: ArrayBuffer;
  width: number;
  height: number;
}

/** Even dimensions at a fraction of the frame - decoders round odd sizes. */
function scaled(frame: { width: number; height: number }, quality: number) {
  const fraction = Number.isFinite(quality) && quality > 0 ? Math.min(quality, 1) : 0.5;
  return {
    width: Math.max(2, Math.round((frame.width * fraction) / 2) * 2),
    height: Math.max(2, Math.round((frame.height * fraction) / 2) * 2),
  };
}

export function useEngineTruth({
  playing,
  loaded,
  playhead,
  project,
  renderableClipCount,
  frame,
  fps,
  quality,
  latest,
}: {
  playing: boolean;
  loaded: boolean;
  playhead: number;
  /** The engine's state; a new identity per edit refreshes the paused still. */
  project: EditorProject;
  /** Non-text clips on the active timeline - zero means nothing to composite. */
  renderableClipCount: number;
  frame: { width: number; height: number };
  /** The project's frame rate; the grid streamed requests snap to. */
  fps: number;
  /** Preview resolution as a fraction of the output frame: 1, 0.5, 0.25. */
  quality: number;
  /** Live values, read mid-flight without restarting the loops. */
  latest: { current: { playhead: number; frame: { width: number; height: number }; project: EditorProject } };
}): EngineStill | null {
  const [engineStill, setEngineStill] = useState<EngineStill | null>(null);
  const stillToken = useRef(0);

  // The paused dwell.
  useEffect(() => {
    // While playing, the streaming loop below owns the still; clearing it
    // here on every playhead tick would fight the stream frame by frame.
    if (playing) return;
    const token = ++stillToken.current;
    setEngineStill(null);
    if (!loaded || renderableClipCount === 0) return;

    const timer = window.setTimeout(() => {
      const { width, height } = scaled(frame, quality);
      previewFrame({ time: latest.current.playhead, width, height })
        .then((bytes) => {
          if (stillToken.current === token) setEngineStill({ bytes, width, height });
        })
        .catch(() => {
          if (stillToken.current === token) setEngineStill(null);
        });
    }, 160);
    return () => window.clearTimeout(timer);
    // `project` is the refresh signal: an edit while paused must re-fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, loaded, playhead, project, renderableClipCount, frame, quality]);

  // The playback stream, paced by the transport clock. Still pull - one
  // request in flight, never a queue of stale frames - but the requests are
  // snapped to the project's frame grid and issued one lead ahead of the
  // interpolated playhead, where the lead tracks the measured round-trip:
  // a frame arrives about when it is due instead of a round-trip late, and
  // the same instant is never composited twice. After each presented frame
  // a fire-and-forget prefetch warms the engine's cache for the instants
  // after it, which keeps the pool's readers rolling forward and turns the
  // next pulls into cache hits. Frames come back at the chosen preview
  // quality; dropping it is the lever when playback cannot keep up.
  useEffect(() => {
    if (!playing || !loaded) return;
    let live = true;
    // Invalidate any paused-dwell fetch still in flight from before play.
    stillToken.current += 1;
    const rate = Number.isFinite(fps) && fps > 0 ? fps : 30;
    const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
    const visualLayers = (time: number) =>
      clipsAt(latest.current.project, time).filter(
        (clip) => clip.kind === "video" || clip.kind === "image",
      ).length;

    const run = async () => {
      // The lead starts at one frame and follows the round-trip, clamped to
      // [1, 3] frames - a pathological decode must not push requests seconds
      // ahead of what the user is hearing.
      let lead = 1 / rate;
      let presented = -1;
      while (live) {
        const now = latest.current.playhead;
        if (visualLayers(now) < 2) {
          setEngineStill(null);
          presented = -1;
          await wait(120);
          continue;
        }
        const target = Math.round((now + lead) * rate);
        if (target === presented) {
          // The clock has not reached the next frame yet; a quarter-frame
          // nap keeps the loop from spinning on an already-shown instant.
          await wait(250 / rate);
          continue;
        }
        const { width, height } = scaled(latest.current.frame, quality);
        const started = performance.now();
        try {
          const bytes = await previewFrame({ time: target / rate, width, height });
          const trip = (performance.now() - started) / 1000;
          lead = Math.min(Math.max(0.8 * lead + 0.2 * trip, 1 / rate), 3 / rate);
          presented = target;
          if (live) {
            setEngineStill({ bytes, width, height });
            void previewPrefetch({ time: (target + 1) / rate, width, height }, 2).catch(
              () => undefined,
            );
          }
        } catch {
          // A failed frame keeps the approximation on screen; back off so a
          // persistently failing source cannot spin the pool.
          if (live) setEngineStill(null);
          presented = -1;
          await wait(200);
        }
      }
    };
    void run();
    return () => {
      live = false;
    };
  }, [playing, loaded, latest, fps, quality]);

  return engineStill;
}
