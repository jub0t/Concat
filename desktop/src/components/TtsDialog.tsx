// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import {
  cancelSpeak,
  cancelTtsModelDownload,
  downloadTtsModel,
  onSpeakProgress,
  onTtsDownload,
  speakText,
  ttsStatus,
  type TtsStatus,
  type TtsVoice,
} from "../lib/engine";
import { useLocale, type MsgKey } from "../lib/i18n";
import { getTtsModel, getTtsVoice, setTtsVoice } from "../lib/settings";
import { ErrorNotice } from "./ErrorNotice";
import { Icon } from "./Icon";

/** Speaking-rate presets, in the terms a person picking one thinks in. */
const PACES: { labelKey: MsgKey; speed: number }[] = [
  { labelKey: "tts.pace.slower", speed: 0.85 },
  { labelKey: "tts.pace.natural", speed: 1.0 },
  { labelKey: "tts.pace.faster", speed: 1.2 },
];

type Phase =
  | { kind: "idle" }
  | { kind: "running"; fraction: number }
  | { kind: "failed"; message: string };

/**
 * Decodes a Kokoro voice name's prefix - `af` American female, `bm` British
 * male, `zf` Chinese female - into the keys of its picker group.
 */
function voiceGroup(name: string): { accent: MsgKey; gender: MsgKey } | null {
  const accent: Record<string, MsgKey> = {
    a: "tts.accent.american",
    b: "tts.accent.british",
    z: "tts.accent.chinese",
  };
  const gender: Record<string, MsgKey> = {
    f: "tts.gender.female",
    m: "tts.gender.male",
  };
  const [prefix] = name.split("_");
  const a = accent[prefix?.[0] ?? ""];
  const g = gender[prefix?.[1] ?? ""];
  return a && g ? { accent: a, gender: g } : null;
}

/** "af_heart" -> "Heart": the part after the prefix, title-cased. */
function voiceLabel(name: string): string {
  const rest = name.split("_")[1] ?? name;
  return rest.charAt(0).toUpperCase() + rest.slice(1);
}

/**
 * The speech sheet: text in, a narration clip at the playhead out.
 *
 * Synthesis runs in the host and reports through an event, so this listens
 * rather than waits - a paragraph takes long enough that a silent button
 * reads as a hang. If the voice model is not on disk yet the sheet offers
 * the download right here instead of sending the user to Settings first.
 */
