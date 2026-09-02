// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useState } from "react";

import type { Clip, CustomFont } from "../lib/editor";
import { t, useLocale } from "../lib/i18n";
import { FONTS, textCss, WEIGHTS, type TextStyle } from "../lib/text";
import { Icon } from "./Icon";
import { clamp } from "./inspector/base";
import {
  ColourField,
  HelpButton,
  Note,
  Param,
  Row,
  Section,
  SwitchRow,
} from "./inspector/controls";
import { SegmentedControl, Select, type SelectOption } from "./inspector/fields";
import styles from "./inspector/inspector.module.css";
import { Empty } from "./Panel";

/**
 * The Text tab: everything about how a title looks.
 *
 * Sizes are fractions of the frame rather than points, so a title composed
 * against a 1080p project lands in the same place when the same project is
 * exported at 4K. The panel converts for display - nobody thinks in "0.09 of
 * the frame height" - but the stored value stays relative.
 *
 * The preview overlay and the export rasteriser both read these numbers
 * through `textCss`, so what is on screen is what gets burned in.
 *
 * This is the first panel on the new inspector chrome in `./inspector`:
 * hairline sections instead of shouty group headings, faders paired with a
 * scrubbable field, and a specimen of the title pinned to the top. The
 * settings and their bounds are unchanged - only the way they are handled is.
 */

/** Bounds shared with the monitor's corner drag; see Preview.tsx. */
const MIN_FONT_SIZE = 0.02;
const MAX_FONT_SIZE = 0.4;

/*
 * The specimen's scale.
 *
 * A fraction of the frame height means nothing in a 62px strip, so the sample
 * reads the size against a nominal surface and then holds the result to
 * something legible. The monitor beside this panel is the truth for scale and
 * placement; the specimen is here to judge face, weight, colour and outline.
 */
const SAMPLE_SURFACE = 190;
const SAMPLE_MIN = 11;
const SAMPLE_MAX = 34;

