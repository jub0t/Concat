// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useRef, useState } from "react";

import { useLocale } from "../lib/i18n";
import { Icon } from "./Icon";

/**
 * The standard error strip.
 *
 * The text is selectable and there is always a copy button, because the main
 * thing anyone does with an error message is put it somewhere else - a bug
 * report, a search box, a chat. The app disables selection globally, so a
 * plain <p> here would be a message you can read but not quote.
 */
export function ErrorNotice({
  message,
  onDismiss,
  className = "",
}: {
  message: string;
  /** Rendered as a close button when given; omit for errors that clear themselves. */
  onDismiss?: () => void;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const { t } = useLocale();
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(message);
    } catch {
      // WebKit can refuse the async API; the selection route still works.
      const node = document.createElement("textarea");
      node.value = message;
      document.body.appendChild(node);
      node.select();
      document.execCommand("copy");
      node.remove();
    }
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div
      className={`flex items-start gap-2 rounded-lg border border-danger bg-danger-soft px-2 py-1.5 ${className}`}
    >
      <p className="selectable min-w-0 flex-1 wrap-break-word font-technical text-[10px] leading-snug text-danger">
        {message}
      </p>
      <button
        type="button"
        title={copied ? t("errorNotice.copied") : t("errorNotice.copy")}
        aria-label={t("errorNotice.copy")}
        onClick={() => void copy()}
        className="shrink-0 cursor-pointer text-danger opacity-70 hover:opacity-100"
      >
        <Icon name={copied ? "check" : "copy"} size={12} />
      </button>
      {onDismiss && (
        <button
          type="button"
          aria-label={t("errorNotice.dismiss")}
          onClick={onDismiss}
          className="shrink-0 cursor-pointer text-danger opacity-70 hover:opacity-100"
        >
          <Icon name="close" size={12} />
        </button>
      )}
    </div>
  );
}
