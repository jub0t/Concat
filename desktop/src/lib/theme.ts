// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useCallback, useLayoutEffect, useState } from "react";

/**
 * Light or dark, chosen explicitly.
 *
 * WolfCut defaults to light whatever the OS is set to. A creative tool that
 * inverts itself because of a system setting the user was not thinking about
 * is disorienting, and judging an image against a surround that changed on its
 * own is worse than disorienting.
 *
 * The choice is stored per machine. Nothing about it reaches the project file:
 * it is a property of the person sitting there, not of the edit.
 */
export type Theme = "light" | "dark";

const STORAGE_KEY = "wolfcut.theme";

function stored(): Theme {
  try {
    return localStorage.getItem(STORAGE_KEY) === "dark" ? "dark" : "light";
  } catch {
    // Private mode, blocked storage - light is the default anyway.
    return "light";
  }
}

/**
 * Puts the theme on `<html>`, where the stylesheet keys off it.
 *
 * Called imperatively the moment the theme changes rather than from an effect,
 * and this matters more than it looks. React runs child effects before parent
 * effects, so anything that reads computed styles in its own effect - the
 * timeline, which copies the palette out for its canvas - would run *before* a
 * parent effect had applied the attribute, and read the outgoing theme. The
 * symptom is a timeline that is always exactly one theme behind, so switching
 * to dark paints it light.
 */
function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

function persist(theme: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Not being able to remember the choice is not worth failing over.
  }
}

// At import time, before React renders anything, so the first paint and the
// first canvas palette read already agree with the stored choice.
applyTheme(stored());

export function useTheme(): { theme: Theme; setTheme: (theme: Theme) => void; toggle: () => void } {
  const [theme, setThemeState] = useState<Theme>(stored);

  // A backstop only: the attribute is normally already correct by now.
  useLayoutEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const setTheme = useCallback((next: Theme) => {
    applyTheme(next);
    persist(next);
    setThemeState(next);
  }, []);

  const toggle = useCallback(() => {
    setThemeState((current) => {
      const next = current === "dark" ? "light" : "dark";
      applyTheme(next);
      persist(next);
      return next;
    });
  }, []);

  return { theme, setTheme, toggle };
}

/**
 * Reads a theme colour as a concrete value.
 *
 * Canvas cannot use `var(--color-x)`; it needs a real colour string. Call this
 * when the theme changes, not per frame - `getComputedStyle` forces a style
 * recalculation every time it is asked.
 */
export function themeColor(name: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(`--color-${name}`);
  return value.trim() || fallback;
}
