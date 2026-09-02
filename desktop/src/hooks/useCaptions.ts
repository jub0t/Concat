// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Auto-captions: transcribe one clip's audio and lay the result out as text
 * clips on a "Captions" track (created on first use, reused after).
 *
 * The captions land as one batch - one round trip, one undo step - which is
 * why each addTextClip carries its own duration and placement.
 */
import { useCallback, useState } from "react";

import {
  activeTimeline,
  type Clip,
  type EditorCommand,
  type EditorProject,
  type MediaItem,
} from "../lib/editor";
import { transcribeClip } from "../lib/engine";
import { getTranscriberLanguage, getTranscriberModel } from "../lib/settings";
import { defaultTextStyle } from "../lib/text";

export function useCaptions({
  dispatch,
  getProject,
  onToast,
}: {
  dispatch: (command: EditorCommand) => Promise<string | undefined>;
  getProject: () => EditorProject;
  onToast: (message: string, failed: boolean) => void;
}) {
  const [transcribing, setTranscribing] = useState(false);

  const autoCaption = useCallback(
    async (clip: Clip, media: MediaItem) => {
      setTranscribing(true);
      onToast("Transcribing...", false);
      try {
        const segments = await transcribeClip({
          path: media.path,
          sourceStart: clip.sourceStart,
          window: clip.duration * clip.speed,
          language: getTranscriberLanguage(),
          modelId: getTranscriberModel(),
        });
        if (segments.length === 0) {
          onToast("No speech found", false);
          return;
        }

        let trackId = activeTimeline(getProject()).tracks.find(
          (track) => track.name === "Captions",
        )?.id;
        if (!trackId) {
          trackId = await dispatch({ op: "addTrack" });
          if (!trackId) return;
          await dispatch({ op: "renameTrack", trackId, name: "Captions" });
        }

        await dispatch({
          op: "batch",
          commands: segments.map((segment) => ({
            op: "addTextClip",
            trackId,
            start: clip.start + segment.start / clip.speed,
            duration: Math.max(0.4, (segment.end - segment.start) / clip.speed),
            offsetY: 0.38,
            // Caption-sized and lower-third, not title-sized and centred.
            style: { ...defaultTextStyle(), content: segment.text, fontSize: 0.045 },
          })),
        });
        onToast(
          `Added ${segments.length} caption${segments.length === 1 ? "" : "s"}`,
          false,
        );
      } catch (cause) {
        onToast(String(cause), true);
      } finally {
        setTranscribing(false);
      }
    },
    [dispatch, getProject, onToast],
  );

  return { autoCaption, transcribing };
}
