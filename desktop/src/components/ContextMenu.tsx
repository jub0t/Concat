// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Icon, type IconName } from "./Icon";

export interface ContextItem {
  label: string;
  onSelect: () => void;
  icon?: IconName;
  hint?: string;
  danger?: boolean;
}

export interface ContextTarget {
  x: number;
  y: number;
  /** Related actions grouped together; a hairline divides the groups. */
  groups: ContextItem[][];
}

/**
 * A right-click menu, rendered through a portal so it is never clipped by a
 * panel's `overflow: hidden`.
 *
 * Opened near the bottom or right edge it would spill off-screen, so its
 * position is corrected once - after layout, before paint - from its real
 * measured size.
 */
export function ContextMenu({ target, onClose }: { target: ContextTarget; onClose: () => void }) {
  const root = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ x: target.x, y: target.y });

  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    setPosition({
      x: Math.max(8, Math.min(target.x, window.innerWidth - width - 8)),
      y: Math.max(8, Math.min(target.y, window.innerHeight - height - 8)),
    });
  }, [target.x, target.y]);

  useEffect(() => {
    // A portal renders outside the React tree, so propagation cannot tell us
    // about an outside click. Check containment against the real node.
    const onPointerDown = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={root}
      role="menu"
      style={{ left: position.x, top: position.y }}
      onContextMenu={(event) => event.preventDefault()}
      className="surface fixed z-50 min-w-48 rounded-xl px-1.5 py-1.5 text-primary"
    >
      {target.groups
        .filter((group) => group.length > 0)
        .map((group, index) => (
          // Index as key is fine here: the menu is rebuilt from scratch on
          // every open and never reorders while visible.
          <div key={index} className={index > 0 ? "mt-1 border-t border-hairline pt-1" : ""}>
            {group.map((item) => (
              <button
                key={item.label}
                role="menuitem"
                type="button"
                onClick={() => {
                  item.onSelect();
                  onClose();
                }}
                className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-1.5
                            text-left text-xs transition-colors hover:bg-hover
                            ${item.danger ? "text-danger" : ""}`}
              >
                {item.icon ? (
                  <Icon name={item.icon} size={14} className="shrink-0 opacity-70" />
                ) : (
                  <span className="w-3.5 shrink-0" />
                )}
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                {item.hint && (
                  <span className="shrink-0 font-technical text-[10px] text-tertiary">
                    {item.hint}
                  </span>
                )}
              </button>
            ))}
          </div>
        ))}
    </div>,
    document.body,
  );
}
