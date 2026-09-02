// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import {
  cancelModelDownload,
  cancelTtsModelDownload,
  deleteTranscriberModel,
  deleteTtsModel,
  downloadTranscriberModel,
  downloadTtsModel,
  engineVersion,
  onTranscriberDownload,
  onTtsDownload,
  setTranscriberBinary,
  transcriberStatus,
  ttsStatus,
  type TranscriberStatus,
  type TtsStatus,
} from "../lib/engine";
import { LOCALES, systemLocale, useLocale, type MsgKey } from "../lib/i18n";
import {
  getTranscriberLanguage,
  getTranscriberModel,
  getTtsModel,
  setTranscriberLanguage,
  setTranscriberModel,
  setTtsModel,
} from "../lib/settings";
import { HelpTip } from "./controls";
import { Icon } from "./Icon";

/** The pages of the settings dialog, in display order. */
type SettingsTab = "general" | "transcriber" | "speech" | "about";

const TABS: {
  id: SettingsTab;
  labelKey: MsgKey;
  icon: "settings" | "waveform" | "volume" | "info";
}[] = [
  { id: "general", labelKey: "settings.tabs.general", icon: "settings" },
  { id: "transcriber", labelKey: "settings.tabs.transcriber", icon: "waveform" },
  { id: "speech", labelKey: "settings.tabs.speech", icon: "volume" },
  { id: "about", labelKey: "settings.tabs.about", icon: "info" },
];

/**
 * The settings sheet: a tab rail on the left, one feature's settings on the
 * right - the same shape as the library panel, so the app has one way of
 * laying out "categories of things".
 *
 * Settings here are app-level (this machine), never project state.
 */
export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<SettingsTab>("general");
  const { t } = useLocale();

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-8
                 backdrop-blur-[2px]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="surface flex h-[460px] w-full max-w-2xl flex-col rounded-2xl">
        <div className="flex items-center gap-2 border-b border-hairline px-5 py-3.5">
          <Icon name="settings" size={16} className="text-accent" />
          <h2 className="flex-1 text-sm font-semibold text-primary">{t("settings.title")}</h2>
          <button
            type="button"
            aria-label={t("common.close")}
            onClick={onClose}
            className="cursor-pointer rounded p-1 text-secondary hover:bg-hover hover:text-primary"
          >
            <Icon name="close" size={14} />
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          <aside className="w-40 shrink-0 border-r border-hairline p-2">
            {TABS.map((entry) => (
              <button
                key={entry.id}
                type="button"
                aria-pressed={tab === entry.id}
                onClick={() => setTab(entry.id)}
                className={`flex w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5
                            text-xs transition-colors ${
                              tab === entry.id
                                ? "bg-active text-primary"
                                : "text-secondary hover:bg-hover"
                            }`}
              >
                <Icon name={entry.icon} size={13} className="shrink-0 opacity-70" />
                {t(entry.labelKey)}
              </button>
            ))}
          </aside>

          <div className="thin-scroll min-w-0 flex-1 overflow-y-auto px-5 py-4">
            {tab === "general" && <GeneralSettings />}
            {tab === "transcriber" && <TranscriberSettings />}
            {tab === "speech" && <SpeechSettings />}
            {tab === "about" && <AboutSettings />}
          </div>
        </div>
      </div>
    </div>
  );
}

function GeneralSettings() {
  const { t, preference, setLocale } = useLocale();
  // Native names, straight from LOCALES: whoever is stranded in a language
  // they cannot read must still recognise their own in this list.
  const system = LOCALES.find((entry) => entry.id === systemLocale())!;
  const choices = [
    {
      id: "system" as const,
      name: t("settings.general.systemDefault", { language: system.name }),
    },
    ...LOCALES,
  ];

  return (
    <div className="flex flex-col gap-5">
      <section>
        <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
          {t("settings.general.language")}
        </h3>
        <div className="flex w-72 flex-col gap-1.5">
          {choices.map((entry) => {
            const selected = preference === entry.id;
            return (
              <button
                key={entry.id}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => setLocale(entry.id)}
                className={`flex cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-left
                            ring-1 transition-shadow ${
                              selected ? "ring-accent" : "ring-hairline hover:ring-hairline-strong"
                            }`}
              >
                <span
                  className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full
                             ring-1 ring-hairline-strong"
                >
                  {selected && <span className="h-2 w-2 rounded-full bg-accent" />}
                </span>
                <span className="truncate text-xs text-primary">{entry.name}</span>
              </button>
            );
          })}
        </div>
        <p className="mt-2 text-[11px] leading-snug text-tertiary">
          {t("settings.general.engineNote")}
        </p>
      </section>
    </div>
  );
}

