// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import type { ReactNode } from "react";

import { activeTimeline, type Clip, type EditorProject, type MediaItem } from "../lib/editor";
import { useLocale } from "../lib/i18n";
import { shortDuration, timecode } from "../lib/time";

/**
 * Properties of the current selection - a clip if one is selected, otherwise
 * the highlighted bin item, otherwise the project itself.
 *
 * The rows are read-only; edits go through Modify, whose changes reach the
 * engine and come back as new state - the panel stays dumb.
 */
export function Inspector({
  clip,
  media,
  frameRate,
  project,
  projectName,
  projectPath,
  frame,
  duration,
  onModify,
}: {
  clip: Clip | null;
  media: MediaItem | null;
  frameRate: number;
  project: EditorProject;
  projectName: string;
  projectPath: string;
  /** The output size, as currently set in the preview footer. */
  frame: { width: number; height: number };
  /** Timeline length in seconds. */
  duration: number;
  /** Opens the project-details editor (name, output frame). */
  onModify: () => void;
}) {
  const { t } = useLocale();

  return (
    <div className="flex min-h-full flex-col">
      {!clip && !media ? (
        <>
          <div className="selectable flex-1 px-3 py-2">
            <Section title={t("inspector.project")}>
              <Row label={t("inspector.name")} value={projectName} />
              <Row label={t("inspector.folder")} value={projectPath} mono wrap />
              <Row label={t("inspector.output")} value={`${frame.width} x ${frame.height}`} mono />
              <Row label={t("inspector.rate")} value={`${frameRate.toFixed(2)} fps`} mono />
              <Row label={t("inspector.duration")} value={timecode(duration, frameRate)} mono />
            </Section>
            <Section title={t("inspector.contents")}>
              <Row label={t("inspector.media")} value={`${project.media.length}`} mono />
              <Row
                label={t("inspector.tracks")}
                value={`${activeTimeline(project).tracks.length}`}
                mono
              />
              <Row
                label={t("inspector.clips")}
                value={`${activeTimeline(project).clips.length}`}
                mono
              />
            </Section>
          </div>
          <div className="flex justify-end border-t border-hairline px-3 py-2">
            <button
              type="button"
              onClick={onModify}
              className="cursor-pointer rounded-lg bg-hover px-3.5 py-1.5 text-xs font-medium
                         text-primary transition-colors hover:bg-active"
            >
              {t("inspector.modify")}
            </button>
          </div>
        </>
      ) : (
        <div className="selectable px-3 py-2">
          {clip && (
            <Section title={t("inspector.clip")}>
              <Row label={t("inspector.name")} value={clip.name} />
              <Row label={t("inspector.track")} value={clip.trackId} mono />
              <Row label={t("inspector.start")} value={timecode(clip.start, frameRate)} mono />
              <Row label={t("inspector.duration")} value={timecode(clip.duration, frameRate)} mono />
              <Row
                label={t("inspector.end")}
                value={timecode(clip.start + clip.duration, frameRate)}
                mono
              />
              <Row
                label={t("inspector.inPoint")}
                value={timecode(clip.sourceStart, frameRate)}
                mono
              />
            </Section>
          )}

          {media && (
            <>
              <Section title={t("inspector.source")}>
                <Row label={t("inspector.name")} value={media.name} />
                <Row label={t("inspector.path")} value={media.path} mono wrap />
                <Row label={t("inspector.duration")} value={shortDuration(media.duration)} mono />
              </Section>

              {media.width !== null && (
                <Section title={t("inspector.video")}>
                  <Row
                    label={t("inspector.codec")}
                    value={media.videoCodec ?? t("inspector.unknown")}
                  />
                  <Row label={t("inspector.size")} value={`${media.width} x ${media.height}`} mono />
                  <Row
                    label={t("inspector.rate")}
                    value={
                      media.frameRate ? `${media.frameRate.toFixed(3)} fps` : t("inspector.unknown")
                    }
                    mono
                  />
                  <Row
                    label={t("inspector.exact")}
                    value={media.frameRateFraction ?? t("inspector.unknown")}
                    mono
                  />
                </Section>
              )}

              {media.hasAudio && (
                <Section title={t("inspector.audio")}>
                  <Row
                    label={t("inspector.codec")}
                    value={media.audioCodec ?? t("inspector.unknown")}
                  />
                </Section>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mb-4">
      <h3 className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
        {title}
      </h3>
      <dl className="space-y-1.5">{children}</dl>
    </section>
  );
}

function Row({
  label,
  value,
  mono,
  wrap,
}: {
  label: string;
  value: string;
  mono?: boolean;
  wrap?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0 text-xs text-secondary">{label}</dt>
      <dd
        title={value}
        className={`min-w-0 text-right text-xs text-primary ${mono ? "text-[11px]" : ""} ${
          wrap ? "break-all" : "truncate"
        }`}
      >
        {value}
      </dd>
    </div>
  );
}
