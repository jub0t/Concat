import { useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";

import { useLocale } from "../../lib/i18n";
import { clamp, cx, decimalsOf, roundTo, usePointerDrag } from "./base";
import { NumberField } from "./fields";
import styles from "./inspector.module.css";

/**
 * The inspector's repeating parts: a section, a row, a fader, the parameter
 * that pairs the two, a switch, a help disclosure and a colour well.
 *
 * Ported from the wc-ui-rnd design study and wired to this app's editing
 * contract: `onChange` echoes live while a gesture is in flight, `onCommit`
 * ends it and turns everything echoed into a single engine command. A control
 * that changes without ever committing leaves the edit local, so every path
 * out of a gesture here ends in a commit.
 */

/* ── section and row ──────────────────────────────────────────────────────── */

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className={styles.section}>
      <div className={styles.sectionHeader}>
        <span className={styles.sectionTitle}>{title}</span>
      </div>
      <div className={styles.sectionBody}>{children}</div>
    </div>
  );
}

export function Row({
  label,
  stack = false,
  children,
}: {
  label?: string;
  /** Stack the controls instead of laying them out in one line. */
  stack?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={styles.row}>
      {label !== undefined && <span className={styles.rowLabel}>{label}</span>}
      <div className={cx(styles.controls, stack && styles.stacked)}>{children}</div>
    </div>
  );
}

/* ── fader ────────────────────────────────────────────────────────────────── */

/** Horizontal fader. Click to jump, drag to ride, double-click to reset. */
export function Fader({
  value,
  onChange,
  onCommit,
  onReset,
  min,
  max,
  step,
  valueText,
  "aria-label": ariaLabel,
}: {
  value: number;
  onChange: (value: number) => void;
  onCommit?: () => void;
  /** Double-click restores this. Omitted means double-click does nothing. */
  onReset?: () => void;
  min: number;
  max: number;
  step: number;
  /** What a screen reader says instead of the bare number. */
  valueText?: string;
  "aria-label"?: string;
}) {
  const track = useRef<HTMLDivElement>(null);
  const digits = decimalsOf(step);
  const span = max - min || 1;

  const settle = (next: number) => onChange(roundTo(clamp(next, min, max), digits));

  // The rail is inset from the track by the border plus 2px. Measuring against
  // the rail rather than the track is what puts the cap under the pointer at
  // both ends instead of a few px short of them.
  const INSET = 3;
  const applyAt = (clientX: number) => {
    const rect = track.current?.getBoundingClientRect();
    if (!rect) return;
    const usable = rect.width - INSET * 2;
    if (usable <= 0) return;
    settle(min + clamp((clientX - rect.left - INSET) / usable, 0, 1) * span);
  };

  const beginDrag = usePointerDrag({
    threshold: 0,
    cursor: "ew-resize",
    onStart: ({ x }) => applyAt(x),
    onMove: ({ x }) => applyAt(x),
    onEnd: () => onCommit?.(),
  });

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Same modifier grammar as the number field: shift coarsens, alt refines.
    const scale = event.shiftKey ? 10 : event.altKey ? 0.1 : 1;
    const moves: Record<string, number> = {
      ArrowRight: step * scale,
      ArrowUp: step * scale,
      ArrowLeft: -step * scale,
      ArrowDown: -step * scale,
      PageUp: step * 10,
      PageDown: -step * 10,
    };
    if (event.key === "Home") {
      event.preventDefault();
      settle(min);
    } else if (event.key === "End") {
      event.preventDefault();
      settle(max);
    } else if (event.key in moves) {
      event.preventDefault();
      settle(value + (moves[event.key] ?? 0));
    }
  };

  const percent = clamp((value - min) / span, 0, 1) * 100;

  return (
    <div
      ref={track}
      className={styles.fader}
      role="slider"
      tabIndex={0}
      aria-label={ariaLabel}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuetext={valueText}
      onPointerDown={beginDrag}
      onKeyDown={onKeyDown}
      // One commit per burst of keys, on release, for the same reason a drag
      // commits once: undo should step over the adjustment, not through it.
      onKeyUp={() => onCommit?.()}
      onDoubleClick={onReset}
    >
      <div className={styles.rail}>
        <span className={styles.fill} style={{ width: `${percent}%` }} />
        <span
          className={styles.cap}
          style={{ left: `${percent}%`, transform: `translateX(-${percent}%)` }}
        />
      </div>
    </div>
  );
}

