// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { ConfirmDialog } from "./components/ConfirmDialog";
import { ContextMenu, type ContextTarget } from "./components/ContextMenu";
import { ExportDialog } from "./components/ExportDialog";
import { TtsDialog } from "./components/TtsDialog";
import { Icon } from "./components/Icon";
import { ALL_MEDIA, MediaBin, type BinFilter } from "./components/MediaBin";
import type { MenuOption } from "./components/Menu";
import { ModifyProjectDialog } from "./components/ModifyProjectDialog";
import { Preview } from "./components/Preview";
import {
  exportTitlesOf,
  previewGhostAt,
  previewSourceAt,
  previewVeilAt,
  textOverlaysAt,
  type ExportTitle,
} from "./lib/monitor";
import { Resizer } from "./components/Resizer";
import { RightPanel, type RightTab } from "./components/RightPanel";
import { SaveTemplateDialog } from "./components/SaveTemplateDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { StartScreen, type ProjectSession } from "./components/StartScreen";
import { TitleBar } from "./components/TitleBar";
import { Toast } from "./components/Toast";
import { TimelinePanel, resolveDrop, type Tool } from "./components/TimelinePanel";
import { createAssets, requestAssets, requestVideoPeaks } from "./lib/assets";
import {
  activeTimeline,
  clipsAt,
  detachedAudioOf,
  findClip,
  findMedia,
  precedingClip,
  projectDuration,
  snapTime,
  speedPatch,
  timelineClipCount,
  transformPatch,
  trimPatch,
  whyNotMerge,
  type Clip,
  type MediaItem,
  type TimelineMeta,
} from "./lib/editor";
import {
  editorSave,
  engineVersion,
  newMediaFromSummary,
  probeMedia,
  readMediaBytes,
  templateSave,
  type TemplateInfo,
} from "./lib/engine";
import { findTransition } from "./lib/effects";
import { familyForPath, registerFont } from "./lib/text";
import { useLocale } from "./lib/i18n";
import { useCaptions } from "./hooks/useCaptions";
import { useEngineSession } from "./hooks/useEngineSession";
import { useEngineTruth } from "./hooks/useEngineTruth";
import { usePlaybackBridge } from "./hooks/usePlaybackBridge";
import { useTheme, type Theme } from "./lib/theme";
import { useTransport } from "./lib/transport";

const MIN_SECONDS_PER_PIXEL = 0.0005;
const MAX_SECONDS_PER_PIXEL = 2;

/**
 * The window: either the launch screen or an open editor.
 *
 * The editor is a separate component rather than a branch inside one, so that
 * every hook it owns - transport, asset cache, the engine session - mounts
 * when a project opens and unmounts when it closes.
 */
export function App() {
  const [session, setSession] = useState<ProjectSession | null>(null);
  // Choosing "use template" inside the editor closes the project and lands
  // the launch screen straight in that template's fill mode.
  const [pendingTemplate, setPendingTemplate] = useState<TemplateInfo | null>(null);
  // Owned here rather than in the editor so the launch screen is themed too,
  // and so the choice survives closing a project.
  const { theme, toggle } = useTheme();

  if (!session) {
    return (
      <div className="flex h-full flex-col overflow-hidden">
        <TitleBar projectName="" menus={[]} theme={theme} onToggleTheme={toggle} />
        <div className="min-h-0 flex-1">
          <StartScreen
            initialTemplate={pendingTemplate}
            onCreate={(next) => {
              setPendingTemplate(null);
              setSession(next);
            }}
          />
        </div>
      </div>
    );
  }

  return (
    <Editor
      session={session}
      theme={theme}
      onToggleTheme={toggle}
      onCloseProject={() => setSession(null)}
      onUseTemplate={(template) => {
        setPendingTemplate(template);
        setSession(null);
      }}
    />
  );
}

/**
 * The editor shell.
 *
 * The engine owns the edit (see engine decision 0007). This component holds
 * *window* state - selection, playhead, panel sizes - plus the latest
 * `EditorView` the engine returned, and a transient gesture echo: during a
 * drag the change previews locally with the engine's own arithmetic, and one
 * command commits on release. Every mutation is a command; every render draws
 * the state that came back.
 */
