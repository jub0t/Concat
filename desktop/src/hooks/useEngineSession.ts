// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The engine-owned editing session, as one hook.
 *
 * Owns everything between the window and `editor_*`: the serialised command
 * queue, the authoritative `EditorView`, the gesture echo and its commit,
 * undo/redo, the debounced autosave, and the output frame the preview footer
 * can edit. App.tsx renders what comes out; nothing else talks to the
 * session commands.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  activeTimeline,
  commandsForEcho,
  withEcho,
  type Clip,
  type Echo,
  type EditorCommand,
  type EditorProject,
  type EditorView,
} from "../lib/editor";
import {
  editorApply,
  editorClose,
  editorOpen,
  editorRedo,
  editorSave,
  editorUndo,
} from "../lib/engine";
import type { ProjectSession } from "../components/StartScreen";

/** What the editor renders for the frame or two before the session opens. */
const EMPTY_PROJECT: EditorProject = {
  media: [],
  fonts: [],
  timelines: [
    {
      id: "TL1",
      name: "Timeline 1",
      tracks: [1, 2, 3, 4].map((number) => ({
        id: `T${number}`,
        name: `Track ${number}`,
        visible: true,
        muted: false,
      })),
      clips: [],
    },
  ],
  activeTimelineId: "TL1",
};

export type SaveState = "idle" | "saving" | "saved" | "failed";

