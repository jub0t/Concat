// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import type { ReactNode } from "react";

/**
 * Shared chrome primitives.
 *
 * Panels are separate rounded surfaces with an even gutter between them, not
 * flush regions divided by hairlines. The gap is produced by the resizers
 * themselves rather than by margins or a flex `gap`, so it is the same on both
 * axes by construction and cannot drift as the layout changes.
 *
 * `PANEL_SHELL` is the single definition of what a panel looks like. The
 * preview and the timeline are not built from `Panel` - they own their whole
 * interior - so they reuse this constant instead of copying the classes.
 */
// No outline border: the panels separate from the stage by surface colour
// and the resizer gutters alone, and only *interior* dividers draw lines.
// An outline on every panel doubled each gutter's line weight for nothing.
export const PANEL_SHELL =
  "flex h-full min-h-0 flex-col overflow-hidden rounded-xl bg-panel";

/** A docked panel with a heading strip and a scrolling body. */
export function Panel({
  title,
  actions,
  children,
  className = "",
}: {
  title?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`${PANEL_SHELL} ${className}`}>
      {title && (
        <header className="flex h-9 shrink-0 items-center gap-2 border-b border-hairline px-3">
          <h2 className="text-[11px] font-semibold uppercase tracking-wider text-secondary">
            {title}
          </h2>
          <div className="ml-auto flex items-center gap-0.5">{actions}</div>
        </header>
      )}
      <div className="thin-scroll min-h-0 flex-1 overflow-y-auto">{children}</div>
    </section>
  );
}

/** A horizontal group of controls, e.g. the timeline toolbar. */
export function Bar({ children }: { children: ReactNode }) {
  return (
    // Taller than its buttons on purpose: a 36px button in a 44px bar gets
    // 4px of air at top and bottom instead of pressing against the edges.
    <div className="flex h-11 shrink-0 items-center gap-1 border-b border-hairline bg-panel px-2">
      {children}
    </div>
  );
}

/** The hairline separating groups inside a `Bar`. */
export function Divider() {
  return <span className="mx-1 h-5 w-px shrink-0 bg-active" aria-hidden />;
}

/** Pushes everything after it to the far end of a `Bar`. */
export function Spacer() {
  return <span className="flex-1" aria-hidden />;
}

/** Centred placeholder for a panel with nothing in it yet. */
export function Empty({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-8 text-center">
      {icon && <span className="text-tertiary">{icon}</span>}
      <p className="text-xs leading-relaxed text-tertiary">{children}</p>
    </div>
  );
}
