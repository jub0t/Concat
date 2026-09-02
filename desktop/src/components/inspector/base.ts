// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/**
 * The plumbing under the inspector controls: class joining, number hygiene,
 * and the one drag gesture every scrubbing control in the panel shares.
 *
 * Ported from the wc-ui-rnd design study alongside `inspector.module.css`.
 * Nothing here knows about clips or the engine - these are the parts a Fader
 * and a NumberField would need in any application.
 */

export const cx = (...parts: Array<string | false | null | undefined>): string =>
  parts.filter(Boolean).join(" ");

export const clamp = (value: number, min: number, max: number): number =>
  Math.min(max, Math.max(min, value));

export const roundTo = (value: number, decimals: number): number =>
  Number(value.toFixed(clamp(decimals, 0, 12)));

/** Decimals present in a literal step, so `step={0.005}` implies 3 digits. */
export const decimalsOf = (step: number): number => {
  const [, fraction = ""] = String(step).split(".");
  return fraction.length;
};

export interface DragInfo {
  /** Pointer travel since pointerdown, in px. */
  dx: number;
  dy: number;
  /** Current viewport position, in px. */
  x: number;
  y: number;
  shiftKey: boolean;
  altKey: boolean;
  /** Flips true once the pointer passes `threshold` - tells a click from a drag. */
  moved: boolean;
}

export interface PointerDragOptions {
  onStart?: (info: DragInfo) => void;
  onMove?: (info: DragInfo) => void;
  onEnd?: (info: DragInfo) => void;
  /** Px of travel before the gesture counts as a drag. Default 3. */
  threshold?: number;
  /** Cursor forced on the document while the gesture is live. */
  cursor?: string;
  disabled?: boolean;
}

/**
 * The drag gesture shared by every scrubbing control.
 *
 * Pointer capture keeps the stream alive when the pointer leaves the element
 * or the window, which is what lets a fader keep tracking once you have run
 * off the end of it. Returns the `onPointerDown` handler for the drag surface.
 */
export function usePointerDrag(options: PointerDragOptions) {
  // Latest-ref, so the handler installed on pointerdown always calls the
  // current callbacks rather than the ones captured when the drag began.
  const latest = useRef(options);
  latest.current = options;

  const teardown = useRef<(() => void) | null>(null);
  useEffect(() => () => teardown.current?.(), []);

  return useCallback((event: ReactPointerEvent<Element>) => {
    const { disabled, threshold = 3, cursor } = latest.current;
    if (disabled || event.button !== 0) return;

    // React reuses the event object's currentTarget after the handler returns.
    const target = event.currentTarget;
    const { pointerId } = event;
    const startX = event.clientX;
    const startY = event.clientY;
    let moved = false;

    teardown.current?.();
    target.setPointerCapture(pointerId);

    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    if (cursor) document.body.style.cursor = cursor;
    document.body.style.userSelect = "none";

    const info = (moveEvent: PointerEvent): DragInfo => ({
      dx: moveEvent.clientX - startX,
      dy: moveEvent.clientY - startY,
      x: moveEvent.clientX,
      y: moveEvent.clientY,
      shiftKey: moveEvent.shiftKey,
      altKey: moveEvent.altKey,
      moved,
    });

    const handleMove = (moveEvent: PointerEvent) => {
      if (!moved && Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) > threshold) {
        moved = true;
      }
      latest.current.onMove?.(info(moveEvent));
    };

    const handleEnd = (endEvent: PointerEvent) => {
      const final = info(endEvent);
      teardown.current?.();
      latest.current.onEnd?.(final);
    };

    teardown.current = () => {
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", handleEnd);
      window.removeEventListener("pointercancel", handleEnd);
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
      if (target.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
      teardown.current = null;
    };

    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", handleEnd);
    window.addEventListener("pointercancel", handleEnd);

    latest.current.onStart?.({
      dx: 0,
      dy: 0,
      x: startX,
      y: startY,
      shiftKey: event.shiftKey,
      altKey: event.altKey,
      moved: false,
    });
  }, []);
}

/** Measures an element's content box, for geometry that has to match the DOM. */
export function useElementSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setSize({ width: rect.width, height: rect.height });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  return [ref, size] as const;
}
