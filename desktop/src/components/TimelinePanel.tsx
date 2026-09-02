// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useEffect, useRef, useState } from "react";
import type {
  PointerEvent as ReactPointerEvent,
  WheelEvent as ReactWheelEvent,
} from "react";

import { subscribeAssets, type MediaAssets, type Peaks } from "../lib/assets";
import { themeColor, type Theme } from "../lib/theme";
import {
  activeTimeline,
  snapTime,
  type Clip,
  type ClipMove,
  type EditorProject,
  type TimelineMeta,
  type Track,
} from "../lib/editor";
import { useLocale, type MsgKey } from "../lib/i18n";
import { timecode } from "../lib/time";
import { Icon, IconButton } from "./Icon";
import { Menu, type MenuOption } from "./Menu";
import { Bar, Divider, PANEL_SHELL, Spacer } from "./Panel";

export type Tool = "select" | "razor";

/** Geometry shared by the renderer and the hit tester, so they cannot disagree. */
const RULER_HEIGHT = 28;
/** Tall enough that a filmstrip frame and a waveform are both readable. */
const TRACK_HEIGHT = 64;
/** Marks the canvas so a drag in flight can find it. See `resolveDrop`. */
const CANVAS_MARKER = "data-wolfcut-timeline";
const HEADER_WIDTH = 164;
/** How close to a clip edge the pointer must be to grab it, in pixels. */
const EDGE_GRAB = 6;
/** How near the canvas edge a drag starts pulling the view along, in pixels. */
const EDGE_MARGIN = 56;
/** Top auto-scroll speed. Roughly 4.5 viewport-widths per second at 900px. */
const EDGE_MAX_PIXELS_PER_FRAME = 24;
/** Where the playhead is parked when playback scrolls the view, 0..1. */
const FOLLOW_ANCHOR = 0.1;

/**
 * The canvas palette.
 *
 * Mutable, and deliberately so. Canvas cannot resolve `var(--color-x)` - it
 * needs a concrete colour string - so these are copied out of the stylesheet
 * whenever the theme changes and read straight from here by the draw loop.
 * The initial values are only what shows for the one frame before the first
 * refresh lands.
 */
const COLORS = {
  background: "#0d0d10",
  ruler: "#131316",
  trackOdd: "#0a0a0d",
  trackEven: "#0d0d10",
  hairline: "rgba(255,255,255,0.1)",
  tickMajor: "rgba(255,255,255,0.2)",
  text: "#6e6e73",
  clipSelected: "#0a84ff",
  clipText: "#ffffff",
  playhead: "#ff453a",
  dropZone: "rgba(10,132,255,0.18)",
};

/** Re-reads the canvas palette from the stylesheet. Cheap; call on theme change. */
function refreshCanvasPalette(): void {
  COLORS.background = themeColor("timeline", COLORS.background);
  COLORS.ruler = themeColor("ruler", COLORS.ruler);
  COLORS.trackEven = themeColor("timeline", COLORS.trackEven);
  COLORS.trackOdd = themeColor("timeline-alt", COLORS.trackOdd);
  COLORS.hairline = themeColor("hairline", COLORS.hairline);
  COLORS.tickMajor = themeColor("hairline-strong", COLORS.tickMajor);
  COLORS.text = themeColor("secondary", COLORS.text);
  COLORS.clipSelected = themeColor("accent", COLORS.clipSelected);
  COLORS.clipText = themeColor("on-accent", COLORS.clipText);
  COLORS.playhead = themeColor("playhead", COLORS.playhead);
  COLORS.dropZone = themeColor("accent-soft", COLORS.dropZone);

  PALETTE.video.header = themeColor("clip-video", PALETTE.video.header);
  PALETTE.video.body = themeColor("clip-video-body", PALETTE.video.body);
  PALETTE.video.edge = PALETTE.video.header;
  PALETTE.audio.header = themeColor("clip-audio", PALETTE.audio.header);
  PALETTE.audio.body = themeColor("clip-audio-body", PALETTE.audio.body);
  PALETTE.audio.edge = PALETTE.audio.header;
  PALETTE.audio.wave = themeColor("clip-wave", PALETTE.audio.wave);
  PALETTE.image.header = themeColor("clip-image", PALETTE.image.header);
  PALETTE.image.body = themeColor("clip-image-body", PALETTE.image.body);
  PALETTE.image.edge = PALETTE.image.header;
  PALETTE.text.header = themeColor("clip-text", PALETTE.text.header);
  PALETTE.text.body = themeColor("clip-text-body", PALETTE.text.body);
  PALETTE.text.edge = PALETTE.text.header;
}

/** Where one clip sat when a move began, so the whole set moves rigidly. */
interface MoveOrigin {
  clipId: string;
  start: number;
  row: number;
}

type DragState =
  | { kind: "scrub" }
  | {
      kind: "marquee";
      /** Canvas-relative, so the band survives the view scrolling under it. */
      originX: number;
      originY: number;
      x: number;
      y: number;
      /** Shift held: add to the existing selection rather than replacing it. */
      additive: boolean;
      basis: readonly string[];
    }
  | {
      kind: "move";
      /** The clip actually grabbed; it is the one that snaps. */
      primary: string;
      /** Seconds between that clip's start and where it was grabbed. */
      grab: number;
      originRow: number;
      origins: MoveOrigin[];
    }
  | { kind: "trimStart" | "trimEnd"; clipId: string };

