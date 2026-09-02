// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useState } from "react";

import { useLocale } from "../lib/i18n";
import { Icon } from "./Icon";

/**
 * Names and saves the open project as a template.
 *
 * The one decision it asks for is the name; the substance - which media are
 * slots - was decided in the bin, so the dialog only reports it. Zero slots
 * is allowed (a starter project is a legitimate template) but flagged, since
 * forgetting to mark slots is the likelier reading, and everything that is
 * not a slot ships inside the bundle by copy.
 */
export function SaveTemplateDialog({
  defaultName,
  slotCount,
  busy,
  onSave,
  onCancel,
}: {
  defaultName: string;
  /** How many media items are marked as slots right now. */
  slotCount: number;
  busy: boolean;
  onSave: (name: string) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(defaultName);
  const { t, tp } = useLocale();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const save = () => {
    const trimmed = name.trim();
    if (trimmed && !busy) onSave(trimmed);
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
        <div className="mb-2 flex items-center gap-2">
          <Icon name="slot" size={16} className="text-accent" />
          <h2 className="text-sm font-semibold text-primary">{t("saveTemplate.title")}</h2>
        </div>
        <p className="mb-4 text-xs leading-relaxed text-secondary">
          {slotCount > 0 ? tp("saveTemplate.slots", slotCount) : t("saveTemplate.noSlots")}
        </p>

        <input
          value={name}
          autoFocus
          spellCheck={false}
          placeholder={t("saveTemplate.namePlaceholder")}
          onChange={(event) => setName(event.target.value)}
          onFocus={(event) => event.currentTarget.select()}
          onKeyDown={(event) => event.key === "Enter" && save()}
          className="mb-5 w-full rounded-lg bg-sunken px-3 py-2 text-sm text-primary outline-none
                     ring-1 ring-hairline focus:ring-accent placeholder:text-tertiary"
        />

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
            disabled={busy || !name.trim()}
            onClick={save}
            className="flex-1 cursor-pointer rounded-lg bg-accent px-4 py-2 text-sm font-medium
                       text-on-accent transition-colors hover:bg-accent-hover
                       disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? t("saveTemplate.saving") : t("saveTemplate.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
