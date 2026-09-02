// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useSyncExternalStore } from "react";

import en from "../locales/en.json";
import zhCN from "../locales/zh-CN.json";

/**
 * The UI language.
 *
 * Every string a person reads in the app comes out of a per-locale JSON
 * catalog in `src/locales/`, looked up by a typed key - a typo'd key is a
 * compile error, and `i18n.test.ts` holds every catalog to exactly the key
 * set en.json defines. Adding a language is copying en.json, translating the
 * values, and registering one entry in LOCALES and CATALOGS below; see
 * TRANSLATING.md at the repo root.
 *
 * Unlike the theme, the language *does* follow the OS by default. The theme
 * refuses to (see theme.ts) because a surround that changes on its own is
 * disorienting; language is the opposite case - an interface you cannot read
 * is not a neutral default, so first launch speaks whatever the system
 * speaks, and the explicit choice in Settings overrides it from then on.
 *
 * Switching re-renders by subscription, not remount: App owns the live
 * editing session, so rebuilding the tree under a changed `key` would dump
 * the user out of their project. Components that render text call
 * `useLocale()` and take `t` from it - the snapshot (and `t`'s identity)
 * changes per switch, so `useSyncExternalStore` re-renders subscribers
 * directly (through `React.memo`), and exhaustive-deps forces `t` into any
 * dependency array that caches a translated string. The module-level `t`/`tp`
 * exports are for non-React code and event handlers, which always run after
 * the switch and therefore always see the current language.
 */
export type MsgKey = keyof typeof en;

/** Bases of plural sets: `x.one`/`x.other` in the catalog collapse to `x`. */
// Distributed through a type parameter - a conditional over the union alias
// itself would not distribute and would collapse to never.
type PluralBase<Key> = Key extends `${infer Base}.one` ? Base : never;
export type PluralKey = PluralBase<MsgKey>;

/**
 * Native names, never translated: whoever landed in a language they cannot
 * read must still be able to find their own in the picker.
 */
export const LOCALES = [
  { id: "en", name: "English" },
  { id: "zh-CN", name: "简体中文" },
] as const;

export type LocaleId = (typeof LOCALES)[number]["id"];

// Record<MsgKey, string> makes a catalog missing keys fail `tsc`, before the
// friendlier i18n.test.ts even runs.
const CATALOGS: Record<LocaleId, Record<MsgKey, string>> = { en, "zh-CN": zhCN };

const STORAGE_KEY = "wolfcut.locale";

/** The stored preference: an explicit locale, or absence meaning "system". */
function stored(): LocaleId | "system" {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    const known = LOCALES.find((entry) => entry.id === value);
    return known ? known.id : "system";
  } catch {
    // Private mode, blocked storage, node - the system default anyway.
    return "system";
  }
}

function persist(next: LocaleId | "system"): void {
  try {
    if (next === "system") localStorage.removeItem(STORAGE_KEY);
    else localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Not being able to remember the choice is not worth failing over.
  }
}

/** "system" resolved against what we actually ship: exact id, then language prefix, then en. */
function resolve(next: LocaleId | "system"): LocaleId {
  if (next !== "system") return next;
  const wanted = typeof navigator !== "undefined" ? navigator.language : "";
  if (!wanted) return "en";
  const lower = wanted.toLowerCase();
  for (const entry of LOCALES) if (entry.id.toLowerCase() === lower) return entry.id;
  const prefix = lower.split("-")[0];
  for (const entry of LOCALES) {
    if (entry.id.split("-")[0].toLowerCase() === prefix) return entry.id;
  }
  return "en";
}

/** What "System default" would mean right now, for labelling the picker. */
export function systemLocale(): LocaleId {
  return resolve("system");
}

/**
 * Puts the language on `<html lang>`, where the OS spellchecker, screen
 * readers and CSS quotes rules key off it. index.html carries lang="en" only
 * as the pre-hydration default.
 */
function applyLocale(next: LocaleId): void {
  if (typeof document !== "undefined") document.documentElement.lang = next;
}

let preference: LocaleId | "system" = stored();
let locale: LocaleId = resolve(preference);

function interpolate(message: string, params?: Record<string, string | number>): string {
  if (!params) return message;
  // An unfilled placeholder stays visible as {name} - an on-screen bug is
  // louder and more honest than a silently swallowed one.
  return message.replace(/\{(\w+)\}/g, (whole, name: string) =>
    params[name] != null ? String(params[name]) : whole,
  );
}

/** The message for `key` in the current language, English if the catalog has a hole. */
export function t(key: MsgKey, params?: Record<string, string | number>): string {
  const message = CATALOGS[locale][key] ?? CATALOGS.en[key];
  return interpolate(message, params);
}

/**
 * The plural form of `key` for `count`, per the language's own rules
 * (Intl.PluralRules - English says "one"/"other", Chinese only "other").
 * `{count}` is available as a placeholder without passing it again.
 */
export function tp(key: PluralKey, count: number, params?: Record<string, string | number>): string {
  const category = new Intl.PluralRules(locale).select(count);
  const exact = `${key}.${category}` as MsgKey;
  const other = `${key}.other` as MsgKey;
  const message = CATALOGS[locale][exact] ?? CATALOGS[locale][other] ?? CATALOGS.en[other];
  return interpolate(message, { count, ...params });
}

export function getLocale(): LocaleId {
  return locale;
}

export type LocaleSnapshot = {
  locale: LocaleId;
  preference: LocaleId | "system";
  setLocale: (next: LocaleId | "system") => void;
  t: typeof t;
  tp: typeof tp;
};

// The snapshot is rebuilt (new identities throughout) on every switch, so a
// `t` taken from useLocale() is a reactive value the linter refuses to let
// out of dependency arrays - stale memoized strings become lint errors.
function buildSnapshot(): LocaleSnapshot {
  return {
    locale,
    preference,
    setLocale,
    t: (key, params) => t(key, params),
    tp: (key, count, params) => tp(key, count, params),
  };
}

const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function setLocale(next: LocaleId | "system"): void {
  persist(next);
  preference = next;
  locale = resolve(next);
  applyLocale(locale);
  snapshot = buildSnapshot();
  for (const listener of listeners) listener();
}

let snapshot = buildSnapshot();

// At import time, before React renders anything, so the first paint is
// already in the right language - same move as theme.ts.
applyLocale(locale);

/** The current language for React: subscribes the component to switches. */
export function useLocale(): LocaleSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}
