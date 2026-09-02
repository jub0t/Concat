// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { desktopDir, join } from "@tauri-apps/api/path";

import {
  createProject,
  forgetProject,
  newMediaFromSummary,
  openProject,
  probeMedia,
  projectPreview,
  recentProjects,
  templateDelete,
  templateInstantiate,
  templateList,
  type ProjectInfo,
  type SlotFill,
  type TemplateInfo,
  type TemplateSlot,
} from "../lib/engine";
import { ErrorNotice } from "./ErrorNotice";
import { useLocale, type MsgKey } from "../lib/i18n";
import { relativeTime } from "../lib/time";
import { Icon } from "./Icon";
import { TemplateThumb } from "./TemplateThumb";

/** The settings a project is created with. Fixed for its lifetime, for now. */
export interface ProjectSession {
  name: string;
  /** Absolute path to the project folder on disk. */
  path: string;
  width: number;
  height: number;
  /** Frames per second as a decimal, for display. */
  frameRate: number;
  /** The same rate as an exact fraction. Export uses these, never the decimal. */
  rateNum: number;
  rateDen: number;
}

const RESOLUTIONS: { labelKey: MsgKey; width: number; height: number }[] = [
  { labelKey: "startScreen.res1080p", width: 1920, height: 1080 },
  { labelKey: "startScreen.res720p", width: 1280, height: 720 },
  { labelKey: "startScreen.res4k", width: 3840, height: 2160 },
  { labelKey: "startScreen.resVertical", width: 1080, height: 1920 },
];

// Stored as fractions because 29.97 is 30000/1001 and never anything else.
const FRAME_RATES = [
  { label: "24", num: 24, den: 1 },
  { label: "25", num: 25, den: 1 },
  { label: "29.97", num: 30000, den: 1001 },
  { label: "30", num: 30, den: 1 },
  { label: "60", num: 60, den: 1 },
] as const;

/**
 * The launch screen.
 *
 * Full-bleed on the window's own background - no card, no border, no drop
 * shadow. A setup screen is not a dialog floating over something; it *is* the
 * window, and framing it as a card only adds an edge that means nothing.
 * Hierarchy comes from type size and whitespace instead.
 *
 * An editor should not drop you straight into an untitled timeline either: the
 * frame rate, size and location are decisions the whole edit depends on, and
 * choosing them after material is cut is how you end up conforming footage.
 */