export function TtsDialog({
  projectPath,
  initialText,
  onGenerated,
  onClose,
}: {
  /** The open project's folder - where the host writes the WAV. */
  projectPath: string;
  /** Prefills the text box - a title clip's content when spoken from one. */
  initialText?: string;
  /** Places the finished WAV on the timeline; throwing lands in the sheet's
   * error notice. The sheet closes itself afterwards. */
  onGenerated: (path: string, duration: number) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState(initialText ?? "");
  const [voice, setVoice] = useState(getTtsVoice());
  const [speed, setSpeed] = useState(1.0);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [status, setStatus] = useState<TtsStatus | null>(null);
  /** Fraction of the model download, or "unpacking", or null when idle. */
  const [download, setDownload] = useState<{ fraction: number; unpacking: boolean } | null>(null);
  const unlisten = useRef<(() => void)[]>([]);

  const model = getTtsModel();

  const refresh = () => {
    ttsStatus()
      .then(setStatus)
      .catch((cause: unknown) => setPhase({ kind: "failed", message: String(cause) }));
  };

  useEffect(() => {
    const stops = unlisten.current;
    refresh();
    void onSpeakProgress(({ fraction }) => {
      setPhase((current) => (current.kind === "running" ? { kind: "running", fraction } : current));
    }).then((stop) => stops.push(stop));
    void onTtsDownload((progress) => {
      if (progress.done) {
        setDownload(null);
        refresh();
      } else {
        setDownload({
          fraction: progress.total > 0 ? progress.received / progress.total : 0,
          unpacking: progress.unpacking,
        });
      }
    }).then((stop) => stops.push(stop));
    return () => stops.forEach((stop) => stop());
  }, []);

  // A remembered id that no longer names a known model (an old build's
  // catalog, say) falls back to the first offered one instead of a dead sheet.
  const entry =
    status?.models.find((candidate) => candidate.id === model) ?? status?.models[0] ?? null;
  const ready = entry?.downloaded ?? false;
  const running = phase.kind === "running";

  /** Voices in picker order, grouped by accent and gender. */
  const groups: { label: string; voices: TtsVoice[] }[] = [];
  for (const candidate of status?.voices ?? []) {
    const decoded = voiceGroup(candidate.name);
    if (!decoded) continue;
    const label = `${t(decoded.accent)} · ${t(decoded.gender)}`;
    const group = groups.find((existing) => existing.label === label);
    if (group) group.voices.push(candidate);
    else groups.push({ label, voices: [candidate] });
  }

  const fetchModel = () => {
    if (!entry) return;
    setDownload({ fraction: 0, unpacking: false });
    downloadTtsModel(entry.id)
      .then(() => {
        setDownload(null);
        refresh();
      })
      .catch((cause: unknown) => {
        setDownload(null);
        setPhase({ kind: "failed", message: String(cause) });
        refresh();
      });
  };

  const generate = async () => {
    if (running || !ready || !entry || !text.trim()) return;
    setPhase({ kind: "running", fraction: 0 });
    try {
      const result = await speakText({
        modelId: entry.id,
        voice,
        text: text.trim(),
        speed,
        project: projectPath,
      });
      await onGenerated(result.path, result.duration);
      onClose();
    } catch (cause) {
      const message = String(cause);
      // A cancel is the user's own decision, not a failure to report.
      if (message.includes("cancelled")) setPhase({ kind: "idle" });
      else setPhase({ kind: "failed", message });
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8
                 backdrop-blur-[2px]"
      onMouseDown={(event) => {
        // The backdrop closes the sheet the way every sheet closes - but
        // never out from under a running synthesis.
        if (event.target === event.currentTarget && !running) onClose();
      }}
    >
      <div className="surface w-full max-w-md rounded-2xl p-6">
        <div className="mb-5 flex items-center gap-2">
          <Icon name="volume" size={17} className="text-accent" />
          <h2 className="flex-1 text-sm font-semibold text-primary">{t("tts.title")}</h2>
          {!running && (
            <button
              type="button"
              aria-label={t("common.close")}
              onClick={onClose}
              className="cursor-pointer rounded p-1 text-secondary hover:bg-hover hover:text-primary"
            >
              <Icon name="close" size={14} />
            </button>
          )}
        </div>

        {status !== null && !ready ? (
          // No model on disk: the sheet is a download button until there is.
          <div className="py-2">
            <p className="mb-4 text-xs leading-relaxed text-secondary">{t("tts.modelNeeded")}</p>
            {download ? (
              <>
                <div className="mb-2 flex items-baseline justify-between gap-3">
                  <span className="text-xs text-primary">
                    {download.unpacking ? t("tts.unpacking") : t("tts.downloading")}
                  </span>
                  {!download.unpacking && (
                    <span className="shrink-0 font-technical text-sm tabular-nums text-primary">
                      {`${Math.round(download.fraction * 100)}%`}
                    </span>
                  )}
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-active">
                  <div
                    className="h-full rounded-full bg-accent transition-[width] duration-200"
                    style={{ width: `${download.unpacking ? 100 : Math.round(download.fraction * 100)}%` }}
                  />
                </div>
                <button
                  type="button"
                  onClick={() => void cancelTtsModelDownload()}
                  className="mt-4 w-full cursor-pointer rounded-lg bg-hover px-4 py-2 text-xs
                             text-secondary transition-colors hover:bg-active hover:text-primary"
                >
                  {t("common.cancel")}
                </button>
              </>
            ) : (
              <>
                {phase.kind === "failed" && <ErrorNotice message={phase.message} className="mb-4" />}
                <button
                  type="button"
                  onClick={fetchModel}
                  className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-lg
                             bg-accent px-4 py-2.5 text-sm font-medium text-on-accent
                             transition-colors hover:bg-accent-hover"
                >
                  <Icon name="import" size={15} />
                  {t("tts.downloadModel", {
                    size: entry ? `${Math.round(entry.sizeBytes / 1_000_000)} MB` : "",
                  })}
                </button>
              </>
            )}
          </div>
        ) : (
          <>
            <Label>{t("tts.textLabel")}</Label>
            <textarea
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder={t("tts.textPlaceholder")}
              rows={5}
              autoFocus
              disabled={running}
              className="mb-5 w-full resize-none rounded-lg border border-hairline bg-sunken px-3
                         py-2 text-xs leading-relaxed text-primary placeholder:text-tertiary
                         focus:border-accent focus:outline-none disabled:opacity-60"
            />

            <Label>{t("tts.voiceLabel")}</Label>
            <select
              value={voice}
              disabled={running}
              onChange={(event) => {
                const next = Number(event.target.value);
                setVoice(next);
                setTtsVoice(next);
              }}
              className="mb-5 w-full cursor-pointer rounded-lg border border-hairline bg-sunken
                         px-3 py-2 text-xs text-primary focus:border-accent focus:outline-none
                         disabled:opacity-60"
            >
              {groups.map((group) => (
                <optgroup key={group.label} label={group.label}>
                  {group.voices.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>
                      {voiceLabel(candidate.name)}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>

            <Label>{t("tts.paceLabel")}</Label>
            <div className="mb-5 grid grid-cols-3 gap-1.5">
              {PACES.map((option) => (
                <button
                  key={option.labelKey}
                  type="button"
                  aria-pressed={option.speed === speed}
                  disabled={running}
                  onClick={() => setSpeed(option.speed)}
                  className={`cursor-pointer rounded-lg px-2 py-1.5 text-xs transition-colors ${
                    option.speed === speed
                      ? "bg-accent text-on-accent"
                      : "bg-hover text-secondary hover:bg-active"
                  }`}
                >
                  {t(option.labelKey)}
                </button>
              ))}
            </div>

            {phase.kind === "failed" && <ErrorNotice message={phase.message} className="mb-4" />}

            {running ? (
              <div>
                <div className="mb-2 flex items-baseline justify-between gap-3">
                  <span className="text-xs text-primary">{t("tts.generating")}</span>
                  <span className="shrink-0 font-technical text-sm tabular-nums text-primary">
                    {`${Math.round(phase.fraction * 100)}%`}
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-active">
                  <div
                    className="h-full rounded-full bg-accent transition-[width] duration-200"
                    style={{ width: `${Math.max(3, Math.round(phase.fraction * 100))}%` }}
                  />
                </div>
                <button
                  type="button"
                  onClick={() => void cancelSpeak().catch(() => undefined)}
                  className="mt-4 w-full cursor-pointer rounded-lg bg-hover px-4 py-2 text-xs
                             text-secondary transition-colors hover:bg-active hover:text-primary"
                >
                  {t("common.cancel")}
                </button>
              </div>
            ) : (
              <button
                type="button"
                onClick={() => void generate()}
                disabled={!text.trim() || !ready}
                className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-lg
                           bg-accent px-4 py-2.5 text-sm font-medium text-on-accent
                           transition-colors hover:bg-accent-hover disabled:cursor-not-allowed
                           disabled:opacity-40"
              >
                <Icon name="volume" size={15} />
                {t("tts.generate")}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function Label({ children }: { children: ReactNode }) {
  return (
    <span className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wider text-secondary">
      {children}
    </span>
  );
}
