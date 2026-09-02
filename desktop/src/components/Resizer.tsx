// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useRef } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/**
 * A draggable divider between two docked panels.
 *
 * The hit area is deliberately wider than the visible line (8px versus 1px):
 * a 1px grab target is the single most common way a docked layout feels bad
 * to use. The line itself only lights up once you are on it.
 */
export function Resizer({
  direction,
  onResize,
  className = "",
}: {
  /** `vertical` = a vertical bar you drag left/right to size a side panel. */
  direction: "vertical" | "horizontal";
  /** Receives the pointer delta in pixels since the last event. */
  onResize: (delta: number) => void;
  className?: string;
}) {
  const last = useRef(0);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    last.current = direction === "vertical" ? event.clientX : event.clientY;
  }, [direction]);

  const onPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
      const position = direction === "vertical" ? event.clientX : event.clientY;
      onResize(position - last.current);
      last.current = position;
    },
    [direction, onResize],
  );

  const vertical = direction === "vertical";

  return (
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      // The resizer *is* the gutter between two panels, which is why it has a
      // real size and no background: whatever this is set to becomes the gap,
      // on both axes, with nothing else to keep in step.
      className={`group relative shrink-0 ${
        vertical ? "w-2 cursor-col-resize" : "h-2 cursor-row-resize"
      } ${className}`}
    >
      {/* Nothing is drawn until you are on it. The panels either side already
          have their own edges; a third line between them is just noise. Lime,
          like the active tools and the selection - grabbing a divider is an
          action, and actions read lime everywhere else in the app. */}
      <span
        aria-hidden
        className={`absolute rounded-full bg-tool-active opacity-0 transition-opacity
                    group-hover:opacity-100 group-active:opacity-100 ${
                      vertical
                        ? "inset-y-2 left-1/2 w-0.5 -translate-x-1/2"
                        : "inset-x-2 top-1/2 h-0.5 -translate-y-1/2"
                    }`}
      />
    </div>
  );
}