export function StartScreen({
  onCreate,
  initialTemplate = null,
}: {
  onCreate: (session: ProjectSession) => void;
  /** Opens straight into a template's fill mode - how "Use template" from
   * inside the editor lands here. */
  initialTemplate?: TemplateInfo | null;
}) {
  const { t, tp } = useLocale();
  const [name, setName] = useState(initialTemplate?.name ?? t("startScreen.untitled"));
  const [location, setLocation] = useState("");
  const [resolution, setResolution] = useState<(typeof RESOLUTIONS)[number]>(RESOLUTIONS[0]);
  const [rate, setRate] = useState<(typeof FRAME_RATES)[number]>(FRAME_RATES[3]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recents, setRecents] = useState<ProjectInfo[]>([]);
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  /** A chosen template swaps the form into fill mode. */
  const [template, setTemplate] = useState<TemplateInfo | null>(initialTemplate);
  /** The chosen media per slot, keyed by the slot's media id. */
  const [fills, setFills] = useState<Record<string, SlotFill>>({});

  // Default to Desktop/WolfCut. Failing to resolve it is not worth surfacing;
  // the field simply starts empty and Choose still works.
  useEffect(() => {
    void desktopDir()
      .then((desktop) => join(desktop, "WolfCut"))
      .then(setLocation)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    void recentProjects()
      .then(setRecents)
      .catch(() => undefined);
    void templateList()
      .then(setTemplates)
      .catch(() => undefined);
  }, []);

  const chooseTemplate = (chosen: TemplateInfo) => {
    setTemplate(chosen);
    setFills({});
    setError(null);
    setName(chosen.name);
  };

  const removeTemplate = async (doomed: TemplateInfo) => {
    setTemplates((current) => current.filter((entry) => entry.path !== doomed.path));
    if (template?.path === doomed.path) setTemplate(null);
    await templateDelete(doomed.path).catch(() => undefined);
  };

  /** Asks for one slot's file and probes it into fill shape. */
  const pickSlot = async (slot: TemplateSlot) => {
    if (busy) return;
    const chosen = await open({
      multiple: false,
      title: t("startScreen.chooseMediaFor", { name: slot.name }),
      filters: [
        slot.kind === "audio"
          ? {
              name: t("startScreen.audioFilter"),
              extensions: ["mp3", "wav", "flac", "aac", "m4a", "ogg", "opus"],
            }
          : {
              // A photo in a video slot is fine - it freeze-frames for the
              // slot's length - so visual slots take either.
              name: t("startScreen.videoImagesFilter"),
              extensions: [
                "mp4", "mov", "mkv", "webm", "avi", "m4v",
                "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "gif",
              ],
            },
        { name: t("startScreen.allFilesFilter"), extensions: ["*"] },
      ],
    });
    if (typeof chosen !== "string") return;

    setError(null);
    try {
      const summary = await probeMedia(chosen);
      setFills((current) => ({
        ...current,
        [slot.mediaId]: { mediaId: slot.mediaId, item: newMediaFromSummary(summary) },
      }));
    } catch (cause) {
      setError(String(cause));
    }
  };

  const createFromTemplate = async () => {
    if (!template || busy || !location.trim()) return;
    const ready = template.slots.every((slot) => fills[slot.mediaId]);
    if (!ready) return;

    setBusy(true);
    setError(null);
    try {
      const project = await templateInstantiate({
        template: template.path,
        location: location.trim(),
        name: name.trim() || template.name,
        fills: template.slots.map((slot) => fills[slot.mediaId]),
      });
      onCreate(toSession(project));
    } catch (cause) {
      setError(String(cause));
      setBusy(false);
    }
  };

  const reopen = async (project: ProjectInfo) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      // Re-read from disk rather than trusting the cached entry: the manifest
      // is the truth, and the folder may have been edited or moved since.
      const fresh = await openProject(project.path);
      onCreate(toSession(fresh));
    } catch (cause) {
      setError(String(cause));
      setBusy(false);
    }
  };

  const forget = async (project: ProjectInfo) => {
    setRecents((current) => current.filter((entry) => entry.path !== project.path));
    await forgetProject(project.path).catch(() => undefined);
  };

  const browse = async () => {
    const chosen = await open({
      directory: true,
      multiple: false,
      title: t("startScreen.locationTitle"),
      defaultPath: location || undefined,
    });
    if (typeof chosen === "string") setLocation(chosen);
  };

  const create = async () => {
    const trimmed = name.trim() || t("startScreen.untitled");
    if (!location.trim() || busy) return;

    setBusy(true);
    setError(null);
    try {
      // The engine creates the folder and writes the manifest, and hands back
      // where it actually landed - which is not always `location/name`, since
      // the name has to be made filesystem-safe first.
      const project = await createProject({
        location: location.trim(),
        name: trimmed,
        width: resolution.width,
        height: resolution.height,
        rateNum: rate.num,
        rateDen: rate.den,
      });

      onCreate(toSession(project));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="thin-scroll h-full overflow-y-auto bg-base">
      {/*
        Two columns once there is history: the setup form on the left, the
        projects being returned to on the right - the two reasons anyone is on
        this screen, side by side instead of one buried under the other. With
        no history the form sits alone, centred, exactly as before.
      */}
      <div
        className={`mx-auto flex min-h-full w-full flex-col justify-center px-8 py-16 ${
          recents.length > 0
            ? "max-w-5xl lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(0,22rem)] lg:items-start lg:gap-16"
            : "max-w-xl"
        }`}
      >
      <div className="min-w-0">
        <header className="mb-10">
          {template && (
            <button
              type="button"
              onClick={() => setTemplate(null)}
              className="mb-3 flex cursor-pointer items-center gap-1 text-[12px] text-accent
                         transition-opacity hover:opacity-70"
            >
              <Icon name="chevronRight" size={11} className="rotate-180" />
              {t("startScreen.allTemplates")}
            </button>
          )}
          <h1 className="font-display text-[34px] font-semibold leading-tight tracking-[-0.03em] text-primary">
            {template ? template.name : t("startScreen.newProject")}
          </h1>
          <p className="mt-1.5 text-sm text-secondary">
            {template
              ? t("startScreen.templateIntro", {
                  width: template.width,
                  height: template.height,
                  rate: (template.rateNum / template.rateDen).toFixed(2),
                })
              : t("startScreen.settingsIntro")}
          </p>
        </header>

        <Field label={t("startScreen.name")}>
          <input
            value={name}
            autoFocus
            spellCheck={false}
            onChange={(event) => setName(event.target.value)}
            onFocus={(event) => event.currentTarget.select()}
            onKeyDown={(event) => event.key === "Enter" && void create()}
            className="w-full bg-transparent py-1 text-[15px] text-primary outline-none
                       placeholder:text-tertiary"
          />
        </Field>

        <Field label={t("startScreen.location")}>
          <div className="flex items-center gap-3">
            <input
              value={location}
              spellCheck={false}
              placeholder={t("startScreen.chooseFolder")}
              onChange={(event) => setLocation(event.target.value)}
              className="min-w-0 flex-1 bg-transparent py-1 font-technical text-[12px] text-primary
                         outline-none placeholder:text-tertiary"
            />
            <button
              type="button"
              onClick={() => void browse()}
              className="shrink-0 cursor-pointer text-[13px] text-accent transition-opacity
                         hover:opacity-70"
            >
              {t("startScreen.choose")}
            </button>
          </div>
        </Field>

        {!template && (
          <>
            <Field label={t("startScreen.resolution")}>
              <Segmented
                options={RESOLUTIONS.map((option) => t(option.labelKey))}
                value={t(resolution.labelKey)}
                onChange={(label) =>
                  setResolution(
                    RESOLUTIONS.find((option) => t(option.labelKey) === label) ?? RESOLUTIONS[0],
                  )
                }
              />
              <p className="mt-2 font-technical text-[11px] text-tertiary">
                {resolution.width} x {resolution.height}
              </p>
            </Field>

            <Field label={t("startScreen.frameRate")} last>
              <Segmented
                options={FRAME_RATES.map((option) => option.label)}
                value={rate.label}
                onChange={(label) =>
                  setRate(FRAME_RATES.find((option) => option.label === label) ?? FRAME_RATES[3])
                }
              />
              <p className="mt-2 font-technical text-[11px] text-tertiary">
                {rate.num}/{rate.den} fps
              </p>
            </Field>
          </>
        )}

        {template && (
          <Field label={t("startScreen.slots", { count: template.slots.length })} last>
            <ul className="flex flex-col gap-1.5">
              {template.slots.map((slot, index) => {
                const fill = fills[slot.mediaId];
                return (
                  <li key={slot.mediaId} className="flex items-center gap-2.5">
                    <span
                      className={`flex h-8 w-12 shrink-0 items-center justify-center rounded-md
                                  ${fill ? "bg-accent-soft text-accent" : "bg-sunken text-tertiary"}`}
                    >
                      <Icon
                        name={
                          slot.kind === "audio" ? "music" : slot.kind === "image" ? "image" : "film"
                        }
                        size={13}
                      />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] text-primary">
                        {fill ? fill.item.name : `${index + 1}. ${slot.name}`}
                      </span>
                      <span className="block font-technical text-[11px] text-tertiary">
                        {t("startScreen.slotSeconds", {
                          kind: t(
                            slot.kind === "audio"
                              ? "startScreen.kindAudio"
                              : slot.kind === "image"
                                ? "startScreen.kindImage"
                                : "startScreen.kindVideo",
                          ),
                          seconds: slot.seconds.toFixed(1),
                        })}
                      </span>
                    </span>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void pickSlot(slot)}
                      className="shrink-0 cursor-pointer text-[13px] text-accent transition-opacity
                                 hover:opacity-70 disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      {fill ? t("startScreen.change") : t("startScreen.choose")}
                    </button>
                  </li>
                );
              })}
              {template.slots.length === 0 && (
                <li className="text-[12px] text-tertiary">{t("startScreen.noSlots")}</li>
              )}
            </ul>
          </Field>
        )}

        {error && <ErrorNotice message={error} onDismiss={() => setError(null)} className="mt-6" />}

        <div className="mt-10 flex items-center gap-4">
          <button
            type="button"
            onClick={() => void (template ? createFromTemplate() : create())}
            disabled={
              busy ||
              !location.trim() ||
              (template !== null && !template.slots.every((slot) => fills[slot.mediaId]))
            }
            className="cursor-pointer rounded-full bg-accent px-6 py-2.5 text-[14px] font-medium
                       text-on-accent transition-colors hover:bg-accent-hover
                       disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? t("startScreen.creating") : t("startScreen.create")}
          </button>
          <p className="flex items-center gap-1.5 text-[11px] text-tertiary">
            <Icon name="info" size={12} className="shrink-0" />
            {template
              ? (() => {
                  const missing = template.slots.filter((slot) => !fills[slot.mediaId]).length;
                  return missing > 0
                    ? tp("startScreen.slotsMissing", missing)
                    : t("startScreen.slotsFilled");
                })()
              : t("startScreen.folderNote")}
          </p>
        </div>

        {!template && templates.length > 0 && (
          <section className="mt-14 border-t border-hairline pt-7">
            <h2 className="mb-3 text-[13px] font-semibold text-secondary">
              {t("startScreen.fromTemplate")}
            </h2>
            <ul className="grid grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] gap-3">
              {templates.map((entry) => (
                <li key={entry.path} className="group relative">
                  <button
                    type="button"
                    disabled={busy}
                    title={entry.path}
                    onClick={() => chooseTemplate(entry)}
                    className="w-full cursor-pointer text-left disabled:cursor-not-allowed"
                  >
                    <TemplateThumb template={entry} />
                    <span className="mt-1.5 block truncate text-[13px] text-primary">
                      {entry.name}
                    </span>
                    <span className="block font-technical text-[11px] text-tertiary">
                      {tp("startScreen.slotCount", entry.slots.length)} · {entry.width} x{" "}
                      {entry.height}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label={t("startScreen.deleteTemplate", { name: entry.name })}
                    title={t("startScreen.deleteTemplateHint")}
                    onClick={() => void removeTemplate(entry)}
                    className="invisible absolute right-1.5 top-1.5 cursor-pointer rounded
                               bg-black/55 p-1 text-white transition-colors hover:bg-danger
                               group-hover:visible"
                  >
                    <Icon name="trash" size={11} />
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

        {/* No empty state: a "Recent" heading over nothing is worse than no
            heading at all, so the column simply is not there until it has
            something to show. */}
        {recents.length > 0 && (
          <section className="mt-14 border-t border-hairline pt-7 lg:mt-0 lg:border-t-0 lg:pt-0">
            <h2 className="mb-3 text-[13px] font-semibold text-secondary">
              {t("startScreen.recent")}
            </h2>
            <ul className="flex flex-col gap-1.5">
              {recents.map((project) => (
                <li
                  key={project.path}
                  className="group flex items-center gap-3 rounded-xl p-2 transition-colors
                             hover:bg-hover"
                >
                  <button
                    type="button"
                    title={project.path}
                    disabled={busy}
                    onClick={() => void reopen(project)}
                    className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left
                               disabled:cursor-not-allowed"
                  >
                    <ProjectThumb project={project} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[14px] text-primary">
                        {project.name}
                      </span>
                      <span className="block truncate font-technical text-[11px] text-tertiary">
                        {project.width} x {project.height} ·{" "}
                        {(project.rateNum / project.rateDen).toFixed(2)} fps
                      </span>
                      <span className="block text-[11px] text-tertiary">
                        {relativeTime(project.openedAt)}
                      </span>
                    </span>
                  </button>

                  <button
                    type="button"
                    aria-label={t("startScreen.removeRecent", { name: project.name })}
                    title={t("startScreen.removeRecentHint")}
                    onClick={() => void forget(project)}
                    className="invisible shrink-0 cursor-pointer rounded p-1 text-tertiary
                               transition-colors hover:bg-active hover:text-primary
                               group-hover:visible"
                  >
                    <Icon name="close" size={12} />
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}

/**
 * A project's poster frame: a frame from the earliest clip of its timeline,
 * generated and cached by the host. A project with nothing visual on it - or
 * whose media has moved - falls back to a quiet film icon rather than an
 * error; the launch screen is not the place to complain about footage.
 */
function ProjectThumb({ project }: { project: ProjectInfo }) {
  const [poster, setPoster] = useState<string | null>(null);

  useEffect(() => {
    let url: string | null = null;
    let cancelled = false;
    void projectPreview(project.path)
      .then((bytes) => {
        if (cancelled) return;
        url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
        setPoster(url);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [project.path]);

  return (
    <span
      className="relative block aspect-video w-24 shrink-0 overflow-hidden rounded-lg bg-sunken
                 ring-1 ring-hairline"
    >
      {poster ? (
        <img src={poster} alt="" draggable={false} className="h-full w-full object-cover" />
      ) : (
        <span className="absolute inset-0 flex items-center justify-center text-tertiary">
          <Icon name="film" size={16} strokeWidth={1.5} />
        </span>
      )}
    </span>
  );
}

/** The decimal rate is derived, never stored - the fraction is the truth. */
function toSession(project: ProjectInfo): ProjectSession {
  return {
    name: project.name,
    path: project.path,
    width: project.width,
    height: project.height,
    frameRate: project.rateNum / project.rateDen,
    rateNum: project.rateNum,
    rateDen: project.rateDen,
  };
}

/** A labelled row separated by a hairline, in the style of a settings list. */
function Field({
  label,
  children,
  last = false,
}: {
  label: string;
  children: ReactNode;
  last?: boolean;
}) {
  return (
    <div
      className={`grid grid-cols-[7rem_minmax(0,1fr)] items-baseline gap-4 py-4 ${
        last ? "" : "border-b border-hairline"
      }`}
    >
      <span className="text-[13px] text-secondary">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/**
 * An Apple-style segmented control: one recessed track with a raised pill
 * marking the selection, rather than a row of separate buttons.
 */
function Segmented({
  options,
  value,
  onChange,
}: {
  options: readonly string[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div
      role="radiogroup"
      className="inline-flex rounded-lg bg-sunken p-0.5"
    >
      {options.map((option) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={option === value}
          onClick={() => onChange(option)}
          className={`cursor-pointer rounded-[6px] px-3.5 py-1 text-[13px] transition-colors ${
            option === value
              ? "bg-panel text-primary shadow-[0_1px_2px_rgba(0,0,0,0.14)]"
              : "text-secondary hover:text-primary"
          }`}
        >
          {option}
        </button>
      ))}
    </div>
  );
}