export function TimelinePanel({
  project,
  playhead,
  playing,
  frameRate,
  tool,
  snap,
  selectedClipIds,
  secondsPerPixel,
  scrollLeft,
  trackScroll,
  assets,
  theme,
  onToolChange,
  onSnapChange,
  onScrub,
  onSelectClips,
  onMoveClips,
  onTrimClip,
  onSplitAtPlayhead,
  onMergeSelected,
  mergeBlockedBecause,
  onDeleteSelected,
  mediaDrag,
  onZoom,
  onScroll,
  onTrackScroll,
  onFit,
  onTrackFlag,
  onAddTrack,
  onRemoveTrack,
  onRenameTrack,
  onClipContextMenu,
  clipTools,
  onSelectTimeline,
  onAddTimeline,
  onRenameTimeline,
  onMoveTimeline,
  onRequestRemoveTimeline,
  onGestureEnd,
}: {
  project: EditorProject;
  playhead: number;
  /** Drives the view following the playhead during playback. */
  playing: boolean;
  frameRate: number;
  tool: Tool;
  snap: boolean;
  selectedClipIds: readonly string[];
  secondsPerPixel: number;
  scrollLeft: number;
  /** Vertical offset into the track stack, in pixels. */
  trackScroll: number;
  /** Waveform and filmstrip cache, read live from inside the draw loop. */
  assets: MediaAssets;
  /** Only used to know when to re-read the canvas palette. */
  theme: Theme;
  onToolChange: (tool: Tool) => void;
  onSnapChange: (snap: boolean) => void;
  onScrub: (seconds: number) => void;
  onSelectClips: (clipIds: string[]) => void;
  onMoveClips: (moves: ClipMove[]) => void;
  onTrimClip: (clipId: string, edge: "start" | "end", delta: number) => void;
  onSplitAtPlayhead: () => void;
  onMergeSelected: () => void;
  /** Null when the selection can be merged; otherwise why it cannot. */
  mergeBlockedBecause: MsgKey | null;
  onDeleteSelected: () => void;
  /** A bin item currently being dragged, in client coordinates. */
  mediaDrag: { x: number; y: number } | null;
  onZoom: (factor: number, anchorSeconds?: number) => void;
  onScroll: (seconds: number) => void;
  onTrackScroll: (pixels: number) => void;
  /** Receives the canvas width, which is the only place that knows it. */
  onFit: (canvasWidth: number) => void;
  onTrackFlag: (trackId: string, flag: "visible" | "muted", value: boolean) => void;
  onAddTrack: () => void;
  onRemoveTrack: (trackId: string) => void;
  onRenameTrack: (trackId: string, name: string) => void;
  onClipContextMenu: (clipId: string, x: number, y: number) => void;
  /**
   * The audio/video tools dropdown, as menu groups. Built by the app, which
   * owns the selection logic; this panel only hangs it in the tray.
   */
  clipTools: MenuOption[][];
  onSelectTimeline: (timelineId: string) => void;
  onAddTimeline: () => void;
  onRenameTimeline: (timelineId: string, name: string) => void;
  /** Drops a dragged tab at a new slot; the index counts the tab as removed. */
  onMoveTimeline: (timelineId: string, index: number) => void;
  /** Asks the app to confirm and delete; the panel never deletes directly. */
  onRequestRemoveTimeline: (timelineId: string) => void;
  /** A move or trim drag finished; the echoed change becomes one command. */
  onGestureEnd: () => void;
}) {
  const { t } = useLocale();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const drag = useRef<DragState | null>(null);
  /** Last pointer position, replayed by the edge-scroll loop. */
  const pointer = useRef<{ x: number; y: number } | null>(null);
  /** Auto-scroll speed in pixels per frame; zero when not near an edge. */
  const edgeSpeed = useRef(0);
  /** Latest drag applier and scroll callback, for the frame loop to reach. */
  const applyDragRef = useRef<(clientX: number, clientY: number) => void>(() => {});
  const scrollRef = useRef(onScroll);
  scrollRef.current = onScroll;
  /** The header column, kept in step with the canvas's vertical offset. */
  const headerScroll = useRef<HTMLDivElement>(null);
  // The lane and clip set being drawn: the active timeline's.
  const timeline = activeTimeline(project);
  // Top-most track first on screen; the model stores them bottom-most first to
  // match the engine's compositing order.
  const rows: Track[] = [...timeline.tracks].reverse();

  // Which lane a dragged bin item would land on. Tracks are untyped, so any
  // lane under the pointer is a valid target.
  const dropTrack = (() => {
    if (!mediaDrag) return null;
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const bounds = canvas.getBoundingClientRect();
    if (mediaDrag.x < bounds.left || mediaDrag.x > bounds.right) return null;
    const offsetY = mediaDrag.y - bounds.top - RULER_HEIGHT;
    if (offsetY < 0) return null;
    return rows[Math.floor((offsetY + trackScroll) / TRACK_HEIGHT)]?.id ?? null;
  })();

  // Set membership is checked once per visible clip per frame; an array scan
  // would be O(clips x selection).
  const selected = new Set(selectedClipIds);

  // The draw loop reads everything through this ref, so a prop change never
  // tears down and rebuilds the loop.
  const view = useRef({ project, timeline, playhead, playing, secondsPerPixel, scrollLeft, trackScroll, frameRate, selected, rows, dropTrack, assets, tool });
  view.current = { project, timeline, playhead, playing, secondsPerPixel, scrollLeft, trackScroll, frameRate, selected, rows, dropTrack, assets, tool };

  // Repaint only when something could have changed. Every render marks the
  // canvas dirty (props are how state reaches it), pointer moves mark it
  // (the cursor and razor guide draw without a render), artwork arrival
  // marks it (thumbnails land outside React on purpose), and a live drag
  // draws every frame regardless - edge auto-scroll happens inside draw.
  // An idle editor previously repainted at full frame rate forever, which
  // on a laptop is a battery tax for drawing the same pixels.
  const dirty = useRef(true);
  dirty.current = true;

  const timeAt = useCallback(
    (clientX: number) => {
      const canvas = canvasRef.current;
      if (!canvas) return 0;
      const bounds = canvas.getBoundingClientRect();
      return Math.max(0, (clientX - bounds.left) * view.current.secondsPerPixel + view.current.scrollLeft);
    },
    [],
  );

  const rowAt = useCallback((clientY: number) => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const bounds = canvas.getBoundingClientRect();
    const y = clientY - bounds.top - RULER_HEIGHT;
    if (y < 0) return null;
    const index = Math.floor((y + view.current.trackScroll) / TRACK_HEIGHT);
    return view.current.rows[index] ?? null;
  }, []);

  const clipAt = useCallback(
    (clientX: number, clientY: number): { clip: Clip; edge: "start" | "end" | null } | null => {
      const track = rowAt(clientY);
      if (!track) return null;
      const time = timeAt(clientX);
      const { timeline, secondsPerPixel } = view.current;

      // Later clips draw on top, so search back to front.
      for (let index = timeline.clips.length - 1; index >= 0; index -= 1) {
        const clip = timeline.clips[index];
        if (clip.trackId !== track.id) continue;
        if (time < clip.start || time > clip.start + clip.duration) continue;

        const grabSeconds = EDGE_GRAB * secondsPerPixel;
        const edge =
          time - clip.start < grabSeconds
            ? "start"
            : clip.start + clip.duration - time < grabSeconds
              ? "end"
              : null;
        return { clip, edge };
      }
      return null;
    },
    [rowAt, timeAt],
  );

  // ── drawing ──────────────────────────────────────────────────────────────
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d", { alpha: false });
    if (!canvas || !context) return;

    const state = view.current;
    const ratio = window.devicePixelRatio || 1;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
      canvas.width = Math.max(1, Math.round(width * ratio));
      canvas.height = Math.max(1, Math.round(height * ratio));
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0);

    // Keeping the playhead reachable, once per frame.
    //
    // Two cases, and dragging wins: while a drag is pulling at an edge the
    // view travels with it and the drag is replayed at the new scroll offset,
    // so the playhead stays glued to the pointer. Otherwise, during playback,
    // the view pages along whenever the playhead leaves the visible span.
    const span = width * state.secondsPerPixel;
    if (drag.current && edgeSpeed.current !== 0 && pointer.current) {
      scrollRef.current(Math.max(0, state.scrollLeft + edgeSpeed.current * state.secondsPerPixel));
      applyDragRef.current(pointer.current.x, pointer.current.y);
    } else if (
      state.playing &&
      (state.playhead < state.scrollLeft || state.playhead > state.scrollLeft + span)
    ) {
      scrollRef.current(Math.max(0, state.playhead - span * FOLLOW_ANCHOR));
    }

    const toX = (seconds: number) => (seconds - state.scrollLeft) / state.secondsPerPixel;

    context.fillStyle = COLORS.background;
    context.fillRect(0, 0, width, height);

    // Track bands.
    state.rows.forEach((track, index) => {
      const y = RULER_HEIGHT + index * TRACK_HEIGHT - state.trackScroll;
      context.fillStyle = index % 2 === 0 ? COLORS.trackEven : COLORS.trackOdd;
      context.fillRect(0, y, width, TRACK_HEIGHT);

      if (state.dropTrack === track.id) {
        context.fillStyle = COLORS.dropZone;
        context.fillRect(0, y, width, TRACK_HEIGHT);
      }

      context.strokeStyle = COLORS.hairline;
      context.beginPath();
      context.moveTo(0, y + TRACK_HEIGHT - 0.5);
      context.lineTo(width, y + TRACK_HEIGHT - 0.5);
      context.stroke();
    });

    drawRuler(context, width, state.scrollLeft, state.secondsPerPixel, state.frameRate);

    // Grid lines dropping out of the ruler.
    const interval = tickInterval(state.secondsPerPixel);
    context.strokeStyle = COLORS.hairline;
    context.beginPath();
    for (
      let seconds = Math.ceil(state.scrollLeft / interval) * interval;
      toX(seconds) < width;
      seconds += interval
    ) {
      const x = Math.round(toX(seconds)) + 0.5;
      context.moveTo(x, RULER_HEIGHT);
      context.lineTo(x, height);
    }
    context.stroke();

    // Clips and the selection band are confined to the lane area: scrolled far
    // enough, a clip would otherwise paint straight over the ruler.
    context.save();
    context.beginPath();
    context.rect(0, RULER_HEIGHT, width, Math.max(0, height - RULER_HEIGHT));
    context.clip();

    // Row index per track id, once per repaint - the loop below runs per
    // clip per frame during playback, and a caption run puts hundreds of
    // clips on the timeline.
    const rowByTrack = new Map(state.rows.map((track, index) => [track.id, index]));
    for (const clip of state.timeline.clips) {
      const rowIndex = rowByTrack.get(clip.trackId) ?? -1;
      if (rowIndex < 0) continue;

      const x = toX(clip.start);
      const clipWidth = clip.duration / state.secondsPerPixel;
      if (x + clipWidth < 0 || x > width) continue;

      drawClip(
        context,
        clip,
        x,
        RULER_HEIGHT + rowIndex * TRACK_HEIGHT - state.trackScroll,
        clipWidth,
        state.selected.has(clip.id),
        state.assets,
        state.secondsPerPixel,
      );
    }

    // Razor guide: a dashed line at the pointer, but only while it is over a
    // clip. Showing it in empty space would promise a cut that does nothing.
    if (state.tool === "razor" && pointer.current && !drag.current) {
      const canvas = canvasRef.current;
      const bounds = canvas?.getBoundingClientRect();
      if (bounds) {
        const localX = pointer.current.x - bounds.left;
        const localY = pointer.current.y - bounds.top;
        const time = localX * state.secondsPerPixel + state.scrollLeft;
        const rowIndex = Math.floor((localY - RULER_HEIGHT + state.trackScroll) / TRACK_HEIGHT);
        const track = state.rows[rowIndex];

        const overClip =
          localY > RULER_HEIGHT &&
          track !== undefined &&
          state.timeline.clips.some(
            (clip) =>
              clip.trackId === track.id && time > clip.start && time < clip.start + clip.duration,
          );

        if (overClip) {
          const guide = Math.round(localX) + 0.5;
          context.save();
          context.strokeStyle = COLORS.playhead;
          context.lineWidth = 1;
          context.setLineDash([4, 3]);
          context.beginPath();
          context.moveTo(guide, RULER_HEIGHT);
          context.lineTo(guide, height);
          context.stroke();
          context.restore();
        }
      }
    }

    // The selection band, drawn over the clips it is catching. Palette
    // colours like every other mark on this canvas - these were the only
    // two that bypassed the theme.
    if (drag.current?.kind === "marquee") {
      const band = normalise(drag.current);
      context.fillStyle = COLORS.dropZone;
      context.fillRect(band.x, band.y, band.width, band.height);
      context.strokeStyle = COLORS.clipSelected;
      context.lineWidth = 1;
      context.strokeRect(
        Math.round(band.x) + 0.5,
        Math.round(band.y) + 0.5,
        Math.round(band.width),
        Math.round(band.height),
      );
    }

    context.restore();

    drawPlayhead(context, height, toX(state.playhead));
  }, []);

  // Scrolling the canvas moves the headers. Guarded, because assigning
  // scrollTop fires onScroll again and the two would chase each other.
  useEffect(() => {
    const element = headerScroll.current;
    if (element && Math.abs(element.scrollTop - trackScroll) > 0.5) {
      element.scrollTop = trackScroll;
    }
  }, [trackScroll]);

  // The stylesheet has already been applied by the time this runs, so the
  // computed values it reads are the new theme's.
  useEffect(() => {
    refreshCanvasPalette();
  }, [theme]);

  useEffect(() => {
    let frame = 0;
    const tick = () => {
      if (dirty.current || drag.current !== null || edgeSpeed.current !== 0) {
        dirty.current = false;
        draw();
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);

    const canvas = canvasRef.current;
    // Resize draws immediately: resizing clears the canvas, and waiting for
    // the next tick would flash a blank frame.
    const observer = new ResizeObserver(() => draw());
    if (canvas) observer.observe(canvas);

    const unsubscribe = subscribeAssets(view.current.assets, () => {
      dirty.current = true;
    });

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      unsubscribe();
    };
  }, [draw]);

  // ── pointer interaction ──────────────────────────────────────────────────
  const onPointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.button === 2) return; // handled by onContextMenu

    const canvas = event.currentTarget;
    const bounds = canvas.getBoundingClientRect();
    const overRuler = event.clientY - bounds.top < RULER_HEIGHT;
    const hit = overRuler ? null : clipAt(event.clientX, event.clientY);

    canvas.setPointerCapture(event.pointerId);

    // The ruler is the scrub strip. Dragging in the track area draws a
    // selection band instead, which is the convention every NLE shares and the
    // only way marquee select and scrubbing can coexist on one surface.
    if (overRuler) {
      drag.current = { kind: "scrub" };
      onScrub(timeAt(event.clientX));
      return;
    }

    if (hit && tool === "razor") {
      // The razor cuts where you click, not where the playhead is.
      onSelectClips([hit.clip.id]);
      onScrub(timeAt(event.clientX));
      window.setTimeout(onSplitAtPlayhead, 0);
      return;
    }

    if (hit) {
      const alreadySelected = selectedClipIds.includes(hit.clip.id);

      // Shift toggles. Otherwise grabbing a clip that is already part of the
      // selection keeps the whole set - so a multi-clip drag does not collapse
      // to one clip the instant you touch it.
      const next = event.shiftKey
        ? alreadySelected
          ? selectedClipIds.filter((id) => id !== hit.clip.id)
          : [...selectedClipIds, hit.clip.id]
        : alreadySelected
          ? [...selectedClipIds]
          : [hit.clip.id];

      onSelectClips(next);

      if (hit.edge && next.length <= 1) {
        drag.current = {
          kind: hit.edge === "start" ? "trimStart" : "trimEnd",
          clipId: hit.clip.id,
        };
        return;
      }

      const moving = next.includes(hit.clip.id) ? next : [hit.clip.id];
      const originRow = rows.findIndex((track) => track.id === hit.clip.trackId);

      drag.current = {
        kind: "move",
        primary: hit.clip.id,
        grab: timeAt(event.clientX) - hit.clip.start,
        originRow,
        origins: moving.flatMap((clipId) => {
          const clip = timeline.clips.find((candidate) => candidate.id === clipId);
          if (!clip) return [];
          return [
            {
              clipId,
              start: clip.start,
              row: rows.findIndex((track) => track.id === clip.trackId),
            },
          ];
        }),
      };
      return;
    }

    // Empty track area: start a band.
    drag.current = {
      kind: "marquee",
      originX: event.clientX - bounds.left,
      originY: event.clientY - bounds.top,
      x: event.clientX - bounds.left,
      y: event.clientY - bounds.top,
      additive: event.shiftKey,
      basis: event.shiftKey ? [...selectedClipIds] : [],
    };
    if (!event.shiftKey) onSelectClips([]);
  };

  /**
   * Applies whatever drag is in flight at these client coordinates.
   *
   * Split out from the pointer handler because the edge-scroll loop replays it
   * every frame from the last known pointer position - the pointer has stopped
   * moving at the window edge, but the timeline underneath it has not.
   */
  const applyDrag = (clientX: number, clientY: number) => {
    const state = drag.current;
    if (!state) return;

    const time = timeAt(clientX);

    if (state.kind === "scrub") {
      onScrub(time);
      return;
    }

    if (state.kind === "marquee") {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const bounds = canvas.getBoundingClientRect();
      state.x = clientX - bounds.left;
      state.y = clientY - bounds.top;
      return;
    }

    if (state.kind === "move") {
      const primary = timeline.clips.find((candidate) => candidate.id === state.primary);
      const anchor = state.origins.find((origin) => origin.clipId === state.primary);
      if (!primary || !anchor) return;

      // Only the grabbed clip snaps; everything else keeps its offset from it,
      // so the shape of a multi-clip selection is preserved exactly.
      const raw = time - state.grab;
      const snapped = snap
        ? snapTime(project, raw, {
            threshold: 8 * secondsPerPixel,
            playhead,
            exclude: primary.id,
          })
        : raw;

      const deltaTime = snapped - anchor.start;
      const currentRow = rowAt(clientY)
        ? rows.findIndex((track) => track.id === rowAt(clientY)?.id)
        : anchor.row;
      const deltaRow = currentRow - state.originRow;

      onMoveClips(
        state.origins.flatMap((origin) => {
          const row = Math.min(rows.length - 1, Math.max(0, origin.row + deltaRow));
          const track = rows[row];
          if (!track) return [];
          return [
            { clipId: origin.clipId, start: Math.max(0, origin.start + deltaTime), trackId: track.id },
          ];
        }),
      );
      return;
    }

    const clip = timeline.clips.find((candidate) => candidate.id === state.clipId);
    if (!clip) return;
    if (state.kind === "trimStart") onTrimClip(clip.id, "start", time - clip.start);
    if (state.kind === "trimEnd") onTrimClip(clip.id, "end", time - (clip.start + clip.duration));
  };
  applyDragRef.current = applyDrag;

  const onPointerMove = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const canvas = event.currentTarget;
    pointer.current = { x: event.clientX, y: event.clientY };
    // The razor guide follows the pointer without a React render.
    dirty.current = true;

    if (!drag.current) {
      edgeSpeed.current = 0;
      // Cursor feedback: the edges are grabbable, the body is draggable.
      const hit = clipAt(event.clientX, event.clientY);
      canvas.style.cursor =
        tool === "razor" && hit
          ? "crosshair"
          : hit?.edge
            ? "ew-resize"
            : hit
              ? "grab"
              : "text";
      return;
    }

    // Drag the playhead (or a clip) toward either edge and the view starts
    // travelling with it, so it cannot be parked somewhere off-screen and
    // lost. Speed ramps with how far past the margin the pointer is.
    const bounds = canvas.getBoundingClientRect();
    const past =
      event.clientX < bounds.left + EDGE_MARGIN
        ? event.clientX - (bounds.left + EDGE_MARGIN)
        : event.clientX > bounds.right - EDGE_MARGIN
          ? event.clientX - (bounds.right - EDGE_MARGIN)
          : 0;

    edgeSpeed.current =
      Math.sign(past) * Math.min(Math.abs(past) / EDGE_MARGIN, 1) * EDGE_MAX_PIXELS_PER_FRAME;

    applyDrag(event.clientX, event.clientY);
  };

  const endDrag = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }

    const state = drag.current;
    // A move or trim was echoing locally; releasing is what makes it real.
    if (state?.kind === "move" || state?.kind === "trimStart" || state?.kind === "trimEnd") {
      onGestureEnd();
    }
    if (state?.kind === "marquee") {
      // Commit on release rather than live: selecting as the band sweeps looks
      // busy and makes a mistaken sweep hard to back out of.
      const band = normalise(state);
      const caught = timeline.clips.filter((clip) => {
        const rect = clipRect(clip, rows, secondsPerPixel, scrollLeft, trackScroll);
        return rect !== null && intersects(band, rect);
      });

      const ids = caught.map((clip) => clip.id);
      onSelectClips(state.additive ? [...new Set([...state.basis, ...ids])] : ids);
    }

    drag.current = null;
    edgeSpeed.current = 0;
    // An empty marquee that changed no selection triggers no render; the
    // band still has to leave the screen.
    dirty.current = true;
  };

  const onWheel = (event: ReactWheelEvent<HTMLCanvasElement>) => {
    if (event.ctrlKey || event.metaKey) {
      onZoom(event.deltaY > 0 ? 1.15 : 1 / 1.15, timeAt(event.clientX));
      return;
    }

    // A trackpad's horizontal axis always pans time. Shift does the same for a
    // wheel, which has no horizontal axis of its own. Everything else scrolls
    // the track stack, which is what a plain wheel means in every other list.
    if (event.deltaX !== 0 || event.shiftKey) {
      const delta = (event.deltaX !== 0 ? event.deltaX : event.deltaY) * secondsPerPixel;
      onScroll(Math.max(0, scrollLeft + delta));
      return;
    }

    onTrackScroll(clampTrackScroll(trackScroll + event.deltaY, rows.length, canvasRef.current));
  };

  // ── timeline tab reorder ──────────────────────────────────────────────────
  // A pointer drag on a tab, not HTML5 drag-and-drop: the webview's OS file
  // drop handling owns that channel (see the bin drag in App for the same
  // choice). Browser-tab behaviour: past a small threshold the pressed tab
  // rides the pointer while its neighbours slide aside in real time, and
  // letting go drops it into the gap it is hovering. A plain click still
  // selects.
  const tabStrip = useRef<HTMLDivElement>(null);
  /** A drag in flight: which tab and how far the pointer has pulled it. */
  const [tabDrag, setTabDrag] = useState<{ id: string; dx: number } | null>(null);
  const tabPress = useRef<{ id: string; x: number; dragging: boolean } | null>(null);
  /**
   * The strip's geometry frozen at lift-off: every tab's box in the order
   * they sat, plus the origin for converting into strip content pixels.
   * Live rects would feed back - the neighbours move as they slide aside -
   * so every slot decision measures against where the tabs *were*.
   */
  const tabRest = useRef<{
    from: number;
    origin: number;
    lefts: number[];
    widths: number[];
  } | null>(null);
  /** Swallows exactly one click: the one the browser fires after a drop. */
  const tabDropped = useRef(false);
  /** The flex gap between tabs (gap-0.5), part of the hole a tab leaves. */
  const TAB_GAP = 2;

  const tabCentre = (rest: { lefts: number[]; widths: number[] }, i: number) =>
    rest.lefts[i] + rest.widths[i] / 2;

  /** Where the lifted tab would land, counted with it removed from the row:
   * how many resting neighbours its centre has passed to the left of. */
  const tabSlotFor = (
    rest: NonNullable<typeof tabRest.current>,
    dx: number,
  ): number => {
    const centre = tabCentre(rest, rest.from) + dx;
    return rest.lefts.filter((_, i) => i !== rest.from && tabCentre(rest, i) < centre)
      .length;
  };

  const onTabPointerDown = (timelineId: string, event: React.PointerEvent) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    tabPress.current = { id: timelineId, x: event.clientX, dragging: false };
  };
  const onTabPointerMove = (event: React.PointerEvent) => {
    const press = tabPress.current;
    if (!press) return;
    if (!press.dragging) {
      if (Math.abs(event.clientX - press.x) < 4) return;
      const strip = tabStrip.current;
      const tabs = strip
        ? Array.from(strip.querySelectorAll<HTMLElement>('[role="tab"]'))
        : [];
      const from = project.timelines.findIndex((timeline) => timeline.id === press.id);
      if (!strip || from === -1 || tabs.length !== project.timelines.length) return;
      const rects = tabs.map((tab) => tab.getBoundingClientRect());
      tabRest.current = {
        from,
        origin: strip.getBoundingClientRect().left - strip.scrollLeft,
        lefts: rects.map((rect) => rect.left),
        widths: rects.map((rect) => rect.width),
      };
      press.dragging = true;
      // Rebase to lift-off so the tab does not jump the threshold distance.
      press.x = event.clientX;
    }
    setTabDrag({ id: press.id, dx: event.clientX - press.x });
  };
  const onTabPointerUp = (event: React.PointerEvent) => {
    const press = tabPress.current;
    tabPress.current = null;
    if (!press?.dragging) return;
    setTabDrag(null);
    tabDropped.current = true;
    const rest = tabRest.current;
    tabRest.current = null;
    if (!rest) return;
    // The slot is read from the saved geometry, never from the ref the
    // line above just cleared - clearing first is how every drop once
    // landed at index 0 no matter where the preview showed it.
    const index = tabSlotFor(rest, event.clientX - press.x);
    if (index !== rest.from) onMoveTimeline(press.id, index);
  };
  const selectUnlessDropped = (timelineId: string) => {
    // Reordering must not also switch documents: the engine command leaves
    // the selection alone, so the click that follows a drop is swallowed.
    if (tabDropped.current) {
      tabDropped.current = false;
      return;
    }
    onSelectTimeline(timelineId);
  };

  return (
    <div className={PANEL_SHELL}>
      {/*
        The timeline tabs. One per timeline in the project, like sequences in
        any other editor: the active one is what the canvas below, the preview
        and the exporter all show. Kept above the tool bar so switching reads
        as changing documents, not changing tools.
      */}
      <div
        ref={tabStrip}
        className="thin-scroll relative flex h-8 shrink-0 items-end gap-0.5 overflow-x-auto
                   overflow-y-hidden border-b border-hairline bg-sunken px-1.5 pt-1"
      >
        {project.timelines.map((timeline, index) => {
          // While a drag is in flight the row is drawn as it will land: the
          // lifted tab rides the pointer, and each neighbour the pointer has
          // carried it past slides one tab-width towards the hole it left.
          let slide: "lifted" | "aside" | null = null;
          let shift = 0;
          const rest = tabDrag ? tabRest.current : null;
          if (tabDrag && rest) {
            const centre = tabCentre(rest, rest.from) + tabDrag.dx;
            const width = rest.widths[rest.from];
            if (timeline.id === tabDrag.id) {
              slide = "lifted";
              shift = tabDrag.dx;
            } else {
              slide = "aside";
              if (index > rest.from && centre > tabCentre(rest, index)) {
                shift = -(width + TAB_GAP);
              } else if (index < rest.from && centre < tabCentre(rest, index)) {
                shift = width + TAB_GAP;
              }
            }
          }
          return (
            <TimelineTab
              key={timeline.id}
              timeline={timeline}
              active={timeline.id === project.activeTimelineId}
              closable={project.timelines.length > 1}
              slide={slide}
              shift={shift}
              onSelect={selectUnlessDropped}
              onRename={onRenameTimeline}
              onRequestRemove={onRequestRemoveTimeline}
              onDragPointerDown={onTabPointerDown}
              onDragPointerMove={onTabPointerMove}
              onDragPointerUp={onTabPointerUp}
            />
          );
        })}
        {(() => {
          // The drop marker: the same truth the drop uses, drawn as a line at
          // the left edge of the hole the tab will land in. The live preview
          // shows the shape of the row; the line pins down the exact slot.
          const rest = tabDrag ? tabRest.current : null;
          if (!tabDrag || !rest) return null;
          const slot = tabSlotFor(rest, tabDrag.dx);
          let edge = rest.lefts[0];
          let counted = 0;
          for (let i = 0; i < rest.widths.length && counted < slot; i += 1) {
            if (i === rest.from) continue;
            edge += rest.widths[i] + TAB_GAP;
            counted += 1;
          }
          return (
            <span
              aria-hidden
              className="pointer-events-none absolute bottom-0.5 top-1.5 z-20 w-0.5
                         rounded bg-accent"
              style={{ left: edge - rest.origin - 2 }}
            />
          );
        })()}
        <span className="self-center">
          <IconButton icon="plus" label={t("timeline.newTimeline")} size={7} onClick={onAddTimeline} />
        </span>
      </div>

      <Bar>
        <IconButton
          icon="select"
          label={t("timeline.selectTool")}
          active={tool === "select"}
          onClick={() => onToolChange("select")}
        />
        <IconButton
          icon="razor"
          label={t("timeline.razorTool")}
          active={tool === "razor"}
          onClick={() => onToolChange("razor")}
        />
        <Divider />
        <IconButton icon="split" label={t("timeline.split")} onClick={onSplitAtPlayhead} />
        <IconButton
          icon="merge"
          // The reason lives in the tooltip: a button that greys out without
          // saying why leaves you guessing at the rule.
          label={mergeBlockedBecause ? t(mergeBlockedBecause) : t("timeline.merge")}
          disabled={mergeBlockedBecause !== null}
          onClick={onMergeSelected}
        />
        <IconButton
          icon="trash"
          label={
            selectedClipIds.length > 1
              ? t("timeline.deleteClips", { count: selectedClipIds.length })
              : t("timeline.deleteSelected")
          }
          tone="danger"
          disabled={selectedClipIds.length === 0}
          onClick={onDeleteSelected}
        />
        {selectedClipIds.length > 1 && (
          <span className="px-1 font-technical text-[10px] text-accent">
            {selectedClipIds.length}
          </span>
        )}
        <Divider />
        <IconButton
          icon="magnet"
          label={t("timeline.snapping")}
          active={snap}
          onClick={() => onSnapChange(!snap)}
        />
        {/* Only while the selected clip has tools to offer - sound to detach
            or transcribe, or a title's text to speak. A menu of grey rows
            teaches nothing. */}
        {clipTools.length > 0 && (
          <>
            <Divider />
            <Menu
              groups={clipTools}
              trigger={(open) => (
                <span
                  title={t("timeline.avTools")}
                  className={`flex h-9 items-center gap-0.5 rounded-lg px-1.5 transition-colors
                              duration-150 ${
                                open
                                  ? "bg-tool-active-bg text-tool-active"
                                  : "text-primary hover:bg-hover"
                              }`}
                >
                  <Icon name="waveform" size={16} />
                  <Icon name="chevronDown" size={11} />
                </span>
              )}
            />
          </>
        )}
        <Divider />
        <IconButton icon="plus" label={t("timeline.addTrack")} onClick={onAddTrack} />

        <Spacer />

        {/* Fixed width for the same reason: without it the zoom controls to
            its right shuffle every time the frame digits change. */}
        <span className="w-30 px-2 text-right font-mono text-[10px] tabular-nums text-tertiary">
          {timecode(playhead, frameRate)}
        </span>
        <Divider />
        <IconButton
          icon="fit"
          label={t("timeline.fit")}
          onClick={() => onFit(canvasRef.current?.clientWidth ?? 0)}
        />
        <IconButton icon="minus" label={t("timeline.zoomOut")} size={7} onClick={() => onZoom(1.4)} />
        <IconButton
          icon="plus"
          label={t("timeline.zoomIn")}
          size={7}
          onClick={() => onZoom(1 / 1.4)}
        />
      </Bar>

      <div className="flex min-h-0 flex-1">
        <div
          className="flex shrink-0 flex-col border-r border-hairline"
          style={{ width: HEADER_WIDTH }}
        >
          {/* Sits opposite the ruler and does not scroll with the lanes. */}
          <div className="shrink-0 border-b border-hairline" style={{ height: RULER_HEIGHT }} />

          {/*
            The headers are a real scroll container, so the native scrollbar and
            trackpad both drive the track stack. Its offset is pushed up to the
            canvas rather than the two keeping separate positions - one source
            of truth is the only way the lanes and their labels stay aligned.
          */}
          <div
            ref={headerScroll}
            onScroll={(event) => onTrackScroll(event.currentTarget.scrollTop)}
            className="thin-scroll min-h-0 flex-1 overflow-y-auto overflow-x-hidden"
          >
          {rows.map((track) => (
            <TrackHeader
              key={track.id}
              track={track}
              removable={rows.length > 1}
              onFlag={onTrackFlag}
              onRemove={onRemoveTrack}
              onRename={onRenameTrack}
            />
          ))}
          </div>
        </div>

        <canvas
          ref={canvasRef}
          {...{ [CANVAS_MARKER]: true }}
          className="min-w-0 flex-1"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onWheel={onWheel}
          onContextMenu={(event) => {
            event.preventDefault();
            const hit = clipAt(event.clientX, event.clientY);
            if (hit) onClipContextMenu(hit.clip.id, event.clientX, event.clientY);
          }}
        />
      </div>
    </div>
  );
}

