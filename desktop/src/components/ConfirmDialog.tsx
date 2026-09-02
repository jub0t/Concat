// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect } from "react";

import { Icon } from "./Icon";

/**
 * A small blocking confirmation, for actions that destroy work.
 *
 * Edits are undoable through the engine's history; this sheet guards the
 * destructions that are not, or that erase enough at once that a stray click
 * should not get to do it. Kept deliberately plain: a title that names the
 * thing, one sentence of consequence, and a danger-coloured button that
 * repeats the verb - never just "OK", because the button is what people read
 * when they skip the sentence.
 */
export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
  /** The verb, e.g. "Delete timeline". Shown on the danger button. */
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  // Escape cancels, like every sheet. Enter deliberately does not confirm:
  // a destructive default keyed to the most reflexive key would delete work.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

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
          <Icon name="trash" size={16} className="text-danger" />
          <h2 className="text-sm font-semibold text-primary">{title}</h2>
        </div>
        <p className="mb-5 text-xs leading-relaxed text-secondary">{message}</p>
        <div className="flex gap-1.5">
          <button
            type="button"
            autoFocus
            onClick={onCancel}
            className="flex-1 cursor-pointer rounded-lg bg-hover px-4 py-2 text-sm text-primary
                       transition-colors hover:bg-active"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="flex-1 cursor-pointer rounded-lg bg-danger px-4 py-2 text-sm font-medium
                       text-white transition-colors hover:opacity-90"
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
