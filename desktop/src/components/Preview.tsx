// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

import { buildPreviewLook, type AppliedEffect, type CanvasOp } from "../lib/effects";
import type { PreviewSource, TextOverlay } from "../lib/monitor";
import { timecode } from "../lib/time";
import { MAX_SCALE, MIN_SCALE } from "../lib/editor";
import { useLocale, type MsgKey } from "../lib/i18n";
import { textCss } from "../lib/text";
import { Icon, IconButton } from "./Icon";
import { Menu } from "./Menu";
import { PANEL_SHELL } from "./Panel";

/**
 * Output frames people actually deliver to. Dimensions rather than bare
 * ratios, because a ratio alone does not say how many pixels the export has.
 */
const FRAME_PRESETS = [
  { label: "16:9", width: 1920, height: 1080 },
  { label: "16:9 · 4K", width: 3840, height: 2160 },
  { label: "9:16", width: 1080, height: 1920 },
  { label: "1:1", width: 1080, height: 1080 },
  { label: "4:3", width: 1440, height: 1080 },
  { label: "21:9", width: 2560, height: 1080 },
] as const;

/**
 * A tiny rectangle in the aspect of a frame size, so "vertical" and
 * "landscape" read at a glance before the numbers do.
 *
 * Normalised so the long side is always the same length - the shapes differ
 * only in proportion, which is the one thing they exist to show. The current
 * size fills with accent, making the shape double as the selection mark.
 */
function RatioShape({
  width,
  height,
  active = false,
}: {
  width: number;
  height: number;
  active?: boolean;
}) {
  const long = 13;
  const shapeWidth = width >= height ? long : Math.max(5, Math.round((long * width) / height));
  const shapeHeight = height >= width ? long : Math.max(5, Math.round((long * height) / width));
  return (
    <span
      aria-hidden
      className={`inline-block rounded-[2px] border ${
        active ? "border-accent bg-accent-soft" : "border-current opacity-50"
      }`}
      style={{ width: shapeWidth, height: shapeHeight }}
    />
  );
}

/** "16:9" for a preset size, the reduced fraction for anything else. */
function ratioLabel(width: number, height: number): string {
  const preset = FRAME_PRESETS.find(
    (candidate) => candidate.width === width && candidate.height === height,
  );
  if (preset) return preset.label.split(" ")[0];
  const gcd = (a: number, b: number): number => (b === 0 ? a : gcd(b, a % b));
  const divisor = gcd(width, height) || 1;
  return `${width / divisor}:${height / divisor}`;
}

/** The video clip that should be on screen right now, if any. */

/** How the displayed clip's picture sits in the frame. Mirrors the engine's
 * `Transform`: scale over the fitted size, offsets as frame fractions from
 * centre, clockwise degrees about the picture's centre. */
export interface PreviewTransform {
  scale: number;
  offsetX: number;
  offsetY: number;
  rotation: number;
}


/** Alignment guides live while something is being dragged: vertical lines at
 * `x` and horizontal ones at `y`, as fractions of the frame. Lists, because a
 * scale snap on a centred picture kisses both bounds of an axis at once and
 * should say so with both lines. Empty = none. */
interface Guides {
  x: number[];
  y: number[];
}

const NO_GUIDES: Guides = { x: [], y: [] };

/** How close an alignment has to be before it takes, in screen pixels. Small
 * on purpose: a guide should feel like a nudge you barely notice accepting,
 * never like a magnet fighting the drag. */
const SNAP_PX = 6;

/** Chrome shared by every draggable box in the monitor. */
const HANDLE_CLASS =
  "absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-[3px] border border-black/50 bg-white shadow-sm";

/**
 * A window-scoped pointer drag.
 *
 * Listeners go on the window because the pointer leaves a ten-pixel handle on
 * the first movement. Moves are coalesced to one per animation frame: writing
 * the project re-renders the whole editor, and macOS delivers pointer events
 * faster than frames paint. Returns a detach that is safe to call twice - the
 * component may unmount mid-drag when the playhead runs off the clip.
 */