/**
 * One timeline tab.
 *
 * Click switches, double-click renames in place (the same commit-on-blur rule
 * as a track rename), and the close button asks the app to confirm - deleting
 * a timeline throws away every clip on it, so the panel never does it
 * directly.
 */
function TimelineTab({
  timeline,
  active,
  closable,
  slide,
  shift,
  onSelect,
  onRename,
  onRequestRemove,
  onDragPointerDown,
  onDragPointerMove,
  onDragPointerUp,
}: {
  timeline: TimelineMeta;
  active: boolean;
  closable: boolean;
  /**
   * This tab's part in a reorder drag: "lifted" rides the pointer raw -
   * easing there would make it lag the hand - while "aside" neighbours
   * ease between their slots. Null when nothing is being dragged.
   */
  slide: "lifted" | "aside" | null;
  /** Horizontal offset in pixels for the drag being drawn. */
  shift: number;
  onSelect: (timelineId: string) => void;
  onRename: (timelineId: string, name: string) => void;
  onRequestRemove: (timelineId: string) => void;
  onDragPointerDown: (timelineId: string, event: React.PointerEvent) => void;
  onDragPointerMove: (event: React.PointerEvent) => void;
  onDragPointerUp: (event: React.PointerEvent) => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const { t } = useLocale();

  if (renaming) {
    return (
      <input
        autoFocus
        defaultValue={timeline.name}
        spellCheck={false}
        onFocus={(event) => event.currentTarget.select()}
        onBlur={(event) => {
          onRename(timeline.id, event.target.value);
          setRenaming(false);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") {
            event.currentTarget.value = timeline.name;
            event.currentTarget.blur();
          }
        }}
        className="w-28 shrink-0 rounded bg-sunken px-1.5 py-0.5 text-xs text-primary
                   outline-none ring-1 ring-accent"
      />
    );
  }

  return (
    <span
      role="tab"
      aria-selected={active}
      title={active ? timeline.name : t("timeline.switchTo", { name: timeline.name })}
      onClick={() => onSelect(timeline.id)}
      onDoubleClick={() => setRenaming(true)}
      onPointerDown={(event) => onDragPointerDown(timeline.id, event)}
      onPointerMove={onDragPointerMove}
      onPointerUp={onDragPointerUp}
      style={shift !== 0 || slide === "lifted" ? { transform: `translateX(${shift}px)` } : undefined}
      // Browser-tab shape: rounded top only, and the active tab overlaps the
      // strip's bottom border by a pixel with the panel's own background, so
      // it reads as one surface with the tray below rather than a pill
      // floating above it.
      className={`group -mb-px flex max-w-44 shrink-0 cursor-pointer items-center gap-1
                  rounded-t-md px-2.5 py-1 text-xs ${
                    active
                      ? "border border-b-0 border-hairline bg-panel text-primary"
                      : "text-secondary hover:bg-hover hover:text-primary"
                  } ${
                    slide === "lifted"
                      ? "relative z-10 bg-panel shadow-[0_2px_10px_rgba(0,0,0,0.3)]"
                      : slide === "aside"
                        ? "transition-transform duration-150 ease-out"
                        : "transition-colors"
                  }`}
    >
      <span className="truncate">{timeline.name}</span>
      {closable && (
        <button
          type="button"
          aria-label={t("timeline.deleteNamed", { name: timeline.name })}
          title={t("timeline.deleteNamed", { name: timeline.name })}
          onClick={(event) => {
            // The x must not also switch tabs: deleting an inactive timeline
            // should not first drag the editor onto it.
            event.stopPropagation();
            onRequestRemove(timeline.id);
          }}
          className="invisible shrink-0 cursor-pointer rounded p-0.5 text-tertiary
                     transition-colors hover:bg-danger-soft hover:text-danger group-hover:visible"
        >
          <Icon name="close" size={10} />
        </button>
      )}
    </span>
  );
}

