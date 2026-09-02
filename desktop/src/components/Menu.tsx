// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { Fragment, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { Icon, type IconName } from "./Icon";

export interface MenuOption {
  label: string;
  onSelect: () => void;
  icon?: IconName;
  /**
   * An arbitrary element in the icon slot, for rows whose glyph is drawn
   * rather than named - the frame-size menu's aspect shapes. Wins over
   * `icon` and `checked` when set.
   */
  leading?: ReactNode;
  /** Right-aligned hint, e.g. a keyboard shortcut. */
  hint?: string;
  disabled?: boolean;
  danger?: boolean;
  /**
   * Renders as a checkable row when set.
   *
   * A checkable row does not close the menu, because the whole point of one is
   * turning several things on and off in a row.
   */
  checked?: boolean;
}

/**
 * A dropdown menu hung under an arbitrary trigger.
 *
 * Options arrive in groups and a hairline is drawn between neighbouring
 * groups, so callers express structure by nesting arrays rather than by
 * threading separator sentinels through a flat list.
 */
export function Menu({
  groups,
  trigger,
  align = "left",
  direction = "down",
}: {
  groups: MenuOption[][];
  trigger: (open: boolean) => ReactNode;
  align?: "left" | "right";
  /** "up" hangs the menu above the trigger, for triggers near the bottom edge. */
  direction?: "down" | "up";
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const onPointerDown = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("blur", () => setOpen(false));
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className="cursor-pointer"
      >
        {trigger(open)}
      </button>

      {open && (
        <div
          role="menu"
          className={`surface absolute z-50 min-w-52 rounded-xl px-1.5 py-1.5 text-primary
                      ${direction === "up" ? "bottom-full mb-1.5" : "mt-1.5"}
                      ${align === "right" ? "right-0" : "left-0"}`}
        >
          {groups.map((group, groupIndex) => (
            <Fragment key={groupIndex}>
              {groupIndex > 0 && (
                <div role="separator" className="my-1.5 border-t border-hairline" />
              )}
              {group.map((option) => (
                <button
                  key={option.label}
                  role="menuitem"
                  type="button"
                  disabled={option.disabled}
                  onClick={() => {
                    option.onSelect();
                    if (option.checked === undefined) setOpen(false);
                  }}
                  className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-1.5
                              text-left text-xs transition-colors hover:bg-hover
                              disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent
                              ${option.danger ? "text-danger" : ""}`}
                >
                  {option.leading ? (
                    <span className="flex w-4 shrink-0 items-center justify-center">
                      {option.leading}
                    </span>
                  ) : option.checked !== undefined ? (
                    <Icon
                      name="check"
                      size={14}
                      className={`shrink-0 ${option.checked ? "text-accent" : "opacity-0"}`}
                    />
                  ) : option.icon ? (
                    <Icon name={option.icon} size={14} className="shrink-0 opacity-70" />
                  ) : (
                    <span className="w-3.5 shrink-0" />
                  )}
                  <span className="min-w-0 flex-1 truncate">{option.label}</span>
                  {option.hint && (
                    <span className="shrink-0 font-technical text-[10px] text-tertiary">
                      {option.hint}
                    </span>
                  )}
                </button>
              ))}
            </Fragment>
          ))}
        </div>
      )}
    </div>
  );
}
