// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useState } from "react";

import { useLocale, type MsgKey } from "../lib/i18n";
import { Icon } from "./Icon";

/** Output bounds the encoder accepts comfortably; also the preview footer's. */
const MIN_DIMENSION = 16;
const MAX_DIMENSION = 8192;

/**
 * Edits the project's own details: its name and output frame.
 *
 * Distinct from Settings on purpose - Settings configures the app, this
 * configures the project. The frame rate is shown but not editable: every
 * clip boundary in the edit is quantised to its grid, and changing it would
 * silently re-time the whole timeline. That is a conform, not a config.
 */
export function ModifyProjectDialog({
  name,
  frame,
  frameRate,
  busy,
  onSave,
  onCancel,
}: {
  name: string;
  frame: { width: number; height: number };
  frameRate: number;
  busy: boolean;
  onSave: (next: { name: string; width: number; height: number }) => void;
  onCancel: () => void;
}) {
  const [draftName, setDraftName] = useState(name);
  const [width, setWidth] = useState(String(frame.width));
  const [height, setHeight] = useState(String(frame.height));
  const { t } = useLocale();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  // Even dimensions inside the encoder's comfort range; the same rounding
  // the preview footer applies, so the two entrances agree.
  const parse = (value: string) => {
    const number = Math.round(Number(value));
    if (!Number.isFinite(number)) return null;
    const clamped = Math.min(MAX_DIMENSION, Math.max(MIN_DIMENSION, number));
    return clamped - (clamped % 2);
  };
  const parsedWidth = parse(width);
  const parsedHeight = parse(height);
  const valid = draftName.trim().length > 0 && parsedWidth !== null && parsedHeight !== null;

  const save = () => {
    if (!valid || busy) return;
    onSave({ name: draftName.trim(), width: parsedWidth!, height: parsedHeight! });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8
                 backdrop-blur-[2px]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div className="surface w-full max-w-sm rounded-2xl p-6">
        <div className="mb-4 flex items-center gap-2">
          <Icon name="settings" size={16} className="text-accent" />
          <h2 className="text-sm font-semibold text-primary">{t("modifyProject.title")}</h2>
        </div>

        <label className="mb-3 block">
          <span className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-tertiary">
            {t("modifyProject.name")}
          </span>
          <input
            value={draftName}
            autoFocus
            spellCheck={false}
            onChange={(event) => setDraftName(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && save()}
            className="w-full rounded-lg bg-sunken px-3 py-2 text-sm text-primary outline-none
                       ring-1 ring-hairline focus:ring-accent"
          />
        </label>

        <div className="mb-3 grid grid-cols-2 gap-3">
          {(
            [
              ["modifyProject.width", width, setWidth],
              ["modifyProject.height", height, setHeight],
            ] as [MsgKey, string, (value: string) => void][]
          ).map(([labelKey, value, set]) => (
            <label key={labelKey} className="block">
              <span className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-tertiary">
                {t(labelKey)}
              </span>
              <input
                value={value}
                inputMode="numeric"
                spellCheck={false}
                onChange={(event) => set(event.target.value)}
                onKeyDown={(event) => event.key === "Enter" && save()}
                className="w-full rounded-lg bg-sunken px-3 py-2 font-technical text-sm
                           text-primary outline-none ring-1 ring-hairline focus:ring-accent"
              />
            </label>
          ))}
        </div>

        <p className="mb-5 flex items-baseline justify-between text-xs text-secondary">
          <span>{t("modifyProject.frameRate")}</span>
          <span className="font-technical text-primary" title={t("modifyProject.frameRateHint")}>
            {t("modifyProject.frameRateValue", { rate: frameRate.toFixed(2) })}
          </span>
        </p>

        <div className="flex gap-1.5">
          <button
            type="button"
            onClick={onCancel}
            className="flex-1 cursor-pointer rounded-lg bg-hover px-4 py-2 text-sm text-primary
                       transition-colors hover:bg-active"
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            disabled={busy || !valid}
            onClick={save}
            className="flex-1 cursor-pointer rounded-lg bg-accent px-4 py-2 text-sm font-medium
                       text-on-accent transition-colors hover:bg-accent-hover
                       disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? t("modifyProject.saving") : t("modifyProject.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