/**
 * One lane's controls.
 *
 * Both a visibility and a mute toggle on every track, because a track is not
 * typed - the same lane can be carrying a video clip and an mp3 side by side,
 * and each needs its own switch.
 */
function TrackHeader({
  track,
  removable,
  onFlag,
  onRemove,
  onRename,
}: {
  track: Track;
  removable: boolean;
  onFlag: (trackId: string, flag: "visible" | "muted", value: boolean) => void;
  onRemove: (trackId: string) => void;
  onRename: (trackId: string, name: string) => void;
}) {
  const silent = !track.visible && track.muted;
  const [renaming, setRenaming] = useState(false);
  const { t } = useLocale();

  return (
    <div
      className="group flex items-center gap-0.5 border-b border-hairline px-2"
      style={{ height: TRACK_HEIGHT }}
    >
      {renaming ? (
        <input
          autoFocus
          defaultValue={track.name}
          spellCheck={false}
          onFocus={(event) => event.currentTarget.select()}
          // Committing on blur means clicking away saves rather than discards,
          // which is what every rename-in-place in this app should do.
          onBlur={(event) => {
            onRename(track.id, event.target.value);
            setRenaming(false);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") {
              event.currentTarget.value = track.name;
              event.currentTarget.blur();
            }
          }}
          className="min-w-0 flex-1 rounded bg-sunken px-1 py-0.5 text-xs text-primary
                     outline-none ring-1 ring-accent"
        />
      ) : (
        <span
          onDoubleClick={() => setRenaming(true)}
          title={t("timeline.renameHint")}
          className={`min-w-0 flex-1 cursor-default truncate text-xs ${
            silent ? "text-tertiary" : "text-primary"
          }`}
        >
          {track.name}
        </span>
      )}

      {removable && !renaming && (
        <button
          type="button"
          aria-label={t("timeline.removeTrack", { name: track.name })}
          title={t("timeline.removeTrackHint", { name: track.name })}
          onClick={() => onRemove(track.id)}
          className="invisible shrink-0 cursor-pointer rounded p-1 text-tertiary transition-colors
                     hover:bg-danger-soft hover:text-danger group-hover:visible"
        >
          <Icon name="close" size={11} />
        </button>
      )}
      <IconButton
        icon={track.visible ? "eye" : "eyeOff"}
        label={track.visible ? t("timeline.hideTrack") : t("timeline.showTrack")}
        size={7}
        active={!track.visible}
        onClick={() => onFlag(track.id, "visible", !track.visible)}
      />
      <IconButton
        icon={track.muted ? "volumeOff" : "volume"}
        label={track.muted ? t("timeline.unmuteTrack") : t("timeline.muteTrack")}
        size={7}
        active={track.muted}
        onClick={() => onFlag(track.id, "muted", !track.muted)}
      />
    </div>
  );
}

