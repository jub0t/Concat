// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";

import { transportPause, transportPlay, transportSeek } from "./engine";
import { listen } from "@tauri-apps/api/event";

/**
 * The playback clock.
 *
 * The engine owns it: the position that matters is the audio device's sample
 * counter, reported through `transport` events at ~30Hz. Between events the
 * UI interpolates on the wall clock from the last reported anchor, so the
 * playhead moves at animation rate while never drifting further than one
 * event interval from the truth.
 *
 * In a plain browser (`npm run dev` with no Rust host) there is no engine and
 * the anchor simply never gets corrected - the wall clock alone is the same
 * prototype behaviour this file always had there.
 */
/**
 * The methods, kept apart from the values on purpose. `playhead` changes
 * every animation frame while playing; the methods never change at all. One
 * object holding both would get a fresh identity per frame, and everything
 * keyed on it - effect deps, memoised children - would churn at 60fps. So
 * consumers that only *drive* the transport take `controls` (one stable
 * identity for the life of the hook) and never re-render on playback, while
 * the few that *display* the playhead subscribe to it explicitly.
 */
export interface TransportControls {
  play: () => void;
  pause: () => void;
  toggle: () => void;
  /** Jumps to an absolute time, clamped at zero. */
  seek: (seconds: number) => void;
  /** Moves by a signed number of frames at the given rate. */
  step: (frames: number, frameRate: number) => void;
}

export interface Transport {
  playhead: number;
  playing: boolean;
  controls: TransportControls;
}

export function useTransport({ duration }: { duration: number }): Transport {
  const native = isTauri();
  const [playhead, setPlayhead] = useState(0);
  const [playing, setPlaying] = useState(false);

  // The last known-true position and when it was known. While playing, the
  // rendered playhead is `position + (now - at)`.
  const anchor = useRef({ position: 0, at: 0 });
  // Read inside loops and callbacks without restarting them.
  const playheadRef = useRef(0);
  playheadRef.current = playhead;
  const playingRef = useRef(false);
  playingRef.current = playing;
  const durationRef = useRef(duration);
  durationRef.current = duration;

  const pause = useCallback(() => {
    setPlaying(false);
    if (native) void transportPause().catch(() => undefined);
  }, [native]);

  const play = useCallback(() => {
    const from = playheadRef.current;
    anchor.current = { position: from, at: performance.now() };
    if (native) void transportPlay(from).catch(() => undefined);
    setPlaying(true);
  }, [native]);

  const toggle = useCallback(() => {
    if (playingRef.current) pause();
    else play();
  }, [pause, play]);

  const seek = useCallback(
    (seconds: number) => {
      const next = Math.max(0, seconds);
      anchor.current = { position: next, at: performance.now() };
      setPlayhead(next);
      if (native) void transportSeek(next).catch(() => undefined);
    },
    [native],
  );

  const step = useCallback(
    (frames: number, frameRate: number) => {
      // Land on an exact frame boundary rather than adding an offset to
      // wherever the playhead happened to stop.
      const next = Math.max(0, (Math.round(playheadRef.current * frameRate) + frames) / frameRate);
      seek(next);
    },
    [seek],
  );

  // Engine position reports re-anchor the interpolation. Ignored while
  // paused: a report can arrive just after a local pause, and rewinding the
  // playhead to it would visibly snap.
  useEffect(() => {
    if (!native) return;
    let stopped = false;
    const stopping = listen<number>("transport", (event) => {
      if (stopped || !playingRef.current) return;
      anchor.current = { position: event.payload, at: performance.now() };
    });
    return () => {
      stopped = true;
      void stopping.then((stop) => stop());
    };
  }, [native]);

  // The render loop: interpolate from the anchor while playing.
  useEffect(() => {
    if (!playing) return;

    let frame = 0;
    const tick = (now: number) => {
      const next = anchor.current.position + (now - anchor.current.at) / 1000;
      const end = durationRef.current;

      if (end > 0 && next >= end) {
        setPlayhead(end);
        setPlaying(false);
        if (native) void transportPause().catch(() => undefined);
        return;
      }

      setPlayhead(next);
      frame = requestAnimationFrame(tick);
    };

    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing, native]);

  // Every method is a useCallback keyed only on `native`, so this memo holds
  // one identity for the hook's whole life - see the note on TransportControls.
  const controls = useMemo<TransportControls>(
    () => ({ play, pause, toggle, seek, step }),
    [play, pause, toggle, seek, step],
  );

  return { playhead, playing, controls };
}
