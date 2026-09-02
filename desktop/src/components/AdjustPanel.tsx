// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { MAX_SCALE, MIN_SCALE, type Clip } from "../lib/editor";
import { t, useLocale } from "../lib/i18n";
import { Group, Slider, Toggle } from "./controls";
import { Icon } from "./Icon";
import { Empty } from "./Panel";

/**
 * Level range, in decibels.
 *
 * The bottom is silence, not -60: a fader that cannot reach zero is a fader
 * you cannot use to mute. +24 at the top because quiet material genuinely
 * needs it - a distant lavalier or a phone recording is routinely 15-20 dB
 * under, and a limit that cannot fix those is a limit in the wrong place.
 */
const MIN_DB = -60;
const MAX_DB = 24;
/** A fade longer than half the clip would overlap the other one. */
const MAX_FADE = 10;

/** Linear gain to decibels. Zero maps to the bottom of the fader, not -inf. */
function toDecibels(gain: number): number {
  if (gain <= 0) return MIN_DB;
  return Math.max(MIN_DB, Math.min(MAX_DB, 20 * Math.log10(gain)));
}

function fromDecibels(decibels: number): number {
  // At the very bottom the fader means silence, so snap to exactly zero
  // rather than leaving an inaudible-but-nonzero gain in the project.
  return decibels <= MIN_DB ? 0 : 10 ** (decibels / 20);
}

function formatDecibels(decibels: number): string {
  if (decibels <= MIN_DB) return t("adjust.silent");
  if (Math.abs(decibels) < 0.05) return "0.0 dB";
  return `${decibels > 0 ? "+" : ""}${decibels.toFixed(1)} dB`;
}

/**
 * The Adjust tab: properties of the selected clip that change how it plays.
 *
 * Everything here applies in two places - the preview, so you hear it now, and
 * the exporter, so the file matches. Both read the same numbers off the clip,
 * which is the only way those two can be guaranteed to agree.
 *
 * The fader works in decibels rather than linear gain. Linear puts every
 * useful setting in the bottom sixth of the travel; decibels is both how the
 * ear works and how anyone mixing thinks.
 */
export function AdjustPanel({
  clip,
  onChange,
  onCommit,
  onSpeedChange,
}: {
  clip: Clip | null;
  /** Live change - echoed locally while the gesture is in flight. */
  onChange: (patch: Partial<Clip>) => void;
  /** Gesture finished - the accumulated change becomes one engine command. */
  onCommit: () => void;
  /** Separate, because changing speed also rescales the clip's duration. */
  onSpeedChange: (speed: number) => void;
}) {
  const { t } = useLocale();

  if (!clip) {
    return (
      <Empty icon={<Icon name="settings" size={26} strokeWidth={1.5} />}>
        {t("adjust.empty")}
      </Empty>
    );
  }

  const fadeLimit = Math.max(0.1, Math.min(MAX_FADE, clip.duration / 2));
  const hasPicture = clip.kind === "video" || clip.kind === "image";
  // A still is picture only: no gain to set, nothing a fade could quieten.
  const hasSound = clip.kind !== "image";

  return (
    <div className="px-3 py-3">
      {hasPicture && (
        <Group title={t("adjust.transform")} help={t("adjust.transformHelp")}>
          <Slider
            label={t("adjust.scale")}
            value={clip.scale}
            min={MIN_SCALE}
            max={MAX_SCALE}
            step={0.01}
            format={(value) => `${Math.round(value * 100)}%`}
            onReset={() => { onChange({ scale: 1 }); onCommit(); }}
            onChange={(scale) => onChange({ scale })}
            onCommit={onCommit}
          />
          <Slider
            label={t("adjust.positionX")}
            value={clip.offsetX}
            min={-1}
            max={1}
            step={0.005}
            format={(value) => `${Math.round(value * 100)}%`}
            onReset={() => { onChange({ offsetX: 0 }); onCommit(); }}
            onChange={(offsetX) => onChange({ offsetX })}
            onCommit={onCommit}
          />
          <Slider
            label={t("adjust.positionY")}
            value={clip.offsetY}
            min={-1}
            max={1}
            step={0.005}
            format={(value) => `${Math.round(value * 100)}%`}
            onReset={() => { onChange({ offsetY: 0 }); onCommit(); }}
            onChange={(offsetY) => onChange({ offsetY })}
            onCommit={onCommit}
          />
          <Slider
            label={t("adjust.rotation")}
            value={clip.rotation}
            min={-180}
            max={180}
            step={1}
            format={(value) => `${Math.round(value)}°`}
            onReset={() => { onChange({ rotation: 0 }); onCommit(); }}
            onChange={(rotation) => onChange({ rotation })}
            onCommit={onCommit}
          />
          <Slider
            label={t("adjust.opacity")}
            value={clip.opacity}
            min={0}
            max={1}
            step={0.01}
            format={(value) => `${Math.round(value * 100)}%`}
            onReset={() => { onChange({ opacity: 1 }); onCommit(); }}
            onChange={(opacity) => onChange({ opacity })}
            onCommit={onCommit}
          />
        </Group>
      )}

      {hasSound && (
      <Group title={t("adjust.volume")}>
        <Slider
          label={t("adjust.level")}
          value={toDecibels(clip.volume)}
          min={MIN_DB}
          max={MAX_DB}
          step={0.5}
          format={formatDecibels}
          onReset={() => { onChange({ volume: 1 }); onCommit(); }}
          onChange={(decibels) => onChange({ volume: fromDecibels(decibels) })}
          onCommit={onCommit}
        />
      </Group>
      )}

      {clip.kind !== "image" && (
        <Group title={t("adjust.speed")} help={t("adjust.speedHelp")}>
          {/* The engine's full range. The track is linear so the useful
              0.25-4x band sits left of centre; the number field beside it is
              how the extremes are actually dialled in. */}
          <Slider
            label={t("adjust.rate")}
            value={clip.speed}
            min={0.0625}
            max={16}
            step={0.05}
            format={(value) => `${value.toFixed(2)}x`}
            onReset={() => { onSpeedChange(1); onCommit(); }}
            onChange={onSpeedChange}
            onCommit={onCommit}
          />
          <Toggle
            label={t("adjust.pitchToggle")}
            hint={t("adjust.pitchHint")}
            checked={!clip.preservePitch}
            onChange={(checked) => { onChange({ preservePitch: !checked }); onCommit(); }}
          />
        </Group>
      )}

      {hasSound && (
      <Group title={t("adjust.fades")}>
        <Slider
          label={t("adjust.fadeIn")}
          value={Math.min(clip.fadeIn, fadeLimit)}
          min={0}
          max={fadeLimit}
          step={0.05}
          format={(value) => (value === 0 ? t("adjust.none") : `${value.toFixed(2)}s`)}
          onReset={() => { onChange({ fadeIn: 0 }); onCommit(); }}
          onChange={(fadeIn) => onChange({ fadeIn })}
          onCommit={onCommit}
        />
        <Slider
          label={t("adjust.fadeOut")}
          value={Math.min(clip.fadeOut, fadeLimit)}
          min={0}
          max={fadeLimit}
          step={0.05}
          format={(value) => (value === 0 ? t("adjust.none") : `${value.toFixed(2)}s`)}
          onReset={() => { onChange({ fadeOut: 0 }); onCommit(); }}
          onChange={(fadeOut) => onChange({ fadeOut })}
          onCommit={onCommit}
        />
      </Group>
      )}
    </div>
  );
}