function Editor({
  session,
  theme,
  onToggleTheme,
  onCloseProject,
  onUseTemplate,
}: {
  session: ProjectSession;
  theme: Theme;
  onToggleTheme: () => void;
  onCloseProject: () => void;
  /** Leaves this project for the launch screen's fill flow on a template. */
  onUseTemplate: (template: TemplateInfo) => void;
}) {
  const { t, tp } = useLocale();
  const [toast, setToast] = useState<{ id: number; message: string; failed: boolean } | null>(
    null,
  );
  const pushToast = useCallback((message: string, failed: boolean) => {
    setToast({ id: Date.now(), message, failed });
  }, []);
  const [error, setError] = useState<string | null>(null);

  // The engine session: queue, echo, undo, autosave. See useEngineSession.
  const {
    view,
    viewRef,
    loaded,
    project,
    dispatch,
    undoAction,
    redoAction,
    liveClip,
    commitEcho,
    frame,
    setFrame,
    saveState,
    saveAndNotify,
  } = useEngineSession({
    session,
    onOpenError: setError,
    onCommandError: useCallback((message: string) => pushToast(message, true), [pushToast]),
    onSaved: useCallback(
      (ok: boolean, message?: string) =>
        pushToast(
          ok ? t("toast.projectSaved") : t("toast.saveFailed", { message: message ?? "" }),
          !ok,
        ),
      [pushToast, t],
    ),
  });

  // The project's display name. Seeded from the session, editable through
  // the Details panel's Modify dialog; the engine persists it on save.
  const [projectName, setProjectName] = useState(session.name);
  const [modifyingProject, setModifyingProject] = useState<null | { busy: boolean }>(null);

  const [selectedClipIds, setSelectedClipIds] = useState<string[]>([]);
  const [selectedMediaIds, setSelectedMediaIds] = useState<string[]>([]);
  const [tool, setTool] = useState<Tool>("select");
  const [snap, setSnap] = useState(true);
  const [binFilter, setBinFilter] = useState<BinFilter>(ALL_MEDIA);
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [context, setContext] = useState<ContextTarget | null>(null);
  const [mediaDrag, setMediaDrag] = useState<{ item: MediaItem; x: number; y: number } | null>(
    null,
  );
  const [exporting, setExporting] = useState(false);
  const [rightTab, setRightTab] = useState<RightTab>("details");
  /** Families whose font files failed to load - shown in the picker. */
  const [missingFonts, setMissingFonts] = useState<Set<string>>(new Set());

  // Panel geometry.
  // Wide enough that the library's five tabs and a row of cards breathe;
  // the resizer still allows 220-560.
  const [leftWidth, setLeftWidth] = useState(420);
  const [rightWidth, setRightWidth] = useState(300);
  const [timelineHeight, setTimelineHeight] = useState(340);

  // Timeline viewport.
  const [secondsPerPixel, setSecondsPerPixel] = useState(0.02);
  const [scrollLeft, setScrollLeft] = useState(0);
  // Vertical offset into the track stack, shared by the lanes and their headers.
  const [trackScroll, setTrackScroll] = useState(0);
  const zoomRef = useRef(secondsPerPixel);
  zoomRef.current = secondsPerPixel;

  const timeline = activeTimeline(project);

  const duration = projectDuration(project);
  // `transport` is the stable controls object - safe in deps and memoised
  // props without per-frame churn. Only `playhead`/`playing` move at
  // animation rate, and only Preview and TimelinePanel take them as props.
  const { playhead, playing, controls: transport } = useTransport({ duration });

  // Read by event listeners that outlive the render that installed them.
  const latest = useRef({ project, secondsPerPixel, scrollLeft, trackScroll, snap, playhead, frame });
  latest.current = { project, secondsPerPixel, scrollLeft, trackScroll, snap, playhead, frame };
  const selectedMediaRef = useRef(selectedMediaIds);
  selectedMediaRef.current = selectedMediaIds;

  // The project's rate is authoritative, not the first clip's.
  const frameRate = session.frameRate;

  // A selection can name clips an undo or a command just removed.
  useEffect(() => {
    setSelectedClipIds((ids) => {
      const kept = ids.filter((id) => timeline.clips.some((clip) => clip.id === id));
      return kept.length === ids.length ? ids : kept;
    });
    setSelectedMediaIds((ids) => {
      const kept = ids.filter((id) => project.media.some((item) => item.id === id));
      return kept.length === ids.length ? ids : kept;
    });
    // Keyed on the engine state, not the echo - the echo never removes clips.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view]);

  // Bring back every font the project names, once, when it opens. A font
  // whose file has moved is marked rather than dropped.
  useEffect(() => {
    if (!loaded || !view || view.project.fonts.length === 0) return;
    let cancelled = false;
    void Promise.all(
      view.project.fonts.map(async (font) => ({
        family: font.family,
        ok: await registerFont(font, readMediaBytes),
      })),
    ).then((results) => {
      if (cancelled) return;
      const failed = results.filter((result) => !result.ok).map((result) => result.family);
      if (failed.length > 0) setMissingFonts(new Set(failed));
    });
    return () => {
      cancelled = true;
    };
    // Deliberately keyed on `loaded` alone: fonts are registered once per open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  const fontsForUi = useMemo(
    () =>
      project.fonts.map((font) =>
        missingFonts.has(font.family) ? { ...font, missing: true } : font,
      ),
    [project.fonts, missingFonts],
  );

  const [settingsOpen, setSettingsOpen] = useState(false);
  /** The speech sheet: open with optional prefilled text and a fixed landing
   * time (a text clip's start), or null when closed. */
  const [speech, setSpeech] = useState<{ text?: string; at?: number } | null>(null);
  // The save-as-template sheet: null closed, otherwise whether the host is
  // packing the bundle right now.
  const [templateDialog, setTemplateDialog] = useState<null | { busy: boolean }>(null);

  useEffect(() => {
    engineVersion()
      .then(setVersion)
      .catch(() => setVersion("unavailable"));
  }, []);

  // Waveforms and filmstrips. A ref, not state: the timeline reads this map
  // from inside its draw loop, so artwork arriving needs no re-render.
  const assets = useRef(createAssets());

  // ── clip edits (echo + commit) ───────────────────────────────────────────

  /** Live patch from a panel control; committed by its onCommit. */
  const changeClip = useCallback(
    (patch: Partial<Clip>) => {
      const clipId = selectedClipIds.length === 1 ? selectedClipIds[0] : null;
      if (!clipId) return;
      // Transform-family fields get the engine's clamps applied to the echo,
      // so what previews is what will commit.
      const { scale, offsetX, offsetY, rotation, ...rest } = patch;
      const clamped =
        scale !== undefined || offsetX !== undefined || offsetY !== undefined || rotation !== undefined
          ? transformPatch({ scale, offsetX, offsetY, rotation })
          : {};
      liveClip(clipId, { ...rest, ...clamped });
    },
    [selectedClipIds, liveClip],
  );

  const changeSpeed = useCallback(
    (speed: number) => {
      const clipId = selectedClipIds.length === 1 ? selectedClipIds[0] : null;
      if (!clipId) return;
      const clip = findClip(latest.current.project, clipId);
      if (clip) liveClip(clipId, speedPatch(clip, speed));
    },
    [selectedClipIds, liveClip],
  );

  // ── media import ─────────────────────────────────────────────────────────
  const importPaths = useCallback(
    async (paths: string[]) => {
      setBusy(true);
      setError(null);
      for (const path of paths) {
        try {
          const summary = await probeMedia(path);
          const mediaId = await dispatch({
            op: "addMedia",
            item: newMediaFromSummary(summary),
          });
          if (mediaId) {
            setSelectedMediaIds([mediaId]);
            // Fire and forget: the clip draws flat until the artwork lands.
            requestAssets(
              assets.current,
              {
                id: mediaId,
                path: summary.path,
                kind: summary.kind,
                duration: summary.duration,
                hasAudio: summary.audio !== null,
              },
              session.path,
            );
          }
        } catch (cause) {
          setError(String(cause));
        }
      }
      setBusy(false);
    },
    [dispatch, session.path],
  );

  // A finished narration WAV: imported like any dropped file, then placed at
  // `at` - a text clip's start when spoken from one, the playhead otherwise.
  // Throwing lets the speech sheet show the failure in place.
  const insertNarration = useCallback(
    async (path: string, at?: number) => {
      const summary = await probeMedia(path);
      const mediaId = await dispatch({
        op: "addMedia",
        item: newMediaFromSummary(summary),
      });
      if (!mediaId) throw new Error(t("tts.importFailed"));
      requestAssets(
        assets.current,
        {
          id: mediaId,
          path: summary.path,
          kind: summary.kind,
          duration: summary.duration,
          hasAudio: summary.audio !== null,
        },
        session.path,
      );
      await dispatch({
        op: "addClipAtFirstFree",
        mediaId,
        start: at ?? latest.current.playhead,
      });
      pushToast(t("toast.narrationAdded"), false);
    },
    [dispatch, session.path, pushToast, t],
  );

  // Artwork for everything already in the project: a reopened project should
  // get its thumbnails and waveforms back without re-importing anything.
  useEffect(() => {
    if (!loaded) return;
    const wantsPeaks = new Set(
      timeline.clips.filter((clip) => clip.kind === "audio").map((clip) => clip.mediaId),
    );
    for (const item of project.media) {
      requestAssets(assets.current, item, session.path);
      if (item.kind === "video" && wantsPeaks.has(item.id)) {
        requestVideoPeaks(assets.current, item, session.path);
      }
    }
  }, [loaded, project.media, timeline.clips, session.path]);

  // Files dropped from the OS.
  useEffect(() => {
    if (!isTauri()) return;
    let stop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") setDropping(true);
        else if (event.payload.type === "drop") {
          setDropping(false);
          void importPaths(event.payload.paths);
        } else setDropping(false);
      })
      .then((unlisten) => {
        stop = unlisten;
      });
    return () => stop?.();
  }, [importPaths]);

  // ── audio playback ───────────────────────────────────────────────────────
  usePlaybackBridge({
    project,
    timeline,
    projectPath: session.path,
    onError: useCallback((message: string) => pushToast(message, true), [pushToast]),
  });

  // ── text, effects, transitions ───────────────────────────────────────────

  const addText = useCallback(() => {
    void dispatch({
      op: "addTextClip",
      trackId: null,
      start: latest.current.playhead,
      style: null,
    }).then((clipId) => {
      if (clipId) {
        setSelectedClipIds([clipId]);
        setSelectedMediaIds([]);
        setRightTab("text");
      }
    });
  }, [dispatch]);

  const pickFont = useCallback(async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        title: t("dialog.addFont.title"),
        filters: [
          { name: t("dialog.addFont.filter"), extensions: ["ttf", "otf", "woff", "woff2", "ttc"] },
        ],
      });
      if (typeof picked !== "string") return;
      const current = latest.current.project;
      if (current.fonts.some((font) => font.path === picked)) return;

      const font = {
        family: familyForPath(
          picked,
          current.fonts.map((existing) => existing.family),
        ),
        path: picked,
      };
      // Registered before it is stored, so a file the webview cannot parse is
      // reported now rather than becoming a broken entry in the picker.
      if (!(await registerFont(font, readMediaBytes))) {
        setError(t("error.fontUnreadable", { file: picked.split(/[/\\]/).pop() ?? picked }));
        return;
      }
      void dispatch({ op: "addFont", family: font.family, path: font.path });
    } catch (cause) {
      setError(String(cause));
    }
  }, [dispatch, t]);

  const applyEffect = useCallback(
    (effectId: string) => {
      const current = latest.current.project;
      const selected = selectedClipIds.length === 1 ? findClip(current, selectedClipIds[0]) : null;
      if (!selected || (selected.kind !== "video" && selected.kind !== "image")) {
        setToast({
          id: Date.now(),
          message: t("toast.effectNeedsClip"),
          failed: true,
        });
        return;
      }
      void dispatch({
        op: "updateClip",
        clipId: selected.id,
        patch: { videoEffects: [...selected.videoEffects, { id: effectId, params: {} }] },
      });
      setRightTab("effects");
    },
    [selectedClipIds, dispatch, t],
  );

  const applyTransition = useCallback(
    (transitionId: string) => {
      const definition = findTransition(transitionId);
      if (!definition?.implemented) return;
      const current = latest.current.project;
      const selected = selectedClipIds.length === 1 ? findClip(current, selectedClipIds[0]) : null;
      if (!selected || (selected.kind !== "video" && selected.kind !== "image")) {
        setToast({
          id: Date.now(),
          message: t("toast.transitionNeedsClip"),
          failed: true,
        });
        return;
      }
      if (!precedingClip(current, selected.id)) {
        setToast({
          id: Date.now(),
          message: t("toast.transitionNeedsCut"),
          failed: true,
        });
        return;
      }
      void dispatch({
        op: "updateClip",
        clipId: selected.id,
        patch: { transitionIn: { id: transitionId, duration: definition.defaultDuration } },
      });
      setRightTab("effects");
      setToast({
        id: Date.now(),
        message: t("toast.transitionAdded", { transition: definition.label }),
        failed: false,
      });
    },
    [selectedClipIds, dispatch, t],
  );

  const { autoCaption, transcribing } = useCaptions({
    dispatch,
    getProject: useCallback(() => latest.current.project, []),
    onToast: pushToast,
  });

  /** The clip tools dropdown. Hidden for clips with nothing to offer. */
  const clipTools = useMemo<MenuOption[][]>(() => {
    const clip = selectedClipIds.length === 1 ? findClip(project, selectedClipIds[0]) : null;
    if (!clip) return [];

    // A title's one tool: speak its text. The sheet opens prefilled, and the
    // narration lands where the title starts.
    if (clip.kind === "text") {
      return [
        [
          {
            label: t("menu.clip.generateVoice"),
            icon: "volume" as const,
            onSelect: () => setSpeech({ text: clip.text?.content ?? "", at: clip.start }),
          },
        ],
      ];
    }

    const media = findMedia(project, clip.mediaId);
    if (clip.kind !== "video" && clip.kind !== "audio") return [];

    const hasSound = Boolean(media?.hasAudio);
    const canDetach = Boolean(
      clip.kind === "video" &&
        !clip.muted &&
        media?.hasAudio &&
        detachedAudioOf(project, clip.id).length === 0,
    );
    const canReattach = Boolean(
      (clip.kind === "video" && detachedAudioOf(project, clip.id).length > 0) ||
        (clip.kind === "audio" && clip.detachedFrom && findClip(project, clip.detachedFrom)),
    );

    return [
      [
        {
          label: transcribing ? t("menu.clip.transcribing") : t("menu.clip.autoCaptions"),
          icon: "type" as const,
          disabled: !hasSound || transcribing,
          onSelect: () => {
            if (clip && media) void autoCaption(clip, media);
          },
        },
      ],
      [
        {
          label: t("menu.clip.detachAudio"),
          icon: "waveform" as const,
          disabled: !canDetach,
          onSelect: () => void dispatch({ op: "detachAudio", clipId: clip.id }),
        },
        {
          label: t("menu.clip.reattachAudio"),
          icon: "merge" as const,
          disabled: !canReattach,
          onSelect: () => {
            void dispatch({ op: "reattachAudio", clipId: clip.id });
            setSelectedClipIds([]);
          },
        },
      ],
    ];
  }, [project, selectedClipIds, transcribing, autoCaption, dispatch, t]);

  // ── timelines ────────────────────────────────────────────────────────────
  const [timelineToDelete, setTimelineToDelete] = useState<TimelineMeta | null>(null);
  const playheadByTimeline = useRef(new Map<string, number>());

  const selectTimeline = useCallback(
    (timelineId: string) => {
      const current = latest.current.project;
      if (timelineId === current.activeTimelineId) return;
      playheadByTimeline.current.set(current.activeTimelineId, latest.current.playhead);
      void dispatch({ op: "selectTimeline", timelineId }).then(() => {
        setSelectedClipIds([]);
        transport.pause();
        transport.seek(playheadByTimeline.current.get(timelineId) ?? 0);
      });
    },
    [dispatch, transport],
  );

  const createTimeline = useCallback(() => {
    playheadByTimeline.current.set(
      latest.current.project.activeTimelineId,
      latest.current.playhead,
    );
    void dispatch({ op: "addTimeline" }).then(() => {
      setSelectedClipIds([]);
      transport.pause();
      transport.seek(0);
    });
  }, [dispatch, transport]);

  const deleteTimelineNow = useCallback(
    (timelineId: string) => {
      const wasActive = timelineId === latest.current.project.activeTimelineId;
      playheadByTimeline.current.delete(timelineId);
      void dispatch({ op: "removeTimeline", timelineId }).then(() => {
        if (wasActive) {
          setSelectedClipIds([]);
          transport.pause();
          const nextActive = viewRef.current?.project.activeTimelineId;
          transport.seek((nextActive && playheadByTimeline.current.get(nextActive)) || 0);
        }
      });
    },
    [dispatch, transport, viewRef],
  );

  // ── edit operations ──────────────────────────────────────────────────────
  const addToTimeline = useCallback(
    (mediaId: string) => {
      void dispatch({
        op: "addClipAtFirstFree",
        mediaId,
        start: latest.current.playhead,
      }).then((clipId) => {
        if (clipId) setSelectedClipIds([clipId]);
      });
    },
    [dispatch],
  );

  /**
   * Places several bin items end to end from `start`, as one batch and one
   * undo step - the multi-select drop and the File menu's add-selected.
   * With a track they land on that lane; without one each finds the first
   * free lane for its own span, like a double-click does.
   */
  const placeMediaSet = useCallback(
    (mediaIds: string[], start: number, trackId: string | null) => {
      const current = latest.current.project;
      let at = Math.max(0, start);
      const commands = mediaIds.flatMap((mediaId) => {
        const media = findMedia(current, mediaId);
        if (!media) return [];
        const from = at;
        // The engine's own placement defaults (wolfcut-project: a still lasts
        // 5s, unknown-duration media falls back to 5s) - mirrored here only
        // to lay items end to end; the engine still decides each clip's
        // real duration.
        at += media.kind === "image" ? 5 : (media.duration ?? 5);
        return [
          trackId
            ? ({ op: "addClip", mediaId, trackId, start: from } as const)
            : ({ op: "addClipAtFirstFree", mediaId, start: from } as const),
        ];
      });
      if (commands.length === 0) return;
      void dispatch(
        commands.length === 1 ? commands[0] : { op: "batch", commands: [...commands] },
      ).then((clipId) => {
        if (clipId) setSelectedClipIds([clipId]);
      });
    },
    [dispatch],
  );

  const beginMediaDrag = useCallback(
    (item: MediaItem, clientX: number, clientY: number) => {
      setMediaDrag({ item, x: clientX, y: clientY });

      const onMove = (event: PointerEvent) => {
        setMediaDrag((current) =>
          current ? { ...current, x: event.clientX, y: event.clientY } : null,
        );
      };
      const onUp = (event: PointerEvent) => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        setMediaDrag(null);

        const state = latest.current;
        const drop = resolveDrop(event.clientX, event.clientY, {
          tracks: activeTimeline(state.project).tracks,
          secondsPerPixel: state.secondsPerPixel,
          scrollLeft: state.scrollLeft,
          trackScroll: state.trackScroll,
        });
        if (!drop) return;

        const start = state.snap
          ? snapTime(state.project, drop.start, {
              threshold: 8 * state.secondsPerPixel,
              playhead: state.playhead,
            })
          : drop.start;
        const set = selectedMediaRef.current;
        const dragged = set.includes(item.id) && set.length > 1 ? set : [item.id];
        placeMediaSet(dragged, Math.max(0, start), drop.trackId);
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [placeMediaSet],
  );

  const splitAtPlayhead = useCallback(() => {
    const current = latest.current.project;
    const at = latest.current.playhead;
    const targets =
      selectedClipIds.length > 0
        ? selectedClipIds
        : clipsAt(current, at).map((clip) => clip.id);
    if (targets.length === 0) return;
    void dispatch({ op: "splitClips", clipIds: targets, time: at });
  }, [selectedClipIds, dispatch]);

  const mergeSelected = useCallback(() => {
    void dispatch({ op: "mergeClips", clipIds: selectedClipIds }).then((clipId) => {
      if (clipId) setSelectedClipIds([clipId]);
    });
  }, [selectedClipIds, dispatch]);

  /** Deletes the bin selection as one undo step; removal cascades clips. */
  const deleteSelectedMedia = useCallback(() => {
    const ids = selectedMediaRef.current;
    if (ids.length === 0) return;
    const commands = ids.map((mediaId) => ({ op: "removeMedia", mediaId }) as const);
    void dispatch(
      commands.length === 1 ? commands[0] : { op: "batch", commands: [...commands] },
    );
    setSelectedMediaIds([]);
  }, [dispatch]);

  const deleteSelected = useCallback(() => {
    if (selectedClipIds.length > 0) {
      void dispatch({ op: "removeClips", clipIds: selectedClipIds });
      setSelectedClipIds([]);
      return;
    }
    // No clips selected: Delete acts on the bin selection instead.
    deleteSelectedMedia();
  }, [selectedClipIds, deleteSelectedMedia, dispatch]);

  const zoom = useCallback((factor: number, anchor?: number) => {
    const previous = zoomRef.current;
    const next = Math.min(
      MAX_SECONDS_PER_PIXEL,
      Math.max(MIN_SECONDS_PER_PIXEL, previous * factor),
    );
    zoomRef.current = next;
    setSecondsPerPixel(next);
    if (anchor !== undefined) {
      setScrollLeft((left) => Math.max(0, anchor - (anchor - left) * (next / previous)));
    }
  }, []);

  const fit = useCallback(
    (canvasWidth: number) => {
      if (canvasWidth <= 0) return;
      const span = Math.max(duration, 1) * 1.05;
      const next = Math.min(
        MAX_SECONDS_PER_PIXEL,
        Math.max(MIN_SECONDS_PER_PIXEL, span / canvasWidth),
      );
      zoomRef.current = next;
      setSecondsPerPixel(next);
      setScrollLeft(0);
    },
    [duration],
  );

  // ── stable prop identities ───────────────────────────────────────────────
  // MediaBin, RightPanel and the TitleBar are React.memo'd so they sit out
  // the per-frame renders playback causes here. That only holds if every
  // callback they receive keeps its identity - an inline arrow minted per
  // render would make the memo a no-op - so their handlers live here as
  // useCallbacks over stable dependencies.

  const selectMedia = useCallback((ids: string[]) => {
    setSelectedMediaIds(ids);
    // Bin and timeline selections are exclusive: picking media drops clips.
    if (ids.length > 0) setSelectedClipIds([]);
  }, []);

  const importFiles = useCallback(
    (paths: string[]) => void importPaths(paths),
    [importPaths],
  );

  const removeMedia = useCallback(
    (id: string) => void dispatch({ op: "removeMedia", mediaId: id }),
    [dispatch],
  );

  const dismissError = useCallback(() => setError(null), []);

  const toggleSlot = useCallback(
    (mediaId: string, placeholder: boolean) =>
      void dispatch({ op: "setMediaPlaceholder", mediaId, placeholder }),
    [dispatch],
  );

  const openTemplateDialog = useCallback(() => setTemplateDialog({ busy: false }), []);

  const leaveForTemplate = useCallback(
    (template: TemplateInfo) => {
      // Save what is open first: leaving for the fill flow closes this
      // project, and the debounced autosave may not have fired.
      void editorSave(latest.current.frame)
        .catch(() => undefined)
        .then(() => onUseTemplate(template));
    },
    [onUseTemplate],
  );

  const addFont = useCallback(() => void pickFont(), [pickFont]);

  const removeFont = useCallback(
    (family: string) => void dispatch({ op: "removeFont", family }),
    [dispatch],
  );

  const openModifyProject = useCallback(() => setModifyingProject({ busy: false }), []);
  const openSettings = useCallback(() => setSettingsOpen(true), []);
  const openSpeech = useCallback(() => setSpeech({}), []);
  const openExport = useCallback(() => setExporting(true), []);

  // ── keyboard ─────────────────────────────────────────────────────────────
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }

      if (event.ctrlKey || event.metaKey) {
        switch (event.code) {
          case "KeyZ":
            event.preventDefault();
            if (event.shiftKey) redoAction();
            else undoAction();
            break;
          case "KeyY":
            event.preventDefault();
            redoAction();
            break;
          case "KeyS":
            event.preventDefault();
            saveAndNotify();
            break;
          case "KeyB":
            event.preventDefault();
            splitAtPlayhead();
            break;
          case "KeyE":
            event.preventDefault();
            setExporting(true);
            break;
          default:
            break;
        }
        return;
      }

      if (event.altKey) return;

      const step = event.shiftKey ? 10 : 1;
      switch (event.code) {
        case "Space":
          event.preventDefault();
          transport.toggle();
          break;
        case "ArrowLeft":
          transport.step(-step, frameRate);
          break;
        case "ArrowRight":
          transport.step(step, frameRate);
          break;
        case "Home":
          transport.seek(0);
          break;
        case "End":
          transport.seek(duration);
          break;
        case "KeyV":
          setTool("select");
          break;
        case "KeyC":
          setTool("razor");
          break;
        case "KeyN":
          setSnap((current) => !current);
          break;
        case "KeyF": {
          // The same marker resolveDrop uses to find the timeline canvas -
          // its width is a layout fact only that element knows.
          const canvas = document.querySelector<HTMLCanvasElement>("[data-wolfcut-timeline]");
          if (canvas) fit(canvas.clientWidth);
          break;
        }
        case "KeyS":
          splitAtPlayhead();
          break;
        case "KeyM":
          mergeSelected();
          break;
        case "Delete":
        case "Backspace":
          deleteSelected();
          break;
        default:
          break;
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    deleteSelected,
    duration,
    fit,
    frameRate,
    mergeSelected,
    redoAction,
    saveAndNotify,
    splitAtPlayhead,
    transport,
    undoAction,
  ]);

  // ── derived views ────────────────────────────────────────────────────────

  // Derived views of the monitor and exporter, all pure in lib/monitor.ts -
  // which is what makes the cross-fade pre-roll and the flattening testable.
  const textOverlays = useMemo(
    () => textOverlaysAt(project, timeline, playhead),
    [project, timeline, playhead],
  );

  const previewSource = useMemo(
    () => previewSourceAt(project, timeline, playhead),
    [project, timeline, playhead],
  );

  const previewClip = previewSource ? findClip(project, previewSource.clipId) : null;
  const previewMedia = previewClip ? findMedia(project, previewClip.mediaId) : null;

  const previewGhost = useMemo(
    () => previewGhostAt(project, timeline, playhead),
    [project, timeline, playhead],
  );

  const previewVeil = useMemo(
    () => previewVeilAt(project, timeline, playhead),
    [project, timeline, playhead],
  );

  // The exporter flattens the engine's own session (decision 0009); the UI
  // only counts renderable clips to know whether there is anything to show,
  // and rasterises its titles.
  const renderableClipCount = useMemo(
    () => timeline.clips.filter((clip) => clip.kind !== "text").length,
    [timeline],
  );

  const exportTitles = useMemo<ExportTitle[]>(
    () => exportTitlesOf(project, timeline),
    [project, timeline],
  );

  // The engine's true frames: paused dwell + playback stream. See
  // useEngineTruth and desktop decision 0009. Quality is the footer's
  // dropdown: what fraction of the output frame the engine composites.
  const [previewQuality, setPreviewQuality] = useState(0.5);
  const engineStill = useEngineTruth({
    playing,
    loaded,
    playhead,
    project,
    renderableClipCount,
    frame,
    fps: session ? session.rateNum / session.rateDen : 30,
    quality: previewQuality,
    latest,
  });

  const selectedClip =
    selectedClipIds.length === 1 ? findClip(project, selectedClipIds[0]) : null;
  const inspectorMedia = selectedClip
    ? findMedia(project, selectedClip.mediaId)
    : selectedMediaIds.length === 1
      ? findMedia(project, selectedMediaIds[0])
      : null;

  // Walks every selected clip's neighbours, so not something to redo per
  // frame of playback - only an edit or a selection change moves the answer.
  const mergeBlockedBecause = useMemo(
    () => whyNotMerge(project, selectedClipIds),
    [project, selectedClipIds],
  );

  // The Export button, built once so the memoised TitleBar sees the same
  // `actions` element across playback renders.
  const exportAction = useMemo(
    () => (
      <button
        type="button"
        onClick={openExport}
        className="flex cursor-pointer items-center gap-1.5 rounded-md bg-accent px-2.5 py-1
                   text-xs font-medium text-on-accent transition-colors hover:bg-accent-hover"
      >
        <Icon name="export" size={13} />
        {t("common.export")}
      </button>
    ),
    [openExport, t],
  );

  // The whole menu structure, rebuilt only when something a menu shows or
  // does actually changes. Building it inline in the JSX minted a new tree
  // per render - 60 times a second during playback - and dragged the
  // memoised TitleBar along with it. Nothing here reads the playhead (the
  // File menu's "add at playhead" goes through `latest`), so playback never
  // invalidates it.
  const menus = useMemo<{ label: string; groups: MenuOption[][] }[]>(
    () => [
      {
        label: t("menu.file.title"),
        groups: [
          [
            {
              label: t("menu.file.addSelected"),
              icon: "plus",
              disabled: selectedMediaIds.length === 0,
              onSelect: () => placeMediaSet(selectedMediaIds, latest.current.playhead, null),
            },
            {
              label: t("menu.file.save"),
              icon: "folder",
              hint: "Ctrl+S",
              onSelect: () => saveAndNotify(),
            },
            {
              label: t("menu.file.export"),
              icon: "export",
              disabled: renderableClipCount === 0,
              onSelect: openExport,
            },
            {
              label: t("menu.file.saveAsTemplate"),
              icon: "slot",
              onSelect: openTemplateDialog,
            },
            {
              label: t("menu.file.speech"),
              icon: "volume",
              onSelect: openSpeech,
            },
          ],
          [
            {
              label: t("menu.file.settings"),
              icon: "settings",
              onSelect: openSettings,
            },
          ],
          [
            {
              label: t("menu.file.closeProject"),
              icon: "folder",
              onSelect: onCloseProject,
            },
            {
              label: t("menu.file.closeWindow"),
              icon: "close",
              onSelect: () => void getCurrentWindow().close(),
              danger: true,
            },
          ],
        ],
      },
      {
        label: t("menu.edit.title"),
        groups: [
          [
            {
              label: t("menu.edit.undo"),
              icon: "chevronUp",
              hint: "Ctrl+Z",
              disabled: !view?.canUndo,
              onSelect: undoAction,
            },
            {
              label: t("menu.edit.redo"),
              icon: "chevronDown",
              hint: "Ctrl+Shift+Z",
              disabled: !view?.canRedo,
              onSelect: redoAction,
            },
          ],
          [
            {
              label: t("menu.edit.splitAtPlayhead"),
              icon: "razor",
              hint: "Ctrl+B",
              onSelect: splitAtPlayhead,
            },
            {
              label: tp("menu.edit.deleteClips", Math.max(1, selectedClipIds.length)),
              icon: "trash",
              hint: "Del",
              disabled: selectedClipIds.length === 0,
              onSelect: deleteSelected,
              danger: true,
            },
          ],
          [
            {
              label: snap ? t("menu.edit.disableSnapping") : t("menu.edit.enableSnapping"),
              icon: "magnet",
              hint: "N",
              onSelect: () => setSnap((current) => !current),
            },
          ],
        ],
      },
      {
        label: t("menu.view.title"),
        groups: [
          [
            { label: t("menu.view.zoomIn"), icon: "plus", onSelect: () => zoom(1 / 1.4) },
            { label: t("menu.view.zoomOut"), icon: "minus", onSelect: () => zoom(1.4) },
          ],
          [
            {
              label: t("menu.view.goToStart"),
              icon: "skipStart",
              hint: "Home",
              onSelect: () => transport.seek(0),
            },
            {
              label: t("menu.view.goToEnd"),
              icon: "skipEnd",
              hint: "End",
              onSelect: () => transport.seek(duration),
            },
          ],
        ],
      },
    ],
    [
      deleteSelected,
      duration,
      renderableClipCount,
      onCloseProject,
      openExport,
      openSettings,
      openSpeech,
      openTemplateDialog,
      placeMediaSet,
      redoAction,
      saveAndNotify,
      selectedClipIds,
      selectedMediaIds,
      snap,
      splitAtPlayhead,
      t,
      tp,
      transport,
      undoAction,
      view,
      zoom,
    ],
  );

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <TitleBar
        projectName={projectName}
        status={
          saveState === "saving"
            ? t("status.saving")
            : saveState === "failed"
              ? t("status.saveFailed")
              : saveState === "saved"
                ? t("status.saved")
                : version === "unavailable"
                  ? t("status.engineUnavailable")
                  : version
                    ? t("status.engine", { version })
                    : undefined
        }
        theme={theme}
        onToggleTheme={onToggleTheme}
        onOpenSettings={openSettings}
        actions={exportAction}
        menus={menus}
      />

      <div className="flex min-h-0 flex-1 flex-col p-2">
        <div className="flex min-h-0 flex-1">
          <div style={{ width: leftWidth }} className="min-w-0 shrink-0">
            <MediaBin
              items={project.media}
              assets={assets.current}
              dropping={dropping}
              filter={binFilter}
              selectedIds={selectedMediaIds}
              busy={busy}
              error={error}
              onFilter={setBinFilter}
              onAddText={addText}
              onSelect={selectMedia}
              onImport={importFiles}
              onRemove={removeMedia}
              onRemoveSelected={deleteSelectedMedia}
              onDismissError={dismissError}
              onBeginDrag={beginMediaDrag}
              onAddToTimeline={addToTimeline}
              onApplyEffect={applyEffect}
              onApplyTransition={applyTransition}
              onToggleSlot={toggleSlot}
              onSaveTemplate={openTemplateDialog}
              onUseTemplate={leaveForTemplate}
            />
          </div>

          <Resizer
            direction="vertical"
            onResize={(delta) =>
              setLeftWidth((width) => Math.min(560, Math.max(220, width + delta)))
            }
          />

          <div className="min-w-0 flex-1">
            <Preview
              source={previewSource}
              overlays={textOverlays}
              playing={playing}
              playhead={playhead}
              duration={duration}
              frameRate={frameRate}
              frame={frame}
              quality={previewQuality}
              onQualityChange={setPreviewQuality}
              transform={
                previewClip
                  ? {
                      scale: previewClip.scale,
                      offsetX: previewClip.offsetX,
                      offsetY: previewClip.offsetY,
                      rotation: previewClip.rotation,
                    }
                  : null
              }
              opacity={previewClip?.opacity ?? 1}
              effects={previewClip?.videoEffects ?? null}
              ghost={previewGhost}
              engineStill={engineStill}
              veil={previewVeil}
              mediaSize={
                previewMedia && previewMedia.width && previewMedia.height
                  ? { width: previewMedia.width, height: previewMedia.height }
                  : null
              }
              selectedClipId={selectedClipIds.length === 1 ? selectedClipIds[0] : null}
              onSelectClip={(clipId) => setSelectedClipIds([clipId])}
              onTransformChange={(clipId, transform) =>
                liveClip(clipId, transformPatch(transform))
              }
              onTransformEnd={commitEcho}
              onOverlayChange={(clipId, change) => {
                const patch: Partial<Clip> = transformPatch({
                  offsetX: change.offsetX,
                  offsetY: change.offsetY,
                });
                if (change.fontSize !== undefined) {
                  const clip = findClip(latest.current.project, clipId);
                  if (clip?.text) patch.text = { ...clip.text, fontSize: change.fontSize };
                }
                liveClip(clipId, patch);
              }}
              onOverlayEnd={commitEcho}
              onFrameChange={(width, height) => setFrame({ width, height })}
              onTogglePlay={transport.toggle}
              onStep={(frames) => transport.step(frames, frameRate)}
              onSeek={transport.seek}
            />
          </div>

          <Resizer
            direction="vertical"
            onResize={(delta) =>
              setRightWidth((width) => Math.min(520, Math.max(220, width - delta)))
            }
          />

          <div style={{ width: rightWidth }} className="min-w-0 shrink-0">
            <RightPanel
              tab={rightTab}
              onTab={setRightTab}
              clip={selectedClip}
              media={inspectorMedia}
              project={project}
              fonts={fontsForUi}
              projectName={projectName}
              projectPath={session.path}
              frame={frame}
              duration={duration}
              frameRate={frameRate}
              onAddFont={addFont}
              onRemoveFont={removeFont}
              onChangeClip={changeClip}
              onCommitClip={commitEcho}
              onSpeedChange={changeSpeed}
              onModifyProject={openModifyProject}
            />
          </div>
        </div>

        <Resizer
          direction="horizontal"
          onResize={(delta) =>
            setTimelineHeight((height) => Math.min(700, Math.max(160, height - delta)))
          }
        />

        <div style={{ height: timelineHeight }} className="min-h-0 shrink-0">
          <TimelinePanel
            project={project}
            playhead={playhead}
            playing={playing}
            frameRate={frameRate}
            tool={tool}
            snap={snap}
            selectedClipIds={selectedClipIds}
            secondsPerPixel={secondsPerPixel}
            scrollLeft={scrollLeft}
            trackScroll={trackScroll}
            assets={assets.current}
            theme={theme}
            onToolChange={setTool}
            onSnapChange={setSnap}
            onScrub={transport.seek}
            onSelectClips={setSelectedClipIds}
            onMoveClips={(moves) => {
              for (const move of moves) {
                liveClip(move.clipId, { start: Math.max(0, move.start), trackId: move.trackId });
              }
            }}
            onTrimClip={(clipId, edge, delta) => {
              const clip = findClip(latest.current.project, clipId);
              if (clip) liveClip(clipId, trimPatch(clip, edge, delta));
            }}
            onGestureEnd={commitEcho}
            onSplitAtPlayhead={splitAtPlayhead}
            onMergeSelected={mergeSelected}
            mergeBlockedBecause={mergeBlockedBecause}
            onDeleteSelected={deleteSelected}
            mediaDrag={mediaDrag ? { x: mediaDrag.x, y: mediaDrag.y } : null}
            onZoom={zoom}
            onScroll={setScrollLeft}
            onTrackScroll={setTrackScroll}
            onFit={fit}
            onTrackFlag={(trackId, flag, value) =>
              void dispatch({ op: "setTrackFlag", trackId, flag, value })
            }
            clipTools={clipTools}
            onSelectTimeline={selectTimeline}
            onAddTimeline={createTimeline}
            onRenameTimeline={(timelineId, name) =>
              void dispatch({ op: "renameTimeline", timelineId, name })
            }
            onMoveTimeline={(timelineId, index) =>
              void dispatch({ op: "moveTimeline", timelineId, index })
            }
            onRequestRemoveTimeline={(timelineId) => {
              const meta = project.timelines.find((candidate) => candidate.id === timelineId);
              if (meta) setTimelineToDelete({ id: meta.id, name: meta.name });
            }}
            onAddTrack={() => void dispatch({ op: "addTrack" })}
            onRenameTrack={(trackId, name) =>
              void dispatch({ op: "renameTrack", trackId, name })
            }
            onRemoveTrack={(trackId) => void dispatch({ op: "removeTrack", trackId })}
            onClipContextMenu={(clipId, x, y) => {
              const target = selectedClipIds.includes(clipId) ? selectedClipIds : [clipId];
              setSelectedClipIds(target);

              const clip = findClip(project, clipId);
              const many = target.length > 1;

              setContext({
                x,
                y,
                // Related actions share a group; the menu draws a divider
                // between groups and drops any that come up empty.
                groups: [
                  // Cutting.
                  [
                    {
                      label: tp("contextMenu.splitClips", target.length),
                      icon: "razor",
                      hint: "S",
                      onSelect: splitAtPlayhead,
                    },
                    ...(whyNotMerge(project, target) === null
                      ? [
                          {
                            label: tp("contextMenu.mergeClips", target.length),
                            icon: "merge" as const,
                            hint: "M",
                            onSelect: mergeSelected,
                          },
                        ]
                      : []),
                  ],
                  // Generation: captions out of speech, speech out of titles.
                  [
                    ...(() => {
                      const media = clip ? findMedia(project, clip.mediaId) : null;
                      return !many &&
                        clip &&
                        (clip.kind === "video" || clip.kind === "audio") &&
                        media?.hasAudio &&
                        !transcribing
                        ? [
                            {
                              label: t("menu.clip.autoCaptions"),
                              icon: "type" as const,
                              onSelect: () => void autoCaption(clip, media),
                            },
                          ]
                        : [];
                    })(),
                    ...(!many && clip?.kind === "text"
                      ? [
                          {
                            label: t("menu.clip.generateVoice"),
                            icon: "volume" as const,
                            onSelect: () =>
                              setSpeech({ text: clip.text?.content ?? "", at: clip.start }),
                          },
                        ]
                      : []),
                  ],
                  // The audio attached to a video clip.
                  [
                    ...(!many &&
                    clip?.kind === "video" &&
                    !clip.muted &&
                    findMedia(project, clip.mediaId)?.hasAudio &&
                    detachedAudioOf(project, clip.id).length === 0
                      ? [
                          {
                            label: t("menu.clip.detachAudio"),
                            icon: "waveform" as const,
                            onSelect: () => void dispatch({ op: "detachAudio", clipId }),
                          },
                        ]
                      : []),
                    ...(!many &&
                    clip &&
                    ((clip.kind === "video" && detachedAudioOf(project, clip.id).length > 0) ||
                      (clip.kind === "audio" &&
                        clip.detachedFrom &&
                        findClip(project, clip.detachedFrom)))
                      ? [
                          {
                            label: t("menu.clip.reattachAudio"),
                            icon: "merge" as const,
                            onSelect: () => {
                              void dispatch({ op: "reattachAudio", clipId });
                              setSelectedClipIds([]);
                            },
                          },
                        ]
                      : []),
                  ],
                  // Arranging.
                  [
                    {
                      label: t("contextMenu.moveToPlayhead"),
                      icon: "select",
                      onSelect: () => {
                        const moving = findClip(latest.current.project, clipId);
                        if (moving) {
                          void dispatch({
                            op: "moveClips",
                            moves: [
                              {
                                clipId,
                                start: latest.current.playhead,
                                trackId: moving.trackId,
                              },
                            ],
                          });
                        }
                      },
                    },
                  ],
                  // Destruction, kept in its own section at the bottom.
                  [
                    {
                      label: tp("contextMenu.deleteClips", target.length),
                      icon: "trash",
                      hint: "Del",
                      danger: true,
                      onSelect: () => {
                        void dispatch({ op: "removeClips", clipIds: target });
                        setSelectedClipIds([]);
                      },
                    },
                  ],
                ],
              });
            }}
          />
        </div>
      </div>

      {context && <ContextMenu target={context} onClose={() => setContext(null)} />}
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      {speech && (
        <TtsDialog
          projectPath={session.path}
          initialText={speech.text}
          onGenerated={(path) => insertNarration(path, speech.at)}
          onClose={() => setSpeech(null)}
        />
      )}

      {modifyingProject && (
        <ModifyProjectDialog
          name={projectName}
          frame={frame}
          frameRate={frameRate}
          busy={modifyingProject.busy}
          onSave={({ name, width, height }) => {
            setModifyingProject({ busy: true });
            setFrame({ width, height });
            editorSave({ width, height, name })
              .then(() => {
                setProjectName(name);
                setModifyingProject(null);
                pushToast(t("toast.projectDetailsSaved"), false);
              })
              .catch((cause: unknown) => {
                setModifyingProject({ busy: false });
                pushToast(String(cause), true);
              });
          }}
          onCancel={() => setModifyingProject(null)}
        />
      )}

      {templateDialog && (
        <SaveTemplateDialog
          defaultName={projectName}
          slotCount={project.media.filter((item) => item.placeholder).length}
          busy={templateDialog.busy}
          onSave={(name) => {
            setTemplateDialog({ busy: true });
            templateSave(name)
              .then((info) => {
                setTemplateDialog(null);
                setToast({
                  id: Date.now(),
                  message: t("toast.templateSaved", { name: info.name }),
                  failed: false,
                });
              })
              .catch((cause: unknown) => {
                setTemplateDialog({ busy: false });
                setToast({ id: Date.now(), message: String(cause), failed: true });
              });
          }}
          onCancel={() => setTemplateDialog(null)}
        />
      )}

      {timelineToDelete && (
        <ConfirmDialog
          title={t("dialog.deleteTimeline.title", { name: timelineToDelete.name })}
          message={(() => {
            const count = timelineClipCount(project, timelineToDelete.id);
            return count > 0
              ? tp("dialog.deleteTimeline.clipsLost", count)
              : t("dialog.deleteTimeline.empty");
          })()}
          confirmLabel={t("dialog.deleteTimeline.confirm")}
          onConfirm={() => {
            deleteTimelineNow(timelineToDelete.id);
            setTimelineToDelete(null);
          }}
          onCancel={() => setTimelineToDelete(null)}
        />
      )}

      {toast && (
        <Toast
          key={toast.id}
          toast={toast}
          // Guarded so an old toast's exit can never dismiss its replacement.
          onDone={(id) => setToast((current) => (current?.id === id ? null : current))}
        />
      )}

      {exporting && (
        <ExportDialog
          projectName={projectName}
          projectPath={session.path}
          width={frame.width}
          height={frame.height}
          rateNum={session.rateNum}
          rateDen={session.rateDen}
          duration={duration}
          clipCount={renderableClipCount}
          titles={exportTitles}
          onClose={() => setExporting(false)}
        />
      )}

      {/* The dragged item follows the pointer. */}
      {mediaDrag && (
        <div
          style={{ left: mediaDrag.x + 14, top: mediaDrag.y + 14 }}
          className="surface pointer-events-none fixed z-50 flex items-center gap-2 rounded-lg px-2.5 py-1.5"
        >
          <Icon
            name={mediaDrag.item.kind === "video" ? "film" : "music"}
            size={13}
            className={mediaDrag.item.kind === "video" ? "text-accent" : "text-clip-audio"}
          />
          <span className="max-w-48 truncate text-xs text-primary">{mediaDrag.item.name}</span>
          {selectedMediaIds.length > 1 && selectedMediaIds.includes(mediaDrag.item.id) && (
            <span className="rounded bg-accent px-1.5 font-technical text-[10px] text-on-accent">
              {selectedMediaIds.length}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
