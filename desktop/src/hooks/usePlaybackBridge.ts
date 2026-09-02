// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The audio side of the engine bridge: what should be audible, and what the
 * engine says went wrong.
 *
 * The audible clip set is derived from the active timeline and replaced
 * wholesale in the engine's mixer, debounced so a drag does not thrash the
 * decoder. Playback failures come back as `audio://error` events and reach
 * the caller's toast - a mute app must never be a mystery.
 */
import { useEffect, useMemo } from "react";
import { isTauri } from "@tauri-apps/api/core";

import { findMedia, findTrack, type EditorProject, type TimelineData } from "../lib/editor";
import { audioSetClips, onAudioError } from "../lib/engine";
import { buildChain } from "../lib/filters";

export function usePlaybackBridge({
  project,
  timeline,
  projectPath,
  onError,
}: {
  project: EditorProject;
  timeline: TimelineData;
  projectPath: string;
  onError: (message: string) => void;
}) {
  const audibleClips = useMemo(
    () =>
      timeline.clips.flatMap((clip) => {
        if (clip.kind !== "audio" && clip.kind !== "video") return [];
        if (clip.kind === "video" && clip.muted) return [];
        const track = findTrack(project, clip.trackId);
        if (!track || track.muted) return [];
        const media = findMedia(project, clip.mediaId);
        if (!media || !media.hasAudio) return [];
        return [
          {
            path: media.path,
            start: clip.start,
            duration: clip.duration,
            sourceStart: clip.sourceStart,
            volume: clip.volume,
            fadeIn: clip.fadeIn,
            fadeOut: clip.fadeOut,
            speed: clip.speed,
            preservePitch: clip.preservePitch,
            chain: buildChain(clip.filters) ?? "",
          },
        ];
      }),
    [project, timeline],
  );

  useEffect(() => {
    if (!isTauri()) return;
    const timer = window.setTimeout(() => {
      void audioSetClips(projectPath, audibleClips).catch((cause: unknown) =>
        console.error("WolfCut: could not update the mix", cause),
      );
    }, 150);
    return () => window.clearTimeout(timer);
  }, [audibleClips, projectPath]);

  // Identical messages within a burst (one per clip of a broken import,
  // say) collapse into one.
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    let lastMessage = "";
    let lastAt = 0;
    void onAudioError((message) => {
      const now = Date.now();
      if (message === lastMessage && now - lastAt < 10_000) return;
      lastMessage = message;
      lastAt = now;
      onError(message);
    }).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
    // Subscribed once; the handler reaches state through onError.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
