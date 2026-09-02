// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useState } from "react";

import { Icon } from "./Icon";

/**
 * The bottom-right notice.
 *
 * Rises in, holds, sinks out - plain CSS transitions, no animation library.
 * Errors hold long enough to actually be read; confirmations only need a
 * glance. Until this existed the toast just appeared and then sat there
 * forever, which made every error look current long after it was history.
 */
const HOLD_FAILED_MS = 6500;
const HOLD_OK_MS = 3500;
/** Must match the leaving duration-[] class below. */
const LEAVE_MS = 420;

export function Toast({
  toast,
  onDone,
}: {
  toast: { id: number; message: string; failed: boolean };
  onDone: (id: number) => void;
}) {
  // "enter" is the single pre-paint frame at the risen toast's start
  // position; the browser has to paint it before the move to "shown" can
  // animate rather than teleport - hence the double rAF.
  const [phase, setPhase] = useState<"enter" | "shown" | "leaving">("enter");

  useEffect(() => {
    const raise = requestAnimationFrame(() =>
      requestAnimationFrame(() => setPhase("shown")),
    );
    const hold = window.setTimeout(
      () => setPhase("leaving"),
      toast.failed ? HOLD_FAILED_MS : HOLD_OK_MS,
    );
    return () => {
      cancelAnimationFrame(raise);
      clearTimeout(hold);
    };
  }, [toast.id, toast.failed]);

  useEffect(() => {
    if (phase !== "leaving") return;
    const gone = window.setTimeout(() => onDone(toast.id), LEAVE_MS);
    return () => clearTimeout(gone);
  }, [phase, toast.id, onDone]);

  return (
    <div
      role="status"
      className={`surface fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-xl
                  px-3 py-2 text-xs transition-all ease-out
                  ${toast.failed ? "text-danger" : "text-primary"}
                  ${
                    phase === "shown"
                      ? "translate-y-0 opacity-100 duration-[260ms]"
                      : "translate-y-3 opacity-0 duration-[420ms]"
                  }`}
    >
      <Icon
        name={toast.failed ? "close" : "check"}
        size={13}
        className={toast.failed ? "" : "text-success"}
      />
      {toast.message}
    </div>
  );
}