/** Bytes as a short human figure for model rows. */
function formatSize(bytes: number): string {
  return bytes >= 1_000_000_000
    ? `${(bytes / 1_000_000_000).toFixed(1)} GB`
    : `${Math.round(bytes / 1_000_000)} MB`;
}

/** Whisper input languages - what the audio is in, not what the UI speaks. */
const LANGUAGES: { id: string; labelKey: MsgKey }[] = [
  { id: "auto", labelKey: "settings.transcriber.languageAuto" },
  { id: "en", labelKey: "settings.transcriber.languageEnglish" },
];

function TranscriberSettings() {
  const { t } = useLocale();
  const [status, setStatus] = useState<TranscriberStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [model, setModel] = useState(getTranscriberModel());
  const [language, setLanguage] = useState(getTranscriberLanguage());
  /** The model being downloaded and how far along it is, or null. */
  const [download, setDownload] = useState<{ id: string; fraction: number } | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  const refresh = () => {
    transcriberStatus()
      .then((next) => {
        setStatus(next);
        setError(null);
      })
      .catch((cause: unknown) => setError(String(cause)));
  };

  useEffect(() => {
    refresh();
    void onTranscriberDownload((progress) => {
      if (progress.done) {
        setDownload(null);
        refresh();
      } else {
        setDownload({
          id: progress.id,
          fraction: progress.total > 0 ? progress.received / progress.total : 0,
        });
      }
    }).then((stop) => {
      unlisten.current = stop;
    });
    return () => unlisten.current?.();
  }, []);

  const locate = async () => {
    const chosen = await open({
      title: t("settings.transcriber.locateTitle"),
      multiple: false,
      filters: [{ name: t("settings.transcriber.locateFilter"), extensions: ["exe", "*"] }],
    });
    if (typeof chosen !== "string") return;
    try {
      setStatus(await setTranscriberBinary(chosen));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    }
  };

  const startDownload = (id: string) => {
    setDownload({ id, fraction: 0 });
    downloadTranscriberModel(id)
      .then(() => {
        setDownload(null);
        refresh();
      })
      .catch((cause: unknown) => {
        setDownload(null);
        setError(String(cause));
        refresh();
      });
  };

  const remove = (id: string) => {
    deleteTranscriberModel(id)
      .then(refresh)
      .catch((cause: unknown) => setError(String(cause)));
  };

  const chooseModel = (id: string) => {
    setModel(id);
    setTranscriberModel(id);
  };

  const chooseLanguage = (id: string) => {
    setLanguage(id);
    setTranscriberLanguage(id);
  };

  return (
    <div className="flex flex-col gap-5">
      {/* Which binary transcribes is plumbing, not a setting: a healthy
          install says nothing about it. The bar exists only for a dev build
          with no engine at all, where Locate... is the way out. */}
      {status !== null && !status.binary && (
        <section>
          <div className="flex items-center gap-2 rounded-lg bg-sunken px-3 py-2.5">
            <span className="h-2 w-2 shrink-0 rounded-full bg-danger" />
            <span className="min-w-0 flex-1 truncate text-xs text-secondary">
              {t("settings.transcriber.engineMissing")}
            </span>
            <button
              type="button"
              onClick={() => void locate()}
              className="shrink-0 cursor-pointer rounded-md bg-panel px-2.5 py-1 text-[11px]
                         text-primary ring-1 ring-hairline transition-colors hover:bg-hover"
            >
              {t("settings.transcriber.locate")}
            </button>
          </div>
        </section>
      )}

      <section>
        <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
          {t("settings.transcriber.language")}
        </h3>
        <div className="flex w-56 rounded-lg bg-sunken p-0.5">
          {LANGUAGES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              aria-pressed={language === entry.id}
              onClick={() => chooseLanguage(entry.id)}
              className={`flex-1 cursor-pointer rounded-md px-2 py-1 text-[12px] transition-colors ${
                language === entry.id
                  ? "bg-panel text-primary shadow-[0_1px_2px_rgba(0,0,0,0.14)]"
                  : "text-secondary hover:text-primary"
              }`}
            >
              {t(entry.labelKey)}
            </button>
          ))}
        </div>
      </section>

      <section>
        <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
          {t("settings.transcriber.models")}
          <HelpTip align="start" text={t("settings.transcriber.modelsHelp")} />
        </h3>

        <ul className="flex flex-col gap-1.5">
          {(status?.models ?? []).map((entry) => {
            const downloading = download?.id === entry.id;
            const selected = model === entry.id;
            return (
              <li
                key={entry.id}
                // The whole card selects, not just the radio dot: the dot is
                // a 16px target and the card is what the eye reads as the
                // option. Buttons inside stop propagation so deleting or
                // downloading is never also a selection.
                onClick={() => entry.downloaded && chooseModel(entry.id)}
                className={`flex items-center gap-2.5 rounded-lg px-3 py-2 ring-1 transition-shadow ${
                  selected ? "ring-accent" : "ring-hairline"
                } ${entry.downloaded ? "cursor-pointer" : ""} ${
                  entry.downloaded && !selected ? "hover:ring-hairline-strong" : ""
                }`}
              >
                <button
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={t("settings.transcriber.useModel", { model: entry.label })}
                  disabled={!entry.downloaded}
                  onClick={() => chooseModel(entry.id)}
                  title={
                    entry.downloaded
                      ? t("settings.transcriber.useModelHint")
                      : t("settings.transcriber.downloadFirstHint")
                  }
                  className="flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center
                             rounded-full ring-1 ring-hairline-strong
                             disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {selected && <span className="h-2 w-2 rounded-full bg-accent" />}
                </button>

                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs text-primary">{entry.label}</span>
                  <span className="block truncate text-[11px] text-tertiary">{entry.blurb}</span>
                </span>

                <span className="shrink-0 font-technical text-[10px] text-tertiary">
                  {formatSize(entry.sizeBytes)}
                </span>

                {downloading ? (
                  <span className="flex shrink-0 items-center gap-1.5">
                    <span className="h-1 w-16 overflow-hidden rounded-full bg-sunken">
                      <span
                        className="block h-full bg-accent transition-[width]"
                        style={{ width: `${Math.round(download.fraction * 100)}%` }}
                      />
                    </span>
                    <button
                      type="button"
                      aria-label={t("settings.transcriber.cancelDownload")}
                      onClick={(event) => {
                        event.stopPropagation();
                        void cancelModelDownload();
                      }}
                      className="cursor-pointer rounded p-0.5 text-secondary hover:bg-hover
                                 hover:text-primary"
                    >
                      <Icon name="close" size={10} />
                    </button>
                  </span>
                ) : entry.downloaded ? (
                  <span className="flex shrink-0 items-center gap-1">
                    <Icon name="check" size={12} className="text-success" />
                    <button
                      type="button"
                      aria-label={t("settings.transcriber.deleteModel", { model: entry.label })}
                      title={t("settings.transcriber.deleteModelHint")}
                      onClick={(event) => {
                        event.stopPropagation();
                        remove(entry.id);
                      }}
                      className="cursor-pointer rounded p-0.5 text-secondary transition-colors
                                 hover:bg-hover hover:text-danger"
                    >
                      <Icon name="trash" size={11} />
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    disabled={download !== null}
                    onClick={(event) => {
                      event.stopPropagation();
                      startDownload(entry.id);
                    }}
                    className="shrink-0 cursor-pointer rounded-md bg-panel px-2.5 py-1 text-[11px]
                               text-primary ring-1 ring-hairline transition-colors hover:bg-hover
                               disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {t("settings.transcriber.download")}
                  </button>
                )}
              </li>
            );
          })}
        </ul>

        {error && <p className="mt-2 text-[11px] leading-snug text-danger">{error}</p>}
      </section>
    </div>
  );
}