export function TextPanel({
  clip,
  fonts,
  onChange,
  onCommit,
  onAddFont,
  onRemoveFont,
}: {
  clip: Clip | null;
  /** The project's custom fonts, with the UI's missing-file marks applied. */
  fonts: CustomFont[];
  /** Live change - echoed locally while the gesture is in flight. */
  onChange: (patch: Partial<Clip>) => void;
  /** Gesture finished - the accumulated change becomes one engine command. */
  onCommit: () => void;
  onAddFont: () => void;
  onRemoveFont: (family: string) => void;
}) {
  const { t } = useLocale();
  const [shadowHelp, setShadowHelp] = useState(false);
  const [plateHelp, setPlateHelp] = useState(false);

  if (!clip || clip.kind !== "text" || !clip.text) {
    return (
      <Empty icon={<Icon name="type" size={26} strokeWidth={1.5} />}>
        {t("textPanel.empty")}
      </Empty>
    );
  }

  const style = clip.text;

  // Every edit rewrites the whole style, and the clip's name follows the first
  // line so the timeline label tracks the words without a second control.
  const set = (patch: Partial<TextStyle>) => {
    const next = { ...style, ...patch };
    onChange(
      patch.content === undefined
        ? { text: next }
        : { text: next, name: firstLineOf(patch.content) },
    );
  };

  // Discrete controls commit immediately; faders and the textarea echo live
  // and commit on release or blur.
  const commitSet = (patch: Partial<TextStyle>) => {
    set(patch);
    onCommit();
  };

  const commitClip = (patch: Partial<Clip>) => {
    onChange(patch);
    onCommit();
  };

  const families: SelectOption<string>[] = [
    ...FONTS.filter((font) => font.bundled).map((font) => ({
      value: font.value,
      label: font.label,
      group: t("textPanel.bundled"),
      fontFamily: font.value,
    })),
    ...fonts.map((font) => ({
      value: `"${font.family}"`,
      label: font.family,
      note: font.missing ? t("textPanel.fileMissing") : undefined,
      group: t("textPanel.yours"),
      // A missing face has nothing to draw itself in, so its row stays in the
      // panel's own type rather than silently borrowing the fallback's.
      fontFamily: font.missing ? undefined : `"${font.family}"`,
    })),
    ...FONTS.filter((font) => !font.bundled).map((font) => ({
      value: font.value,
      label: font.label,
      group: t("textPanel.system"),
      fontFamily: font.value,
    })),
  ];

  // The specimen is scaled to be readable rather than true, so the plate and
  // the outline - which `textCss` derives from the size - stay in proportion
  // to what is drawn instead of to the frame.
  const samplePx = clamp(style.fontSize * SAMPLE_SURFACE, SAMPLE_MIN, SAMPLE_MAX);
  const sample = textCss(style, samplePx / Math.max(style.fontSize, 0.001));

  return (
    <div className={styles.kit}>
      <div
        className={styles.sample}
        style={{
          justifyContent:
            style.align === "left" ? "flex-start" : style.align === "right" ? "flex-end" : "center",
        }}
      >
        {style.content.trim() === "" ? (
          <span className={styles.samplePlaceholder}>{t("textPanel.placeholder")}</span>
        ) : (
          <span
            className={styles.sampleText}
            style={{ ...sample, display: "inline-block", whiteSpace: "nowrap" }}
          >
            {style.content}
          </span>
        )}
      </div>

      <Section title={t("textPanel.content")}>
        <Row stack>
          <textarea
            className={styles.textarea}
            value={style.content}
            spellCheck={false}
            aria-label={t("textPanel.content")}
            placeholder={t("textPanel.placeholder")}
            onChange={(event) => set({ content: event.target.value })}
            onBlur={onCommit}
          />
        </Row>
      </Section>

      <Section title={t("textPanel.font")}>
        <Row stack>
          <Select
            options={families}
            value={style.fontFamily}
            onChange={(fontFamily) => commitSet({ fontFamily })}
            aria-label={t("textPanel.font")}
          />
        </Row>

        <Row stack>
          <button type="button" className={styles.dashed} onClick={onAddFont}>
            <Icon name="import" size={12} />
            {t("textPanel.addFontFile")}
          </button>
        </Row>

        {fonts.map((font) => (
          <div key={font.family} className={styles.fontRow}>
            <span
              className={styles.fontName}
              style={{ fontFamily: font.missing ? undefined : `"${font.family}"` }}
              title={font.path}
            >
              {font.family}
            </span>
            <button
              type="button"
              className={styles.fontRemove}
              aria-label={t("textPanel.removeFont", { name: font.family })}
              onClick={() => onRemoveFont(font.family)}
            >
              <Icon name="close" size={11} />
            </button>
          </div>
        ))}

        <Row stack>
          <Select
            options={WEIGHTS}
            value={style.fontWeight}
            onChange={(fontWeight) => commitSet({ fontWeight })}
            aria-label={t("textPanel.weight")}
          />
        </Row>

        <Param
          label={t("textPanel.size")}
          value={style.fontSize}
          min={MIN_FONT_SIZE}
          max={MAX_FONT_SIZE}
          step={0.005}
          // Shown as a share of frame height, which is the thing it actually
          // is. Points would be a lie: the frame can be any resolution.
          format={(value) => t("textPanel.sizeOfHeight", { value: (value * 100).toFixed(1) })}
          onChange={(fontSize) => set({ fontSize })}
          onCommit={onCommit}
          onReset={() => commitSet({ fontSize: 0.09 })}
        />

        <SwitchRow
          label={t("textPanel.italic")}
          checked={style.italic}
          onChange={(italic) => commitSet({ italic })}
        />
      </Section>

      <Section title={t("textPanel.colour")}>
        <ColourField
          label={t("textPanel.fill")}
          value={style.color}
          onChange={(color) => commitSet({ color })}
        />

        <Param
          label={t("textPanel.opacity")}
          value={style.opacity}
          min={0}
          max={1}
          step={0.01}
          format={(value) => `${Math.round(value * 100)}%`}
          onChange={(opacity) => set({ opacity })}
          onCommit={onCommit}
          onReset={() => commitSet({ opacity: 1 })}
        />

        <SwitchRow
          label={t("textPanel.dropShadow")}
          checked={style.shadow}
          onChange={(shadow) => commitSet({ shadow })}
          help={
            <HelpButton
              label={t("textPanel.about", { name: t("textPanel.dropShadow") })}
              open={shadowHelp}
              onToggle={() => setShadowHelp((on) => !on)}
            />
          }
        />
        {shadowHelp && <Note>{t("textPanel.dropShadowHint")}</Note>}

        <SwitchRow
          label={t("textPanel.plate")}
          checked={style.background !== ""}
          onChange={(on) => commitSet({ background: on ? "#000000" : "" })}
          help={
            <HelpButton
              label={t("textPanel.about", { name: t("textPanel.plate") })}
              open={plateHelp}
              onToggle={() => setPlateHelp((on) => !on)}
            />
          }
        />
        {plateHelp && <Note>{t("textPanel.plateHint")}</Note>}
        {style.background !== "" && (
          <ColourField
            label={t("textPanel.colour")}
            name={t("textPanel.plateColour")}
            value={style.background}
            onChange={(background) => commitSet({ background })}
          />
        )}
      </Section>

      <Section title={t("textPanel.outline")}>
        <Param
          label={t("textPanel.width")}
          value={style.strokeWidth}
          min={0}
          max={0.15}
          step={0.005}
          format={(value) => (value === 0 ? t("textPanel.none") : `${(value * 100).toFixed(1)}%`)}
          onChange={(strokeWidth) => set({ strokeWidth })}
          onCommit={onCommit}
          onReset={() => commitSet({ strokeWidth: 0 })}
        />
        {style.strokeWidth > 0 && (
          <ColourField
            label={t("textPanel.colour")}
            name={t("textPanel.outlineColour")}
            value={style.strokeColor}
            onChange={(strokeColor) => commitSet({ strokeColor })}
          />
        )}
      </Section>

      <Section title={t("textPanel.layout")}>
        <Row stack>
          <SegmentedControl
            options={[
              { value: "left" as const, label: t("textPanel.alignLeft") },
              { value: "center" as const, label: t("textPanel.alignCenter") },
              { value: "right" as const, label: t("textPanel.alignRight") },
            ]}
            value={style.align}
            onChange={(align) => commitSet({ align })}
            aria-label={t("textPanel.layout")}
          />
        </Row>

        <Param
          label={t("textPanel.lineHeight")}
          value={style.lineHeight}
          min={0.8}
          max={2.5}
          step={0.05}
          format={(value) => value.toFixed(2)}
          onChange={(lineHeight) => set({ lineHeight })}
          onCommit={onCommit}
          onReset={() => commitSet({ lineHeight: 1.2 })}
        />

        <Param
          label={t("textPanel.tracking")}
          value={style.tracking}
          min={-0.1}
          max={0.4}
          step={0.005}
          format={(value) => `${value >= 0 ? "+" : ""}${(value * 100).toFixed(1)}%`}
          onChange={(tracking) => set({ tracking })}
          onCommit={onCommit}
          onReset={() => commitSet({ tracking: 0 })}
        />
      </Section>

      {/*
        Offsets are fractions of the frame, and centred is zero - so the reset
        on each is "put it back in the middle", and a title composed here sits
        in the same place at any export resolution.
      */}
      <Section title={t("textPanel.position")}>
        <Param
          label={t("textPanel.horizontal")}
          value={clip.offsetX}
          min={-0.5}
          max={0.5}
          step={0.005}
          format={centred}
          onChange={(offsetX) => onChange({ offsetX })}
          onCommit={onCommit}
          onReset={() => commitClip({ offsetX: 0 })}
        />
        <Param
          label={t("textPanel.vertical")}
          value={clip.offsetY}
          min={-0.5}
          max={0.5}
          step={0.005}
          format={centred}
          onChange={(offsetY) => onChange({ offsetY })}
          onCommit={onCommit}
          onReset={() => commitClip({ offsetY: 0 })}
        />
      </Section>
    </div>
  );
}

function centred(value: number): string {
  return Math.abs(value) < 0.0025 ? t("textPanel.centred") : `${(value * 100).toFixed(1)}%`;
}

function firstLineOf(content: string): string {
  const line = content.split("\n").find((candidate) => candidate.trim() !== "");
  return (line ?? t("textPanel.defaultName")).trim().slice(0, 40) || t("textPanel.defaultName");
}