function startDrag(onFrame: (pointer: PointerEvent) => void, onEnd: () => void): () => void {
  let pending: PointerEvent | null = null;
  let scheduled = 0;
  let done = false;

  const flush = () => {
    scheduled = 0;
    if (pending) onFrame(pending);
    pending = null;
  };
  const move = (pointer: PointerEvent) => {
    pending = pointer;
    if (!scheduled) scheduled = requestAnimationFrame(flush);
  };
  const finish = () => {
    if (done) return;
    done = true;
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", finish);
    if (scheduled) cancelAnimationFrame(scheduled);
    // The last computed position still lands - releasing mid-frame should not
    // lose the final pixel of the drag.
    flush();
    onEnd();
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", finish);
  return finish;
}

/**
 * Softly snaps one axis of a centre offset while dragging.
 *
 * Candidates are the frame's centre and, for an axis-aligned picture, its two
 * edges. Within `SNAP_PX` the value lands exactly on the target and the
 * matching guide line is reported; outside it the raw value passes through
 * untouched - there is no pull, no resistance, just a small final settle.
 */
function softSnap(
  raw: number,
  frameSpan: number,
  extent: number | null,
): { value: number; guide: number | null } {
  const targets = [{ value: 0, guide: 0.5 }];
  if (extent !== null) {
    targets.push(
      { value: (extent - frameSpan) / 2 / frameSpan, guide: 0 },
      { value: (frameSpan - extent) / 2 / frameSpan, guide: 1 },
    );
  }
  for (const target of targets) {
    if (Math.abs(raw - target.value) * frameSpan < SNAP_PX) return target;
  }
  return { value: raw, guide: null };
}

/**
 * Softly snaps a scale while a corner is being dragged.
 *
 * The candidates are the scales at which a picture edge coincides with a
 * frame bound, given where the picture's centre currently sits - scale a
 * landscape clip up inside a vertical frame and it settles exactly where its
 * top and bottom meet the frame's. The same `SNAP_PX` law as `softSnap`,
 * measured at the moving edge, so the catch feels identical to move-snapping.
 * A centred picture reaches both bounds of an axis at the same scale and
 * lights both guides. `fittedX`/`fittedY` are the picture's unscaled extents
 * per axis, null when the picture is tilted and edges cannot line up.
 */
function snapScale(
  raw: number,
  geometry: {
    fittedX: number | null;
    fittedY: number | null;
    centreX: number;
    centreY: number;
    frameWidth: number;
    frameHeight: number;
  },
): { value: number; guides: Guides } {
  const candidates: { scale: number; axis: "x" | "y"; guide: number; slack: number }[] = [];
  const consider = (scale: number, axis: "x" | "y", guide: number, halfExtent: number) => {
    if (scale <= MIN_SCALE || scale > MAX_SCALE) return;
    const slack = Math.abs(raw - scale) * halfExtent;
    if (slack < SNAP_PX) candidates.push({ scale, axis, guide, slack });
  };
  if (geometry.fittedX !== null) {
    consider((2 * geometry.centreX) / geometry.fittedX, "x", 0, geometry.fittedX / 2);
    consider(
      (2 * (geometry.frameWidth - geometry.centreX)) / geometry.fittedX,
      "x",
      1,
      geometry.fittedX / 2,
    );
  }
  if (geometry.fittedY !== null) {
    consider((2 * geometry.centreY) / geometry.fittedY, "y", 0, geometry.fittedY / 2);
    consider(
      (2 * (geometry.frameHeight - geometry.centreY)) / geometry.fittedY,
      "y",
      1,
      geometry.fittedY / 2,
    );
  }
  if (candidates.length === 0) return { value: raw, guides: NO_GUIDES };

  candidates.sort((left, right) => left.slack - right.slack);
  const winner = candidates[0].scale;
  const guides: Guides = { x: [], y: [] };
  for (const candidate of candidates) {
    if (Math.abs(candidate.scale - winner) < 1e-6) guides[candidate.axis].push(candidate.guide);
  }
  return { value: winner, guides };
}

/**
 * The program monitor.
 *
 * A `<video>` element, synced to the transport the same way audio is. This is
 * a *preview*, not the engine's output: it shows one clip at a time, so it
 * cannot composite, and it knows nothing about effects or the render graph.
 * What it does do is let you see and hear your edit today.
 *
 * The engine takes this over when it can present frames - see
 * docs/decisions/0002. Until then the element keeps a strict aspect box,
 * because a native surface will eventually be positioned to exactly these
 * bounds and the layout has to be right before anything can be aligned to it.
 */
export function Preview({
  source,
  overlays: titleOverlays,
  playing,
  playhead,
  duration,
  frameRate,
  frame,
  quality,
  onQualityChange,
  transform,
  opacity,
  effects,
  ghost,
  engineStill,
  veil,
  mediaSize,
  selectedClipId,
  onSelectClip,
  onTransformChange,
  onTransformEnd,
  onOverlayChange,
  onOverlayEnd,
  onFrameChange,
  onTogglePlay,
  onStep,
  onSeek,
}: {
  source: PreviewSource | null;
  /** Text clips live at the playhead, bottom-most first. */
  overlays: TextOverlay[];
  playing: boolean;
  playhead: number;
  duration: number;
  frameRate: number;
  /** The project's output size, shown and changed in the footer. */
  frame: { width: number; height: number };
  /** Engine-preview resolution as a fraction of the output frame. */
  quality: number;
  onQualityChange: (quality: number) => void;
  /** The displayed clip's picture transform. Null when nothing is showing. */
  transform: PreviewTransform | null;
  /** The displayed clip's blend strength, 1 being solid. */
  opacity: number;
  /** The displayed clip's video effects, drawn live. Null for none. */
  effects: AppliedEffect[] | null;
  /** A cross-fade in progress: the incoming clip's pre-roll, faded in over
   * the picture. Null outside a dissolve window. */
  ghost: { clipId: string; path: string; time: number; speed: number; opacity: number } | null;
  /** The engine's true composite for the paused playhead - the exporter's own
   * plan and compositor. Drawn over the approximation while it holds. */
  engineStill: { bytes: ArrayBuffer; width: number; height: number } | null;
  /** A fade-to-colour transition passing over the playhead: a coloured wash
   * whose opacity the app computes per frame. Null when no fade is live. */
  veil: { color: string; opacity: number } | null;
  /** The source's pixel size, when the probe reported one. */
  mediaSize: { width: number; height: number } | null;
  /** The single selected clip, if there is one - its box shows handles. */
  selectedClipId: string | null;
  onSelectClip: (clipId: string) => void;
  onTransformChange: (clipId: string, transform: Partial<PreviewTransform>) => void;
  /** The gizmo drag ended - the echoed transform becomes one engine command. */
  onTransformEnd: () => void;
  /** Edits to a title dragged in the monitor: position, and size from the
   * corner handles. */
  onOverlayChange: (
    clipId: string,
    change: { offsetX?: number; offsetY?: number; fontSize?: number },
  ) => void;
  /** The title drag ended - the echoed change becomes one engine command. */
  onOverlayEnd: () => void;
  onFrameChange: (width: number, height: number) => void;
  onTogglePlay: () => void;
  onStep: (frames: number) => void;
  onSeek: (seconds: number) => void;
}) {
  const { t } = useLocale();
  const video = useRef<HTMLVideoElement>(null);
  const still = useRef<HTMLImageElement>(null);
  const loadedClip = useRef<string | null>(null);

  // The picture's true pixel size, measured off the element once it has
  // decoded. The probe's numbers are the *coded* size, which lies for rotated
  // phone footage and anything anamorphic; `videoWidth`/`naturalWidth` are
  // what `object-contain` actually lays out, so the gizmo box is derived from
  // the same truth as the picture and the two can never disagree.
  const [pictureSize, setPictureSize] = useState<{ width: number; height: number } | null>(null);
  useEffect(() => {
    setPictureSize(null);
  }, [source?.clipId]);

  // Alignment guides, drawn while a drag holds one. State lives here because
  // both the picture's gizmo and every title box report into the same lines.
  const [guides, setGuides] = useState<Guides>(NO_GUIDES);

  // Text is sized as a fraction of the frame, so drawing it needs the pixel
  // height of the surface it lands on. That is a layout fact, not a prop, and
  // it changes whenever the panel is resized - hence an observer rather than a
  // one-off measurement.
  const stage = useRef<HTMLDivElement>(null);
  const [surface, setSurface] = useState({ width: 0, height: 0 });

  useLayoutEffect(() => {
    const element = stage.current;
    if (!element) return;

    const measure = () => {
      const bounds = element.getBoundingClientRect();
      setSurface({ width: bounds.width, height: bounds.height });
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // The frame, letterboxed into the stage. Transforms are fractions *of the
  // frame*, so drawing them against the raw panel would lie whenever the panel
  // and the project have different shapes - a 9:16 edit in a wide panel most
  // of all. Everything visual lives inside this box; the compositor will clip
  // at its edge, so the preview does too.
  const frameRect = useMemo(() => {
    if (surface.width <= 0 || surface.height <= 0) return null;
    const fit = Math.min(surface.width / frame.width, surface.height / frame.height);
    const width = frame.width * fit;
    const height = frame.height * fit;
    return {
      left: (surface.width - width) / 2,
      top: (surface.height - height) / 2,
      width,
      height,
    };
  }, [surface, frame]);
  const hasFrame = frameRect !== null;

  // The same placement the exporter applies, as CSS. Order matters and matches
  // the compositor: scale about the picture's centre, rotate about it, then
  // translate. (CSS applies the list right to left.)
  // Everything the clip's effects need to draw live: the CSS/SVG filter
  // string, overlay layers, canvas geometry passes, and shake jitter -
  // assembled in applied order, scaled to preview pixels.
  const pixelScale = frameRect ? frameRect.width / frame.width : 1;
  const look = useMemo(
    () => buildPreviewLook(effects ?? undefined, pixelScale),
    [effects, pixelScale],
  );

  // Deterministic shake, evaluated from the playhead clock - the exact
  // `sin(t*f)` expression the export's jitter crop runs, so pausing anywhere
  // shows the same displaced frame the file will have.
  const jitterX = look.jitter
    ? -look.jitter.amount * pixelScale * Math.sin(playhead * look.jitter.speed)
    : 0;
  const jitterY = look.jitter
    ? -look.jitter.amount * pixelScale * Math.cos(playhead * look.jitter.speed * 1.3)
    : 0;

  const mediaCss =
    transform && frameRect
      ? {
          transform: `translate(${transform.offsetX * frameRect.width + jitterX}px, ${
            transform.offsetY * frameRect.height + jitterY
          }px) rotate(${transform.rotation}deg) scale(${transform.scale})`,
          // The same blend the compositor applies on export. On the wrapper
          // with the transform, so the whole picture fades as one surface.
          opacity: Math.min(1, Math.max(0, opacity)),
          // The live effect filters. On the wrapper, so the canvas pass
          // underneath inherits them too.
          filter: look.filter ?? undefined,
        }
      : undefined;

  // Swap the source only when the clip under the playhead actually changes.
  // Reassigning `src` every frame would restart the decoder continuously.
  // `hasFrame` is a dependency because the element only exists once the stage
  // has been measured - the first run of this effect finds no element at all.
  useEffect(() => {
    const element = video.current;
    if (!element) {
      loadedClip.current = null;
      return;
    }

    // A still never goes near the video element - it is rendered as an <img>
    // below - so release whatever the element was holding.
    if (!source || source.isStill) {
      loadedClip.current = null;
      element.removeAttribute("src");
      element.load();
      return;
    }

    if (loadedClip.current !== source.clipId) {
      loadedClip.current = source.clipId;
      element.src = convertFileSrc(source.path);
      element.load();
    }
  }, [source, hasFrame]);

  // Same corrective sync as the audio preview: generous tolerance while
  // playing (a reseek is a visible stutter), tight while paused (a seek is the
  // entire operation).
  useEffect(() => {
    const element = video.current;
    if (!element || !source || source.isStill) return;

    // The element free-runs between corrections, so it has to free-run at the
    // clip's rate. Clamped to what media elements reliably accept.
    const rate = Math.min(16, Math.max(0.0625, source.speed));
    if (element.playbackRate !== rate) element.playbackRate = rate;

    const tolerance = playing ? 0.3 : 0.03;
    if (element.readyState > 0 && Math.abs(element.currentTime - source.time) > tolerance) {
      try {
        element.currentTime = Math.max(0, source.time);
      } catch {
        // Still opening; the next update lands it.
      }
    }

    if (playing && element.paused) void element.play().catch(() => undefined);
    if (!playing && !element.paused) element.pause();
  }, [source, playing, hasFrame]);

  return (
    <div className={PANEL_SHELL}>
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-hairline px-3">
        <h2 className="text-[11px] font-semibold uppercase tracking-wider text-secondary">
          Preview
        </h2>
      </header>

      {/*
        Containment, deliberately belt and braces.

        A flex item defaults to `min-height: auto`, so a 4K video's intrinsic
        size can push a flex container past its parent no matter what
        `max-h-full` says. Rather than rely on that, the stage clips, and the
        media is absolutely positioned to fill a box that is exactly the
        stage's content area. `object-contain` then fits the picture inside
        that box, so it can never exceed the panel whatever its resolution.
      */}
      <div className="min-h-0 min-w-0 flex-1 overflow-hidden bg-stage p-4">
        <div ref={stage} data-preview-surface className="relative h-full w-full">
          {frameRect && (
            <div
              // The frame itself. It clips, because the compositor clips: a
              // picture dragged past the edge is gone in the export, so it has
              // to be gone here too. The ring is the only hint of where the
              // frame ends on an all-black stage.
              className="absolute overflow-hidden ring-1 ring-white/15"
              style={{
                left: frameRect.left,
                top: frameRect.top,
                width: frameRect.width,
                height: frameRect.height,
              }}
            >
              {/* Picture only. Every drop of audio - a video clip's included -
                  comes from the engine's mixer, so there is exactly one clock
                  and one gain law. */}
              {/* The picture's transform lives on a wrapper rather than the
                  media element itself: WKWebView keeps `<video>` in its own
                  compositing layer, and animating transforms directly on it
                  is unreliable where a plain div never is. */}
              {/* The SVG filters the effect chain references via url(#id).
                  Zero-sized and inert; only the defs matter. */}
              {look.svgFilters.length > 0 && (
                <svg width="0" height="0" className="absolute" aria-hidden>
                  <defs
                    dangerouslySetInnerHTML={{
                      __html: look.svgFilters
                        .map(
                          (entry) =>
                            `<filter id="${entry.id}" color-interpolation-filters="sRGB">${entry.content}</filter>`,
                        )
                        .join(""),
                    }}
                  />
                </svg>
              )}

              <div className="absolute inset-0" style={mediaCss}>
                <video
                  ref={video}
                  muted
                  playsInline
                  onLoadedMetadata={(event) =>
                    setPictureSize({
                      width: event.currentTarget.videoWidth,
                      height: event.currentTarget.videoHeight,
                    })
                  }
                  className={`absolute inset-0 h-full w-full object-contain ${
                    source && !source.isStill ? "block" : "hidden"
                  }`}
                  style={look.canvas.length > 0 ? { visibility: "hidden" } : undefined}
                />
                {source?.isStill && (
                  <img
                    // Keyed on the clip so switching between two stills
                    // actually swaps the picture rather than reusing the
                    // decoded one.
                    key={source.clipId}
                    ref={still}
                    src={convertFileSrc(source.path)}
                    alt=""
                    draggable={false}
                    onLoad={(event) =>
                      setPictureSize({
                        width: event.currentTarget.naturalWidth,
                        height: event.currentTarget.naturalHeight,
                      })
                    }
                    className="absolute inset-0 h-full w-full object-contain"
                    style={look.canvas.length > 0 ? { visibility: "hidden" } : undefined}
                  />
                )}

                {/* Geometry effects - pixelate, mirror, fisheye - redraw the
                    media on a canvas the wrapper's filters still apply to.
                    The media element stays live underneath, just invisible:
                    it is the decoder this canvas samples. */}
                {look.canvas.length > 0 && source && (
                  <EffectCanvas
                    ops={look.canvas}
                    pixelScale={pixelScale}
                    source={() => (source.isStill ? still.current : video.current)}
                  />
                )}
              </div>

              {/* Effect overlay layers: vignette's gradient, grain's animated
                  noise. Over the picture, under the incoming dissolve. */}
              {look.overlays.map((overlay, index) => (
                <div
                  key={index}
                  className={`pointer-events-none absolute ${
                    overlay.grain ? "-inset-[12%] grain-layer" : "inset-0"
                  }`}
                  style={overlay.style as React.CSSProperties}
                />
              ))}

              {/* A cross-fade in progress: the incoming clip's pre-roll,
                  ramping in over everything above - the dissolve itself. */}
              {ghost && ghost.opacity > 0 && (
                <GhostVideo key={ghost.clipId} ghost={ghost} playing={playing} />
              )}

              {/* The engine's true frame, once the paused playhead settles:
                  real multi-track compositing, exact effect chains. It covers
                  the approximation layers beneath; the veil and titles still
                  draw above, exactly as the exporter stacks them. */}
              {engineStill && <EngineStillLayer still={engineStill} />}

              {/* A fade-to-black/white transition washing over the cut. Above
                  the picture, below titles - titles ride through a fade the
                  same way the exporter composites them, on top. */}
              {veil && veil.opacity > 0 && (
                <div
                  className="pointer-events-none absolute inset-0"
                  style={{ backgroundColor: veil.color, opacity: Math.min(1, veil.opacity) }}
                />
              )}

              {/* Alignment guides, only alive mid-drag. Dashed and faint on
                  purpose: a hint that something lined up, not a grid. */}
              {guides.x.map((guide) => (
                <div
                  key={`x${guide}`}
                  className="pointer-events-none absolute inset-y-0 w-0 border-l border-dashed
                             border-white/45"
                  style={{ left: `${guide * 100}%` }}
                />
              ))}
              {guides.y.map((guide) => (
                <div
                  key={`y${guide}`}
                  className="pointer-events-none absolute inset-x-0 h-0 border-t border-dashed
                             border-white/45"
                  style={{ top: `${guide * 100}%` }}
                />
              ))}
            </div>
          )}

          {/* Above the frame and unclipped, so handles survive being dragged
              past the edge. */}
          {frameRect && source && transform && (
            <TransformGizmo
              clipId={source.clipId}
              transform={transform}
              frameRect={frameRect}
              mediaSize={pictureSize ?? mediaSize}
              selected={selectedClipId === source.clipId}
              onSelect={onSelectClip}
              onChange={onTransformChange}
              onChangeEnd={onTransformEnd}
              onGuides={setGuides}
            />
          )}

          {/*
            Titles sit above the picture *and* above the picture's gizmo: they
            draw on top, so they select on top - clicking a title must never
            fall through to the footage behind it. The layer clips like the
            frame does.
          */}
          {frameRect && titleOverlays.length > 0 && (
            <div
              // pointer-events-none is load-bearing: this div covers the whole
              // frame above the transform gizmo, and a transparent div still
              // hit-tests. Without it, nothing inside the frame is clickable -
              // each title box re-enables its own pointer events.
              className="pointer-events-none absolute overflow-hidden"
              style={{
                left: frameRect.left,
                top: frameRect.top,
                width: frameRect.width,
                height: frameRect.height,
              }}
            >
              {titleOverlays.map((overlay) => (
                <TextOverlayBox
                  key={overlay.clipId}
                  overlay={overlay}
                  frameRect={frameRect}
                  selected={selectedClipId === overlay.clipId}
                  onSelect={onSelectClip}
                  onChange={onOverlayChange}
                  onChangeEnd={onOverlayEnd}
                  onGuides={setGuides}
                />
              ))}
            </div>
          )}

          {!source && titleOverlays.length === 0 && (
            // No surface of its own: an empty monitor *is* black, and drawing
            // a bordered card on top of it invents an edge that means nothing.
            // Just the words, sitting on the stage.
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3
                            text-on-stage">
              <Icon name="film" size={30} strokeWidth={1.5} />
              <p className="text-xs">{t("preview.nothingUnderPlayhead")}</p>
            </div>
          )}
        </div>
      </div>

      {/*
        A three-column grid, not a flex row with `ml-auto`.

        Two things were making the transport jump as the timecode counted. The
        digits themselves can change width, and - worse - auto margins size the
        centre group from whatever space the sides leave, so *any* change on
        the left shoved the buttons sideways.

        `minmax(0, 1fr)` on both sides fixes the second properly: the columns
        are equal and cannot grow past their share, so the middle column is
        centred on the panel regardless of what the sides contain. The sides
        clip rather than push. `tabular-nums` then handles the first.
      */}
      <div
        className="grid h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center
                   gap-2 border-t border-hairline px-3"
      >
        <span className="min-w-0 truncate font-mono text-[11px] tabular-nums text-secondary">
          {timecode(playhead, frameRate)}
          <span className="text-tertiary"> / </span>
          <span className="text-tertiary">{timecode(duration, frameRate)}</span>
        </span>

        <div className="flex items-center gap-0.5">
          <IconButton
            icon="skipStart"
            label={t("preview.goToStart")}
            size={7}
            onClick={() => onSeek(0)}
          />
          <IconButton
            icon="stepBack"
            label={t("preview.prevFrame")}
            size={7}
            onClick={() => onStep(-1)}
          />
          <IconButton
            icon={playing ? "pause" : "play"}
            label={playing ? t("preview.pause") : t("preview.play")}
            onClick={onTogglePlay}
            tone="go"
            active={playing}
          />
          <IconButton
            icon="stepForward"
            label={t("preview.nextFrame")}
            size={7}
            onClick={() => onStep(1)}
          />
          <IconButton
            icon="skipEnd"
            label={t("preview.goToEnd")}
            size={7}
            onClick={() => onSeek(duration)}
          />
        </div>

        <span className="flex min-w-0 items-center gap-2 justify-self-end">
          <Menu
            align="right"
            direction="up"
            groups={[
              FRAME_PRESETS.map((preset) => {
                const active =
                  preset.width === frame.width && preset.height === frame.height;
                return {
                  label: `${preset.label} — ${preset.width} x ${preset.height}`,
                  leading: (
                    <RatioShape width={preset.width} height={preset.height} active={active} />
                  ),
                  onSelect: () => onFrameChange(preset.width, preset.height),
                };
              }),
            ]}
            trigger={(open) => (
              // Shape and chevron only: the proportions read at a glance,
              // and the numbers live in the tooltip and the menu rows -
              // spelling them out here crowded the tray for nothing.
              <span
                title={t("preview.outputSize", {
                  ratio: ratioLabel(frame.width, frame.height),
                  width: frame.width,
                  height: frame.height,
                })}
                className={`flex items-center gap-1 rounded-md px-1.5 py-1 transition-colors ${
                  open
                    ? "bg-active text-primary"
                    : "text-tertiary hover:bg-hover hover:text-secondary"
                }`}
              >
                <RatioShape width={frame.width} height={frame.height} />
                <Icon name="chevronDown" size={10} />
              </span>
            )}
          />
          <QualityMenu quality={quality} onQualityChange={onQualityChange} />
        </span>
      </div>
    </div>
  );
}

/**
 * Engine-preview resolutions, as fractions of the output frame. The labels
 * follow the convention every editor uses; the pixels follow the project.
 */
const PREVIEW_QUALITIES: { labelKey: MsgKey; ratio: string; value: number }[] = [
  { labelKey: "preview.full", ratio: "1:1", value: 1 },
  { labelKey: "preview.half", ratio: "1:2", value: 0.5 },
  { labelKey: "preview.quarter", ratio: "1:4", value: 0.25 },
];

/**
 * The preview-quality picker, next to the output-size menu it modifies:
 * what fraction of the frame the engine composites for the monitor. The
 * frame rate is not shown here - it lives in the Details panel, and the
 * tray reads better carrying only what can be changed from it.
 */
function QualityMenu({
  quality,
  onQualityChange,
}: {
  quality: number;
  onQualityChange: (quality: number) => void;
}) {
  const { t } = useLocale();
  const active =
    PREVIEW_QUALITIES.find((option) => option.value === quality) ?? PREVIEW_QUALITIES[1];
  return (
    <Menu
      align="right"
      direction="up"
      groups={[
        PREVIEW_QUALITIES.map((option) => ({
          label: `${t(option.labelKey)} — ${option.ratio}`,
          leading: (
            <span
              className={`h-1 w-1 rounded-full ${
                option.value === active.value ? "bg-current" : "bg-transparent"
              }`}
            />
          ),
          onSelect: () => onQualityChange(option.value),
        })),
      ]}
      trigger={(open) => (
        <span
          title={t("preview.qualityHint", { label: t(active.labelKey), ratio: active.ratio })}
          className={`flex items-center gap-1 rounded-md px-1.5 py-1 transition-colors ${
            open
              ? "bg-active text-primary"
              : "text-tertiary hover:bg-hover hover:text-secondary"
          }`}
        >
          <span className="font-technical text-[10px] tabular-nums">{active.ratio}</span>
          <Icon name="chevronDown" size={10} />
        </span>
      )}
    />
  );
}

/**
 * The engine's paused-frame composite, blitted into the frame box.
 *
 * The bytes arrive as raw RGBA at the request's own resolution; the canvas
 * holds them at that resolution and CSS scales it to the frame, which shares
 * its aspect by construction.
 */
function EngineStillLayer({
  still,
}: {
  still: { bytes: ArrayBuffer; width: number; height: number };
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;
    if (still.bytes.byteLength !== still.width * still.height * 4) return;
    canvas.width = still.width;
    canvas.height = still.height;
    context.putImageData(
      new ImageData(new Uint8ClampedArray(still.bytes), still.width, still.height),
      0,
      0,
    );
  }, [still]);

  return (
    <canvas
      ref={canvasRef}
      className="pointer-events-none absolute inset-0 h-full w-full"
      aria-hidden
    />
  );
}

/**
 * The canvas pass for geometry effects the CSS filter pipeline cannot do.
 *
 * Redraws the media element into a canvas every animation frame, applying
 * pixelate, mirror and fisheye in effect order. The canvas sits inside the
 * filtered wrapper, so colour and blur effects stack on top of the geometry
 * the same way the export's single FFmpeg chain runs them - approximate in
 * order, identical in look.
 */
function EffectCanvas({
  ops,
  pixelScale,
  source,
}: {
  ops: CanvasOp[];
  /** Preview pixels per export pixel, so block sizes match the file. */
  pixelScale: number;
  source: () => HTMLVideoElement | HTMLImageElement | null;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const state = useRef({ ops, pixelScale, source });
  state.current = { ops, pixelScale, source };

  useEffect(() => {
    const work = document.createElement("canvas");
    const scratch = document.createElement("canvas");
    let frame = 0;

    const tick = () => {
      frame = requestAnimationFrame(tick);
      const canvas = canvasRef.current;
      const media = state.current.source();
      const context = canvas?.getContext("2d");
      if (!canvas || !media || !context) return;

      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      if (width === 0 || height === 0) return;
      const ratio = window.devicePixelRatio || 1;
      if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
        canvas.width = Math.max(1, Math.round(width * ratio));
        canvas.height = Math.max(1, Math.round(height * ratio));
      }

      const naturalWidth =
        media instanceof HTMLVideoElement ? media.videoWidth : media.naturalWidth;
      const naturalHeight =
        media instanceof HTMLVideoElement ? media.videoHeight : media.naturalHeight;
      if (!naturalWidth || !naturalHeight) return;

      // The same contain-fit the hidden element's object-contain performs.
      const fit = Math.min(width / naturalWidth, height / naturalHeight);
      const drawWidth = naturalWidth * fit;
      const drawHeight = naturalHeight * fit;

      work.width = Math.max(1, Math.round(drawWidth * ratio));
      work.height = Math.max(1, Math.round(drawHeight * ratio));
      const workContext = work.getContext("2d");
      if (!workContext) return;
      workContext.imageSmoothingQuality = "high";
      workContext.drawImage(media, 0, 0, work.width, work.height);

      for (const op of state.current.ops) {
        applyCanvasOp(work, workContext, scratch, op, state.current.pixelScale * ratio);
      }

      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, height);
      context.drawImage(work, (width - drawWidth) / 2, (height - drawHeight) / 2, drawWidth, drawHeight);
    };

    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, []);

  return <canvas ref={canvasRef} className="absolute inset-0 h-full w-full" />;
}

/** Applies one geometry op to `work` in place, using `scratch` as the copy. */
function applyCanvasOp(
  work: HTMLCanvasElement,
  context: CanvasRenderingContext2D,
  scratch: HTMLCanvasElement,
  op: CanvasOp,
  pixelScale: number,
): void {
  const { width, height } = work;
  const scratchContext = scratch.getContext("2d");
  if (!scratchContext) return;

  if (op.kind === "pixelate") {
    // Downscale then nearest-neighbour upscale - the definition of pixelate.
    const block = Math.max(2, op.size * pixelScale);
    const smallWidth = Math.max(1, Math.round(width / block));
    const smallHeight = Math.max(1, Math.round(height / block));
    scratch.width = smallWidth;
    scratch.height = smallHeight;
    scratchContext.imageSmoothingQuality = "high";
    scratchContext.drawImage(work, 0, 0, smallWidth, smallHeight);
    context.imageSmoothingEnabled = false;
    context.clearRect(0, 0, width, height);
    context.drawImage(scratch, 0, 0, width, height);
    context.imageSmoothingEnabled = true;
    return;
  }

  if (op.kind === "mirror") {
    scratch.width = width;
    scratch.height = height;
    scratchContext.drawImage(work, 0, 0);
    const half = Math.floor(width / 2);
    context.clearRect(0, 0, width, height);
    context.drawImage(scratch, 0, 0, half, height, 0, 0, half, height);
    context.save();
    context.translate(width, 0);
    context.scale(-1, 1);
    context.drawImage(scratch, 0, 0, half, height, 0, 0, half, height);
    context.restore();
    return;
  }

  // Fisheye: a two-pass one-dimensional barrel remap in strips. Not a true
  // lens equation, but the centre magnifies and the edges compress the same
  // way, at a cost of ~100 strip draws instead of a per-pixel loop.
  const k = op.strength * 0.45;
  const strips = 48;
  const remap = (normalised: number) => normalised * (1 - k * (1 - normalised * normalised));

  scratch.width = width;
  scratch.height = height;
  scratchContext.drawImage(work, 0, 0);
  context.clearRect(0, 0, width, height);
  for (let index = 0; index < strips; index += 1) {
    const x0 = (index / strips) * 2 - 1;
    const x1 = ((index + 1) / strips) * 2 - 1;
    const sx0 = ((remap(x0) + 1) / 2) * width;
    const sx1 = ((remap(x1) + 1) / 2) * width;
    context.drawImage(
      scratch,
      sx0, 0, Math.max(1, sx1 - sx0), height,
      (index / strips) * width, 0, width / strips + 1, height,
    );
  }
  scratchContext.clearRect(0, 0, width, height);
  scratchContext.drawImage(work, 0, 0);
  context.clearRect(0, 0, width, height);
  for (let index = 0; index < strips; index += 1) {
    const y0 = (index / strips) * 2 - 1;
    const y1 = ((index + 1) / strips) * 2 - 1;
    const sy0 = ((remap(y0) + 1) / 2) * height;
    const sy1 = ((remap(y1) + 1) / 2) * height;
    context.drawImage(
      scratch,
      0, sy0, width, Math.max(1, sy1 - sy0),
      0, (index / strips) * height, width, height / strips + 1,
    );
  }
}

/**
 * The incoming clip of a cross-fade, playing its pre-roll over the picture.
 *
 * A second, short-lived video element with the same corrective sync the main
 * one uses. It exists only inside the dissolve window; the `key` on its mount
 * point retires it the moment the transition's clip changes.
 */
function GhostVideo({
  ghost,
  playing,
}: {
  ghost: { clipId: string; path: string; time: number; speed: number; opacity: number };
  playing: boolean;
}) {
  const element = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const media = element.current;
    if (!media) return;
    media.src = convertFileSrc(ghost.path);
    media.load();
    // Mount effect only: the sync effect below lands the position.
     
  }, [ghost.path]);

  useEffect(() => {
    const media = element.current;
    if (!media) return;
    const rate = Math.min(16, Math.max(0.0625, ghost.speed));
    if (media.playbackRate !== rate) media.playbackRate = rate;
    const tolerance = playing ? 0.3 : 0.03;
    if (media.readyState > 0 && Math.abs(media.currentTime - ghost.time) > tolerance) {
      try {
        media.currentTime = Math.max(0, ghost.time);
      } catch {
        // Still opening; the next update lands it.
      }
    }
    if (playing && media.paused) void media.play().catch(() => undefined);
    if (!playing && !media.paused) media.pause();
  }, [ghost, playing]);

  return (
    <video
      ref={element}
      muted
      playsInline
      className="pointer-events-none absolute inset-0 h-full w-full object-contain"
      style={{ opacity: Math.min(1, Math.max(0, ghost.opacity)) }}
    />
  );
}

/**
 * Direct manipulation of the displayed clip's picture.
 *
 * Drag the body to move, a corner to scale, the lollipop above the top edge to
 * rotate. The box is laid out exactly where the picture is - fitted size times
 * scale, centred plus offset, rotated - so grabbing the picture is grabbing
 * the picture, not a proxy for it.
 *
 * Pointer moves write absolute transform values through `onChange`; clamping
 * lives with the project model, not here.
 */
function TransformGizmo({
  clipId,
  transform,
  frameRect,
  mediaSize,
  selected,
  onSelect,
  onChange,
  onChangeEnd,
  onGuides,
}: {
  clipId: string;
  transform: PreviewTransform;
  frameRect: { left: number; top: number; width: number; height: number };
  mediaSize: { width: number; height: number } | null;
  selected: boolean;
  onSelect: (clipId: string) => void;
  onChange: (clipId: string, transform: Partial<PreviewTransform>) => void;
  /** The drag finished; whatever was echoed becomes one engine command. */
  onChangeEnd: () => void;
  onGuides: (guides: Guides) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const detach = useRef<(() => void) | null>(null);

  // A drag can outlive the gizmo - the playhead runs past the clip, say - and
  // window listeners do not clean themselves up.
  useEffect(() => () => detach.current?.(), []);

  // The picture's box: the fitted size the engine calls scale 1, times the
  // scale, centred plus the offset. Unknown media dimensions mean the fit
  // cannot be computed, so the frame itself stands in - same as `object-contain`
  // does for the element underneath.
  const fit = mediaSize
    ? Math.min(frameRect.width / mediaSize.width, frameRect.height / mediaSize.height)
    : null;
  const width = (fit && mediaSize ? mediaSize.width * fit : frameRect.width) * transform.scale;
  const height = (fit && mediaSize ? mediaSize.height * fit : frameRect.height) * transform.scale;
  const centerX = frameRect.width / 2 + transform.offsetX * frameRect.width;
  const centerY = frameRect.height / 2 + transform.offsetY * frameRect.height;

  const beginDrag = (mode: "move" | "scale" | "rotate") => (event: ReactPointerEvent) => {
    // Left button only; right-click should not start moving things around.
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    onSelect(clipId);
    detach.current?.();

    // Everything is captured at the pointer-down so every move is computed
    // from absolute positions rather than accumulated deltas - no drift, no
    // stale closures.
    const start = { ...transform };
    const startX = event.clientX;
    const startY = event.clientY;
    const bounds = box.current?.getBoundingClientRect();
    // The bounding rect of a rotated box is axis-aligned, but its centre is
    // still the true centre, which is all scale and rotate need.
    const centre = bounds
      ? { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 }
      : { x: startX, y: startY };
    const startDistance = Math.max(1, Math.hypot(startX - centre.x, startY - centre.y));
    const startAngle = Math.atan2(startY - centre.y, startX - centre.x);
    // Edge guides only mean something while the box is axis-aligned; a tilted
    // picture has no edge that can line up with the frame's.
    const quarter = Math.abs(start.rotation % 180);
    const extentX = quarter === 0 ? width : quarter === 90 ? height : null;
    const extentY = quarter === 0 ? height : quarter === 90 ? width : null;

    detach.current = startDrag(
      (pointer) => {
        if (mode === "move") {
          const x = softSnap(
            start.offsetX + (pointer.clientX - startX) / frameRect.width,
            frameRect.width,
            extentX,
          );
          const y = softSnap(
            start.offsetY + (pointer.clientY - startY) / frameRect.height,
            frameRect.height,
            extentY,
          );
          onChange(clipId, { offsetX: x.value, offsetY: y.value });
          onGuides({ x: x.guide === null ? [] : [x.guide], y: y.guide === null ? [] : [y.guide] });
        } else if (mode === "scale") {
          const distance = Math.hypot(pointer.clientX - centre.x, pointer.clientY - centre.y);
          const raw = start.scale * (distance / startDistance);
          const snapped = snapScale(Math.max(MIN_SCALE, Math.min(MAX_SCALE, raw)), {
            // Unscaled per-axis extents; the drag rescales them from here.
            fittedX: extentX === null ? null : extentX / start.scale,
            fittedY: extentY === null ? null : extentY / start.scale,
            centreX: frameRect.width / 2 + start.offsetX * frameRect.width,
            centreY: frameRect.height / 2 + start.offsetY * frameRect.height,
            frameWidth: frameRect.width,
            frameHeight: frameRect.height,
          });
          onChange(clipId, { scale: snapped.value });
          onGuides(snapped.guides);
        } else {
          const angle = Math.atan2(pointer.clientY - centre.y, pointer.clientX - centre.x);
          let rotation = start.rotation + ((angle - startAngle) * 180) / Math.PI;
          if (pointer.shiftKey) {
            // Shift is the deliberate grid; 15 is the step every editor uses.
            rotation = Math.round(rotation / 15) * 15;
          } else {
            // A soft catch at the right angles, because "exactly straight" is
            // what a free-hand rotation is trying to hit nine times in ten.
            const cardinal = Math.round(rotation / 90) * 90;
            if (Math.abs(rotation - cardinal) < 3) rotation = cardinal;
          }
          onChange(clipId, { rotation });
        }
      },
      () => {
        onGuides(NO_GUIDES);
        detach.current = null;
        onChangeEnd();
      },
    );
  };

  return (
    <div
      className="pointer-events-none absolute"
      style={{
        left: frameRect.left,
        top: frameRect.top,
        width: frameRect.width,
        height: frameRect.height,
      }}
    >
      <div
        ref={box}
        className="pointer-events-auto absolute cursor-move"
        style={{
          left: centerX,
          top: centerY,
          width,
          height,
          transform: `translate(-50%, -50%) rotate(${transform.rotation}deg)`,
          touchAction: "none",
        }}
        onPointerDown={beginDrag("move")}
      >
        {selected && (
          <>
            <div className="pointer-events-none absolute -inset-px border border-selection" />

            {/* The rotate lollipop: a stem out of the top edge, a knob to grab. */}
            <div
              className="pointer-events-none absolute left-1/2 h-6 w-px -translate-x-1/2 bg-selection"
              style={{ top: -24 }}
            />
            <div
              className="absolute left-1/2 h-3 w-3 -translate-x-1/2 -translate-y-1/2 cursor-grab
                         rounded-full border border-black/50 bg-white shadow-sm"
              style={{ top: -28, touchAction: "none" }}
              onPointerDown={beginDrag("rotate")}
            />

            {[
              { x: 0, y: 0, cursor: "nwse-resize" },
              { x: 1, y: 0, cursor: "nesw-resize" },
              { x: 0, y: 1, cursor: "nesw-resize" },
              { x: 1, y: 1, cursor: "nwse-resize" },
            ].map((corner) => (
              <div
                key={`${corner.x}-${corner.y}`}
                className={HANDLE_CLASS}
                style={{
                  left: `${corner.x * 100}%`,
                  top: `${corner.y * 100}%`,
                  cursor: corner.cursor,
                  touchAction: "none",
                }}
                onPointerDown={beginDrag("scale")}
              />
            ))}
          </>
        )}
      </div>
    </div>
  );
}

/** Bounds shared by TextPanel's Size slider; the corner drag obeys the same
 * law the slider does. */
const MIN_FONT_SIZE = 0.02;
const MAX_FONT_SIZE = 0.4;

/**
 * A title in the monitor: the same rendered block as before, now grabbable.
 *
 * The box is the text block itself rather than computed geometry - the border
 * hugs whatever the words actually occupy, padding and plate included, so
 * there is nothing to get out of sync. Dragging moves the offsets; the corner
 * handles resize the type through the same fraction-of-frame-height law the
 * Text panel's Size slider uses. No rotate handle: nothing downstream can
 * draw a rotated title yet, and a handle that lies is worse than none.
 */
function TextOverlayBox({
  overlay,
  frameRect,
  selected,
  onSelect,
  onChange,
  onChangeEnd,
  onGuides,
}: {
  overlay: TextOverlay;
  frameRect: { left: number; top: number; width: number; height: number };
  selected: boolean;
  onSelect: (clipId: string) => void;
  onChange: (clipId: string, change: { offsetX?: number; offsetY?: number; fontSize?: number }) => void;
  /** The drag finished; whatever was echoed becomes one engine command. */
  onChangeEnd: () => void;
  onGuides: (guides: Guides) => void;
}) {
  const block = useRef<HTMLDivElement>(null);
  const detach = useRef<(() => void) | null>(null);
  useEffect(() => () => detach.current?.(), []);

  const begin = (mode: "move" | "scale") => (event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    onSelect(overlay.clipId);
    detach.current?.();

    const start = {
      offsetX: overlay.offsetX,
      offsetY: overlay.offsetY,
      fontSize: overlay.style.fontSize,
    };
    const startX = event.clientX;
    const startY = event.clientY;
    const bounds = block.current?.getBoundingClientRect();
    const centre = bounds
      ? { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 }
      : { x: startX, y: startY };
    const startDistance = Math.max(1, Math.hypot(startX - centre.x, startY - centre.y));

    detach.current = startDrag(
      (pointer) => {
        if (mode === "move") {
          // Centre guides only: a block of type has ragged edges, and lining
          // those up with the frame edge is not a thing anyone means to do.
          const x = softSnap(
            start.offsetX + (pointer.clientX - startX) / frameRect.width,
            frameRect.width,
            null,
          );
          const y = softSnap(
            start.offsetY + (pointer.clientY - startY) / frameRect.height,
            frameRect.height,
            null,
          );
          onChange(overlay.clipId, { offsetX: x.value, offsetY: y.value });
          onGuides({ x: x.guide === null ? [] : [x.guide], y: y.guide === null ? [] : [y.guide] });
        } else {
          const distance = Math.hypot(pointer.clientX - centre.x, pointer.clientY - centre.y);
          const fontSize = start.fontSize * (distance / startDistance);
          onChange(overlay.clipId, {
            fontSize: Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, fontSize)),
          });
        }
      },
      () => {
        onGuides(NO_GUIDES);
        detach.current = null;
        onChangeEnd();
      },
    );
  };

  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
      <div
        ref={block}
        className="pointer-events-auto relative cursor-move"
        style={{
          ...textCss(overlay.style, frameRect.height),
          // Percentages here would resolve against the text block's own
          // width, which varies with the words. The frame is the thing
          // offsets are relative to, so they are converted against its
          // measured box instead.
          transform: `translate(${overlay.offsetX * frameRect.width}px, ${
            overlay.offsetY * frameRect.height
          }px)`,
          maxWidth: "92%",
          touchAction: "none",
        }}
        onPointerDown={begin("move")}
      >
        {overlay.style.content}
        {selected && (
          <>
            <div className="pointer-events-none absolute -inset-1 border border-selection" />
            {[
              { x: 0, y: 0, cursor: "nwse-resize" },
              { x: 1, y: 0, cursor: "nesw-resize" },
              { x: 0, y: 1, cursor: "nesw-resize" },
              { x: 1, y: 1, cursor: "nwse-resize" },
            ].map((corner) => (
              <div
                key={`${corner.x}-${corner.y}`}
                className={HANDLE_CLASS}
                style={{
                  left: `${corner.x * 100}%`,
                  top: `${corner.y * 100}%`,
                  cursor: corner.cursor,
                  touchAction: "none",
                }}
                onPointerDown={begin("scale")}
              />
            ))}
          </>
        )}
      </div>
    </div>
  );
}