/* ── parameter ────────────────────────────────────────────────────────────── */

/**
 * The panel's repeating unit: a name and its value in the unit a person thinks
 * in, over a fader and the raw number. Two ways to the same value - ride the
 * fader for a feel, scrub or type the field for a figure.
 */
export function Param({
  label,
  value,
  onChange,
  onCommit,
  onReset,
  min,
  max,
  step,
  format,
  help,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  onCommit?: () => void;
  onReset?: () => void;
  min: number;
  max: number;
  step: number;
  /** The readout beside the label: the value in the unit a person thinks in. */
  format: (value: number) => string;
  help?: ReactNode;
}) {
  const { t } = useLocale();
  const text = format(value);

  return (
    <Row stack>
      <div className={styles.head}>
        <span className={styles.labelGroup}>
          <span
            className={cx(styles.label, onReset && styles.resettable)}
            title={onReset ? t("controls.resetHint") : undefined}
            onDoubleClick={onReset}
          >
            {label}
          </span>
          {help}
        </span>
        <span className={styles.readout}>{text}</span>
      </div>
      <div className={styles.body}>
        <Fader
          value={value}
          onChange={onChange}
          onCommit={onCommit}
          onReset={onReset}
          min={min}
          max={max}
          step={step}
          valueText={text}
          aria-label={label}
        />
        <div className={styles.numberSlot}>
          <NumberField
            value={value}
            onChange={onChange}
            onCommit={onCommit}
            min={min}
            max={max}
            step={step}
            aria-label={t("controls.sliderValue", { label })}
          />
        </div>
      </div>
    </Row>
  );
}

/* ── switch ───────────────────────────────────────────────────────────────── */

/** A labelled switch on one line. Discrete, so it commits as it changes. */
export function SwitchRow({
  label,
  checked,
  onChange,
  help,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  /** A `HelpButton`, for the settings that need a sentence rather than a name. */
  help?: ReactNode;
}) {
  return (
    <Row>
      <div className={styles.switchRow}>
        <span className={styles.label}>{label}</span>
        {help}
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          aria-label={label}
          onClick={() => onChange(!checked)}
          className={cx(styles.switch, checked && styles.switchOn)}
        >
          <span className={styles.thumb} />
        </button>
      </div>
    </Row>
  );
}

/* ── help disclosure ──────────────────────────────────────────────────────── */

/**
 * A tooltip hides the answer again the moment the pointer moves, and reaching
 * one from the keyboard takes machinery. This toggles a line of prose the
 * caller renders where it belongs instead.
 */
export function HelpButton({
  label,
  open,
  onToggle,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className={cx(styles.help, open && styles.helpOn)}
      aria-label={label}
      aria-expanded={open}
      onClick={onToggle}
    >
      ?
    </button>
  );
}

export function Note({ children }: { children: ReactNode }) {
  return <p className={styles.note}>{children}</p>;
}

/* ── colour ───────────────────────────────────────────────────────────────── */

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;

/**
 * A colour well and its hex, because one of the two is always the faster way.
 *
 * The typed hex is held as a draft until it is a whole colour: repainting on
 * "#f" would flash the title black on the way to "#ff0000". The well itself
 * paints live, since dragging around a picker with no feedback is guesswork.
 */
export function ColourField({
  label,
  name = label,
  value,
  onChange,
}: {
  /** The row's visible label. Short: the section above already says which. */
  label: string;
  /** What the control is called out of that context, for a screen reader. */
  name?: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);

  const commit = (text: string) => {
    const candidate = text.trim().replace(/^#?/, "#");
    if (HEX.test(candidate)) onChange(candidate.toLowerCase());
    setDraft(null);
  };

  return (
    <Row label={label}>
      <input
        className={styles.hex}
        value={draft ?? value}
        spellCheck={false}
        aria-label={name}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={(event) => commit(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") {
            setDraft(null);
            event.currentTarget.blur();
          }
        }}
      />
      <span className={styles.swatch} style={{ background: value }}>
        <input
          type="color"
          className={styles.swatchInput}
          value={value}
          aria-label={name}
          onChange={(event) => onChange(event.target.value)}
        />
      </span>
    </Row>
  );
}