export function useEngineSession({
  session,
  onOpenError,
  onCommandError,
  onSaved,
}: {
  session: ProjectSession;
  /** The project could not be opened at all. */
  onOpenError: (message: string) => void;
  /** A command was refused; the message is the engine's own sentence. */
  onCommandError: (message: string) => void;
  /** An explicit save finished, one way or the other. */
  onSaved: (ok: boolean, message?: string) => void;
}) {
  const [view, setView] = useState<EditorView | null>(null);
  const [echo, setEcho] = useState<Echo | null>(null);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  /**
   * The output frame. Starts from the project settings but is editable from
   * the preview footer; the engine persists whatever this holds on save.
   */
  const [frame, setFrame] = useState({ width: session.width, height: session.height });

  const loaded = view !== null;
  // What the window draws: the engine's state with the gesture echo on top.
  const project = useMemo(() => withEcho(view?.project ?? EMPTY_PROJECT, echo), [view, echo]);

  const viewRef = useRef(view);
  viewRef.current = view;
  const echoRef = useRef(echo);
  echoRef.current = echo;
  const frameRef = useRef(frame);
  frameRef.current = frame;

  // ── the command queue ────────────────────────────────────────────────────
  // Commands are serialised so responses can never land out of order and
  // overwrite newer state with older. Errors surface through onCommandError -
  // they are user-meaningful sentences from the engine.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const dispatch = useCallback(
    (command: EditorCommand): Promise<string | undefined> => {
      const run = queue.current.then(async () => {
        const next = await editorApply(command);
        viewRef.current = next;
        setView(next);
        return next.createdId;
      });
      queue.current = run.catch(() => undefined);
      return run.catch((cause: unknown) => {
        onCommandError(String(cause));
        return undefined;
      });
    },
    [onCommandError],
  );

  const undoAction = useCallback(() => {
    queue.current = queue.current
      .then(async () => {
        const next = await editorUndo();
        viewRef.current = next;
        setView(next);
        setEcho(null);
      })
      .catch(() => undefined);
  }, []);

  const redoAction = useCallback(() => {
    queue.current = queue.current
      .then(async () => {
        const next = await editorRedo();
        viewRef.current = next;
        setView(next);
        setEcho(null);
      })
      .catch(() => undefined);
  }, []);

  // ── the gesture echo ─────────────────────────────────────────────────────

  /**
   * Live: merges a patch into the echo for one clip.
   *
   * The merge goes through `echoRef` and lands in it synchronously, not just
   * through a state updater: a discrete control - adding an effect, picking
   * a transition - echoes and commits in the same tick, and a commit that
   * read the ref before the next render would see the world without the
   * change it is supposed to commit. The engine never hearing about an
   * applied effect is exactly how the paused monitor ends up showing the
   * unprocessed frame.
   */
  const liveClip = useCallback((clipId: string, patch: Partial<Clip>) => {
    const current = echoRef.current;
    const next = {
      ...(current ?? {}),
      [clipId]: { ...(current?.[clipId] ?? {}), ...patch },
    };
    echoRef.current = next;
    setEcho(next);
  }, []);

  /**
   * Commit: everything echoed becomes engine commands - one per gesture is
   * the point, so undo undoes the drag rather than a pixel of it. Multi-clip
   * moves collapse into a single MoveClips; several commands land as one
   * batch, one undo step. The echo clears only after the engine's state
   * arrives, so nothing flashes back mid-flight.
   */
  const commitEcho = useCallback(() => {
    const pending = echoRef.current;
    const engineProject = viewRef.current?.project;
    if (!pending || !engineProject) return;

    const active = activeTimeline(engineProject);
    const commands: EditorCommand[] = [];
    const moves: { clipId: string; start: number; trackId: string }[] = [];

    for (const [clipId, patch] of Object.entries(pending)) {
      const base = active.clips.find((clip) => clip.id === clipId);
      if (!base) continue;
      for (const command of commandsForEcho(base, patch)) {
        if (command.op === "moveClips") moves.push(...command.moves);
        else commands.push(command);
      }
    }
    if (moves.length > 0) commands.push({ op: "moveClips", moves });

    if (commands.length === 0) {
      setEcho(null);
      return;
    }
    const command: EditorCommand =
      commands.length === 1 ? commands[0] : { op: "batch", commands };
    void dispatch(command).then(() => setEcho(null));
  }, [dispatch]);

  // ── the session itself ───────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    const opening = editorOpen({
      path: session.path,
      name: session.name,
      width: session.width,
      height: session.height,
      rateNum: session.rateNum,
      rateDen: session.rateDen,
    });
    // The engine installs the session only when the open resolves, but the
    // editor is already on screen taking gestures. Seeding the command queue
    // with the open makes anything dispatched early - a file dropped onto a
    // still-loading editor - wait for the session instead of bouncing off
    // "no project is open" (#11).
    queue.current = opening.catch(() => undefined);
    opening
      .then((opened) => {
        if (cancelled) return;
        viewRef.current = opened;
        setView(opened);
        setFrame({ width: opened.settings.width, height: opened.settings.height });
      })
      .catch((cause: unknown) => {
        if (!cancelled) onOpenError(String(cause));
      });
    return () => {
      cancelled = true;
      void editorClose().catch(() => undefined);
    };
    // Opened once per session; the error callback must not reopen it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session]);

  // Autosave, debounced. The engine writes the document; the UI only says
  // when, and carries the output frame the preview footer can edit.
  //
  // A failure toasts once per losing streak - the first failed autosave is
  // news the user must not miss (their edits are not reaching disk), but a
  // full disk would otherwise toast every keystroke and a half. The streak
  // resets on the next success so a recurrence speaks up again.
  const autosaveFailing = useRef(false);
  useEffect(() => {
    if (!loaded) return;
    const timer = window.setTimeout(() => {
      setSaveState("saving");
      editorSave(frameRef.current)
        .then(() => {
          setSaveState("saved");
          autosaveFailing.current = false;
        })
        .catch((cause: unknown) => {
          setSaveState("failed");
          if (!autosaveFailing.current) {
            autosaveFailing.current = true;
            onSaved(false, String(cause));
          }
        });
    }, 1500);
    return () => clearTimeout(timer);
  }, [view, frame, loaded, onSaved]);

  const saveAndNotify = useCallback(() => {
    setSaveState("saving");
    editorSave(frameRef.current)
      .then(() => {
        setSaveState("saved");
        onSaved(true);
      })
      .catch((cause: unknown) => {
        setSaveState("failed");
        onSaved(false, String(cause));
      });
     
  }, [onSaved]);

  return {
    view,
    /** Post-dispatch continuations read the freshest state through this. */
    viewRef,
    loaded,
    project,
    echo,
    setEcho,
    dispatch,
    undoAction,
    redoAction,
    liveClip,
    commitEcho,
    frame,
    setFrame,
    saveState,
    saveAndNotify,
  };
}