/**
 * The speech (text to speech) page: which Kokoro bundle to keep on disk.
 *
 * The same shape as the transcriber page, minus the binary bar (synthesis
 * runs in-process, there is no tool to locate) and minus a language toggle
 * (the voice picked in the speech dialog carries the language).
 */
function SpeechSettings() {
  const { t } = useLocale();
  const [status, setStatus] = useState<TtsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [model, setModel] = useState(getTtsModel());
  /** The model being fetched: how far along, and whether it is unpacking. */
  const [download, setDownload] = useState<{
    id: string;
    fraction: number;
    unpacking: boolean;
  } | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  const refresh = () => {
    ttsStatus()
      .then((next) => {
        setStatus(next);
        setError(null);
      })
      .catch((cause: unknown) => setError(String(cause)));
  };

  useEffect(() => {
    refresh();
    void onTtsDownload((progress) => {
      if (progress.done) {
        setDownload(null);
        refresh();
      } else {
        setDownload({
          id: progress.id,
          fraction: progress.total > 0 ? progress.received / progress.total : 0,
          unpacking: progress.unpacking,
        });
      }
    }).then((stop) => {
      unlisten.current = stop;
    });
    return () => unlisten.current?.();
  }, []);

  const startDownload = (id: string) => {
    setDownload({ id, fraction: 0, unpacking: false });
    downloadTtsModel(id)
      .then(() => {
        setDownload(null);
        refresh();
      })
      .catch((cause: unknown) => {
        setDownload(null);
        setError(String(cause));
        refresh();
      });
  };

  const remove = (id: string) => {
    deleteTtsModel(id)
      .then(refresh)
      .catch((cause: unknown) => setError(String(cause)));
  };

  const chooseModel = (id: string) => {
    setModel(id);
    setTtsModel(id);
  };

  return (
    <div className="flex flex-col gap-5">
      <section>
        <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
          {t("settings.speech.models")}
          <HelpTip align="start" text={t("settings.speech.modelsHelp")} />
        </h3>

        <ul className="flex flex-col gap-1.5">
          {(status?.models ?? []).map((entry) => {
            const downloading = download?.id === entry.id;
            const selected = model === entry.id;
            return (
              <li
                key={entry.id}
                onClick={() => entry.downloaded && chooseModel(entry.id)}
                className={`flex items-center gap-2.5 rounded-lg px-3 py-2 ring-1 transition-shadow ${
                  selected ? "ring-accent" : "ring-hairline"
                } ${entry.downloaded ? "cursor-pointer" : ""} ${
                  entry.downloaded && !selected ? "hover:ring-hairline-strong" : ""
                }`}
              >
                <button
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={t("settings.speech.useModel", { model: entry.label })}
                  disabled={!entry.downloaded}
                  onClick={() => chooseModel(entry.id)}
                  title={
                    entry.downloaded
                      ? t("settings.speech.useModelHint")
                      : t("settings.speech.downloadFirstHint")
                  }
                  className="flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center
                             rounded-full ring-1 ring-hairline-strong
                             disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {selected && <span className="h-2 w-2 rounded-full bg-accent" />}
                </button>

                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs text-primary">{entry.label}</span>
                  <span className="block truncate text-[11px] text-tertiary">{entry.blurb}</span>
                </span>

                <span className="shrink-0 font-technical text-[10px] text-tertiary">
                  {formatSize(entry.sizeBytes)}
                </span>

                {downloading ? (
                  <span className="flex shrink-0 items-center gap-1.5">
                    {download.unpacking ? (
                      <span className="text-[10px] text-tertiary">
                        {t("settings.speech.unpacking")}
                      </span>
                    ) : (
                      <span className="h-1 w-16 overflow-hidden rounded-full bg-sunken">
                        <span
                          className="block h-full bg-accent transition-[width]"
                          style={{ width: `${Math.round(download.fraction * 100)}%` }}
                        />
                      </span>
                    )}
                    <button
                      type="button"
                      aria-label={t("settings.speech.cancelDownload")}
                      onClick={(event) => {
                        event.stopPropagation();
                        void cancelTtsModelDownload();
                      }}
                      className="cursor-pointer rounded p-0.5 text-secondary hover:bg-hover
                                 hover:text-primary"
                    >
                      <Icon name="close" size={10} />
                    </button>
                  </span>
                ) : entry.downloaded ? (
                  <span className="flex shrink-0 items-center gap-1">
                    <Icon name="check" size={12} className="text-success" />
                    <button
                      type="button"
                      aria-label={t("settings.speech.deleteModel", { model: entry.label })}
                      title={t("settings.speech.deleteModelHint")}
                      onClick={(event) => {
                        event.stopPropagation();
                        remove(entry.id);
                      }}
                      className="cursor-pointer rounded p-0.5 text-secondary transition-colors
                                 hover:bg-hover hover:text-danger"
                    >
                      <Icon name="trash" size={11} />
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    disabled={download !== null}
                    onClick={(event) => {
                      event.stopPropagation();
                      startDownload(entry.id);
                    }}
                    className="shrink-0 cursor-pointer rounded-md bg-panel px-2.5 py-1 text-[11px]
                               text-primary ring-1 ring-hairline transition-colors hover:bg-hover
                               disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {t("settings.speech.download")}
                  </button>
                )}
              </li>
            );
          })}
        </ul>

        <p className="mt-2 text-[11px] leading-snug text-tertiary">
          {t("settings.speech.privacyNote")}
        </p>

        {error && <p className="mt-2 text-[11px] leading-snug text-danger">{error}</p>}
      </section>
    </div>
  );
}

function AboutSettings() {
  const { t } = useLocale();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    engineVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
        WolfCut
      </h3>
      <p className="text-xs text-secondary">
        {t("settings.about.version", {
          version: version ?? t("settings.about.versionUnknown"),
        })}
      </p>
      <p className="text-[11px] leading-relaxed text-tertiary">{t("settings.about.blurb")}</p>
    </div>
  );
}