/**
 * Where a bin item dropped at these client coordinates would land.
 *
 * Lives here because this module owns the timeline's geometry, and returns
 * null when the point is outside the canvas or above the first track - so the
 * caller can simply do nothing.
 */
export function resolveDrop(
  clientX: number,
  clientY: number,
  {
    tracks,
    secondsPerPixel,
    scrollLeft,
    trackScroll,
  }: { tracks: Track[]; secondsPerPixel: number; scrollLeft: number; trackScroll: number },
): { trackId: string; start: number } | null {
  const canvas = document.querySelector<HTMLCanvasElement>(`[${CANVAS_MARKER}]`);
  if (!canvas) return null;

  const bounds = canvas.getBoundingClientRect();
  if (clientX < bounds.left || clientX > bounds.right) return null;
  if (clientY < bounds.top || clientY > bounds.bottom) return null;

  const offsetY = clientY - bounds.top - RULER_HEIGHT;
  if (offsetY < 0) return null;

  const rows = [...tracks].reverse();
  const track = rows[Math.floor((offsetY + trackScroll) / TRACK_HEIGHT)];
  if (!track) return null;

  return {
    trackId: track.id,
    start: Math.max(0, (clientX - bounds.left) * secondsPerPixel + scrollLeft),
  };
}

/** Keeps the track stack from scrolling past its own contents. */
export function clampTrackScroll(
  value: number,
  trackCount: number,
  canvas: HTMLCanvasElement | null,
): number {
  const viewport = Math.max(0, (canvas?.clientHeight ?? 0) - RULER_HEIGHT);
  const content = trackCount * TRACK_HEIGHT;
  return Math.min(Math.max(0, content - viewport), Math.max(0, value));
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Where a clip sits on the canvas, or null if its track is gone. */
function clipRect(
  clip: Clip,
  rows: readonly Track[],
  secondsPerPixel: number,
  scrollLeft: number,
  trackScroll: number,
): Rect | null {
  const rowIndex = rows.findIndex((track) => track.id === clip.trackId);
  if (rowIndex < 0) return null;
  return {
    x: (clip.start - scrollLeft) / secondsPerPixel,
    y: RULER_HEIGHT + rowIndex * TRACK_HEIGHT - trackScroll,
    width: clip.duration / secondsPerPixel,
    height: TRACK_HEIGHT,
  };
}

/** A drag band has a start and a current corner; this gives it a positive size. */
function normalise(band: { originX: number; originY: number; x: number; y: number }): Rect {
  return {
    x: Math.min(band.originX, band.x),
    y: Math.min(band.originY, band.y),
    width: Math.abs(band.x - band.originX),
    height: Math.abs(band.y - band.originY),
  };
}

/** Touching counts as intersecting, so a band brushing a clip catches it. */
function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Picks a tick spacing that keeps labels readable at any zoom. */
function tickInterval(secondsPerPixel: number): number {
  const candidates = [1 / 30, 0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600];
  return candidates.find((seconds) => seconds / secondsPerPixel >= 90) ?? candidates.at(-1)!;
}

function drawRuler(
  context: CanvasRenderingContext2D,
  width: number,
  scrollLeft: number,
  secondsPerPixel: number,
  frameRate: number,
) {
  context.fillStyle = COLORS.ruler;
  context.fillRect(0, 0, width, RULER_HEIGHT);
  context.strokeStyle = COLORS.hairline;
  context.beginPath();
  context.moveTo(0, RULER_HEIGHT - 0.5);
  context.lineTo(width, RULER_HEIGHT - 0.5);
  context.stroke();

  const interval = tickInterval(secondsPerPixel);
  context.fillStyle = COLORS.text;
  context.font = '10px "Cabinet Grotesk", system-ui, sans-serif';
  context.textBaseline = "middle";

  for (
    let seconds = Math.ceil(scrollLeft / interval) * interval;
    (seconds - scrollLeft) / secondsPerPixel < width;
    seconds += interval
  ) {
    const x = Math.round((seconds - scrollLeft) / secondsPerPixel) + 0.5;
    context.strokeStyle = COLORS.tickMajor;
    context.beginPath();
    context.moveTo(x, RULER_HEIGHT - 7);
    context.lineTo(x, RULER_HEIGHT);
    context.stroke();
    context.fillText(timecode(seconds, frameRate), x + 5, RULER_HEIGHT / 2 - 2);
  }
}

/** Height of the coloured name strip at the top of every clip. */
const CLIP_HEADER = 17;
const LABEL_FONT = '11px "Cabinet Grotesk", system-ui, sans-serif';

/**
 * Truncates text to fit, with an ellipsis.
 *
 * Canvas has no built-in equivalent of `text-overflow: ellipsis`. The
 * `maxWidth` argument to `fillText` is not it - that *condenses* the glyphs to
 * fit, which is what made long clip names look crushed rather than cut.
 *
 * Results are memoised because this runs for every visible clip on every
 * frame, and `measureText` forces text shaping each time it is called.
 */
const labelCache = new Map<string, string>();

function ellipsize(context: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (maxWidth <= 0) return "";

  // Quantise the width so that a clip being dragged does not miss the cache on
  // every single frame.
  const key = `${text}|${Math.round(maxWidth / 4)}`;
  const cached = labelCache.get(key);
  if (cached !== undefined) return cached;

  let result = text;
  if (context.measureText(text).width > maxWidth) {
    let low = 0;
    let high = text.length;
    while (low < high) {
      const mid = Math.ceil((low + high) / 2);
      if (context.measureText(`${text.slice(0, mid)}…`).width <= maxWidth) low = mid;
      else high = mid - 1;
    }
    result = low > 0 ? `${text.slice(0, low)}…` : "";
  }

  // Plain cap rather than a real LRU: names are few and short-lived enough
  // that eviction order does not matter.
  if (labelCache.size > 512) labelCache.clear();
  labelCache.set(key, result);
  return result;
}

/** Clip fills, refreshed alongside COLORS. See `refreshCanvasPalette`. */
const PALETTE = {
  video: {
    body: "#142640",
    header: "#4d8df6",
    edge: "#4d8df6",
  },
  audio: {
    body: "#33220e",
    header: "#f09e2f",
    edge: "#f09e2f",
    wave: "rgba(250,214,158,0.7)",
  },
  image: {
    body: "#251a3d",
    header: "#8b5cf6",
    edge: "#8b5cf6",
  },
  // Rose, distinct from the three media kinds: a title is the one thing on
  // the timeline that is not a file, and it should not be mistaken for one.
  text: {
    body: "#38131d",
    header: "#e8355e",
    edge: "#e8355e",
  },
};

function drawClip(
  context: CanvasRenderingContext2D,
  clip: Clip,
  x: number,
  trackY: number,
  width: number,
  selected: boolean,
  assets: MediaAssets,
  secondsPerPixel: number,
) {
  const y = trackY + 4;
  const height = TRACK_HEIGHT - 10;
  const drawWidth = Math.max(2, width);
  const palette =
    clip.kind === "video"
      ? PALETTE.video
      : clip.kind === "image"
        ? PALETTE.image
        : clip.kind === "text"
          ? PALETTE.text
          : PALETTE.audio;
  const bodyY = y + CLIP_HEADER;
  const bodyHeight = height - CLIP_HEADER;

  context.save();
  // One clip path for the whole clip: artwork drawn inside it cannot bleed
  // past the rounded corners, so no separate masking is needed per layer.
  context.beginPath();
  context.roundRect(x, y, drawWidth, height, 5);
  context.clip();

  context.fillStyle = palette.body;
  context.fillRect(x, y, drawWidth, height);

  if (bodyHeight > 4) {
    if (clip.kind === "audio") {
      const peaks = assets.peaks.get(clip.mediaId);
      if (peaks) {
        drawWaveform(context, peaks, clip, x, bodyY, drawWidth, bodyHeight, secondsPerPixel);
      }
    } else if (clip.kind === "text") {
      // A title has no media to draw, so the body shows the words themselves -
      // which is also the fastest way to tell two titles apart at a glance.
      const words = clip.text?.content.replace(/\s+/g, " ").trim() ?? "";
      if (words && drawWidth > 24) {
        context.save();
        context.font = LABEL_FONT;
        context.fillStyle = "rgba(255,228,178,0.72)";
        context.textBaseline = "middle";
        context.fillText(
          ellipsize(context, words, drawWidth - 14),
          x + 7,
          bodyY + bodyHeight / 2,
        );
        context.restore();
      }
    } else {
      // Video and stills share this path: a still is cached as a one-frame
      // filmstrip, so tiling it repeats the same picture along the clip.
      const strip = assets.strips.get(clip.mediaId);
      const frames = assets.stripFrames.get(clip.mediaId);
      if (strip && frames) {
        drawFilmstrip(context, strip, frames, x, bodyY, drawWidth, bodyHeight);
      }
    }
  }

  if (clip.fadeIn > 0 || clip.fadeOut > 0) {
    drawFades(context, clip, x, bodyY, drawWidth, bodyHeight, secondsPerPixel);
  }

  context.fillStyle = palette.header;
  context.fillRect(x, y, drawWidth, CLIP_HEADER);

  // An "fx" chip in the header when effects are live on the clip, so a styled
  // clip is tellable from a plain one without opening a panel.
  const liveEffects = clip.videoEffects.some((effect) => effect.enabled !== false);
  const chipWidth = liveEffects && drawWidth > 60 ? 20 : 0;

  if (drawWidth > 30) {
    context.fillStyle = COLORS.clipText;
    context.font = LABEL_FONT;
    context.textBaseline = "middle";
    const label = ellipsize(context, clip.name, drawWidth - 14 - chipWidth);
    if (label) context.fillText(label, x + 7, y + CLIP_HEADER / 2 + 0.5);
  }

  if (chipWidth > 0) {
    context.fillStyle = "rgba(0,0,0,0.35)";
    context.beginPath();
    context.roundRect(x + drawWidth - chipWidth + 2, y + 2.5, chipWidth - 6, CLIP_HEADER - 5, 3);
    context.fill();
    context.fillStyle = COLORS.clipText;
    context.font = '9px "Cabinet Grotesk", system-ui, sans-serif';
    context.fillText("fx", x + drawWidth - chipWidth + 6, y + CLIP_HEADER / 2 + 0.5);
  }

  // A transition on the cut into this clip: a small wedge pair at the left
  // edge, the mark every editor uses for "these two dissolve".
  if (clip.transitionIn && drawWidth > 16) {
    const size = Math.min(9, bodyHeight / 2);
    const midY = bodyY + bodyHeight / 2;
    context.fillStyle = "rgba(255,255,255,0.8)";
    context.beginPath();
    context.moveTo(x + 1, midY - size);
    context.lineTo(x + 1 + size, midY);
    context.lineTo(x + 1, midY + size);
    context.closePath();
    context.fill();
    context.fillStyle = "rgba(255,255,255,0.45)";
    context.beginPath();
    context.moveTo(x + 1 + size, midY - size);
    context.lineTo(x + 1, midY);
    context.lineTo(x + 1 + size, midY + size);
    context.closePath();
    context.fill();
  }

  context.restore();

  context.beginPath();
  context.roundRect(x, y, drawWidth, height, 5);
  context.strokeStyle = selected ? COLORS.clipSelected : palette.edge;
  context.lineWidth = selected ? 2 : 1;
  context.stroke();
}

/**
 * Draws the fade ramps as wedges over the clip body.
 *
 * The shaded area is the part being attenuated and the bright line is the
 * envelope itself - the same shape every editor draws, because it reads as
 * "this much is being taken away" at a glance.
 *
 * On a video clip the wedge is confined to a band at the bottom. The fade is
 * an *audio* property, and shading the picture would say it fades to black.
 */
function drawFades(
  context: CanvasRenderingContext2D,
  clip: Clip,
  x: number,
  bodyY: number,
  width: number,
  bodyHeight: number,
  secondsPerPixel: number,
) {
  const band = clip.kind === "audio" ? bodyHeight : Math.min(10, bodyHeight);
  const top = bodyY + bodyHeight - band;
  const bottom = bodyY + bodyHeight;
  if (band <= 1) return;

  context.save();
  context.beginPath();
  context.rect(x, top, width, band);
  context.clip();

  const ramp = (seconds: number, fromLeft: boolean) => {
    const span = Math.min(seconds / secondsPerPixel, width);
    if (span < 1) return;

    const originX = fromLeft ? x : x + width;
    const endX = fromLeft ? x + span : x + width - span;

    context.fillStyle = "rgba(0,0,0,0.42)";
    context.beginPath();
    context.moveTo(originX, top);
    context.lineTo(endX, top);
    context.lineTo(originX, bottom);
    context.closePath();
    context.fill();

    context.strokeStyle = "rgba(255,255,255,0.75)";
    context.lineWidth = 1.5;
    context.beginPath();
    context.moveTo(originX, bottom);
    context.lineTo(endX, top);
    context.stroke();
  };

  if (clip.fadeIn > 0) ramp(clip.fadeIn, true);
  if (clip.fadeOut > 0) ramp(clip.fadeOut, false);

  context.restore();
}

/**
 * Tiles frames from a single strip image across the clip body.
 *
 * Each frame is drawn at its natural aspect ratio for the available height,
 * and which frame appears is chosen by how far along the clip it sits - so a
 * long clip repeats frames rather than stretching four of them.
 */
function drawFilmstrip(
  context: CanvasRenderingContext2D,
  strip: ImageBitmap,
  frames: number,
  x: number,
  y: number,
  width: number,
  height: number,
) {
  const tileWidth = strip.width / frames;
  const drawWidth = height * (tileWidth / strip.height);
  if (!Number.isFinite(drawWidth) || drawWidth <= 0) return;

  context.imageSmoothingQuality = "high";

  const columns = Math.ceil(width / drawWidth);
  for (let column = 0; column < columns; column += 1) {
    const fraction = columns > 1 ? column / (columns - 1) : 0;
    const index = Math.min(frames - 1, Math.round(fraction * (frames - 1)));
    context.drawImage(
      strip,
      index * tileWidth,
      0,
      tileWidth,
      strip.height,
      x + column * drawWidth,
      y,
      drawWidth,
      height,
    );
  }
}

/**
 * Draws the cached peaks, mirrored about a centre line.
 *
 * Zoomed out, one pixel covers many buckets, so each column takes the extremes
 * across its whole range - otherwise transients disappear and loud passages
 * look quiet. The scan is capped at eight samples per column so that zooming
 * all the way out stays cheap.
 */
function drawWaveform(
  context: CanvasRenderingContext2D,
  peaks: Peaks,
  clip: Clip,
  x: number,
  y: number,
  width: number,
  height: number,
  secondsPerPixel: number,
) {
  const centre = y + height / 2;
  const half = height / 2 - 1;
  // The drawn amplitude follows the clip's gain, so turning a clip up makes
  // its waveform visibly taller. Clamped, because a boosted waveform that
  // overflowed its clip would bleed into the lane above.
  const gain = Math.max(0, clip.volume);
  const perPixel = Math.max(1, Math.round(secondsPerPixel * peaks.bucketsPerSecond));
  const stride = Math.max(1, Math.floor(perPixel / 8));

  context.fillStyle = PALETTE.audio.wave;
  context.beginPath();

  for (let column = 0; column < width; column += 1) {
    const from = Math.floor((clip.sourceStart + column * secondsPerPixel) * peaks.bucketsPerSecond);
    if (from < 0 || from >= peaks.max.length) continue;
    const to = Math.min(from + perPixel, peaks.max.length);

    let low = 0;
    let high = 0;
    for (let bucket = from; bucket < to; bucket += stride) {
      if (peaks.min[bucket] < low) low = peaks.min[bucket];
      if (peaks.max[bucket] > high) high = peaks.max[bucket];
    }

    const top = centre - Math.min(1, high * gain) * half;
    const bottom = centre - Math.max(-1, low * gain) * half;
    context.rect(x + column, top, 1, Math.max(1, bottom - top));
  }

  context.fill();

  context.strokeStyle = "rgba(255,255,255,0.18)";
  context.lineWidth = 1;
  context.beginPath();
  context.moveTo(x, Math.round(centre) + 0.5);
  context.lineTo(x + width, Math.round(centre) + 0.5);
  context.stroke();
}

function drawPlayhead(context: CanvasRenderingContext2D, height: number, x: number) {
  const position = Math.round(x) + 0.5;
  context.strokeStyle = COLORS.playhead;
  context.lineWidth = 1;
  context.beginPath();
  context.moveTo(position, 0);
  context.lineTo(position, height);
  context.stroke();

  context.fillStyle = COLORS.playhead;
  context.beginPath();
  context.moveTo(position - 5, 0);
  context.lineTo(position + 5, 0);
  context.lineTo(position, 9);
  context.closePath();
  context.fill();
}
