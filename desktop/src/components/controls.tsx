// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from "react";

import { useLocale } from "../lib/i18n";
import { Icon } from "./Icon";

/**
 * Property controls for the inspector.
 *
 * Pure: value in, callback out. None of them hold state, because the value
 * they show has to be whatever the project says it is - including when it
 * changes from somewhere else, like a drag on the timeline. (The Slider's
 * typing state is the one exception, and it is display state, not a value:
 * the project is not touched until the entry commits.)
 */

/**
 * A circled question mark that explains itself on hover.
 *
 * Inline help paragraphs cost a permanent line each and read like a manual
 * pasted into a tool. This keeps the explanation one hover away instead, so
 * the panel stays controls-first.
 */
export function HelpTip({ text, align = "end" }: { text: string; align?: "start" | "end" }) {
  return (
    <span className="group/help relative inline-flex shrink-0">
      <Icon
        name="help"
        size={12}
        className="cursor-help text-tertiary transition-colors group-hover/help:text-secondary"
      />
      <span
        role="tooltip"
        // The bubble must open towards the room. "end" hangs it leftward from
        // the icon, right for the inspector where the icon hugs the window's
        // right edge; "start" opens rightward, for an icon near the left of a
        // scroll container - hanging left there pushes the bubble outside the
        // container, where it is clipped instead of shown.
        className={`pointer-events-none invisible absolute top-full z-50 mt-1.5 w-52
                   rounded-md border border-hairline bg-panel p-2 text-[11px] font-normal
                   normal-case leading-snug tracking-normal text-secondary
                   shadow-[0_4px_14px_rgba(0,0,0,0.25)] group-hover/help:visible ${
                     align === "end" ? "right-0" : "left-0"
                   }`}
      >
        {text}
      </span>
    </span>
  );
}

/** How much finer a shift-drag moves than a plain one. */
const FINE_FACTOR = 0.1;

/**
 * A slider whose whole track is the grab target.
 *
 * Clicking anywhere on it jumps there and starts dragging, the way parameter
 * sliders work in editors rather than the way a browser range input does. The
 * fill doubles as the readout, so there is no separate progress element to
 * keep in step.
 *
 * Holding Shift while dragging moves at a tenth of the rate, for dialling in
 * an exact value; clicking the readout turns it into a text field for typing
 * one directly.
 */
export function Slider({
  label,
  value,
  min,
  max,
  step = 0.01,
  format,
  onChange,
  onCommit,
  onReset,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Turns the raw value into what the reader sees. */
  format: (value: number) => string;
  onChange: (value: number) => void;
  /**
   * Fires when a gesture finishes: pointer released, typed value committed,
   * or a keyboard step. The engine records one undo entry per commit, so
   * this is what makes a drag one undoable edit instead of sixty.
   */
  onCommit?: () => void;
  /** Double-clicking the label returns the control to this value. */
  onReset?: () => void;
}) {
  const { t } = useLocale();
  const track = useRef<HTMLDivElement>(null);
  // Where the last pointer event landed, so a shift-drag can be relative to
  // the previous position rather than jumping to the pointer.
  const lastX = useRef(0);
  // What the number field holds while it is being edited. Null means it shows
  // the value; the project is not touched until the entry commits.
  const [typing, setTyping] = useState<string | null>(null);

  const clampStep = (raw: number) => {
    const stepped = Math.round(raw / step) * step;
    // Re-round, or steps accumulate float noise like 0.30000000000000004.
    return Number(Math.min(max, Math.max(min, stepped)).toFixed(6));
  };

  const commit = (event: ReactPointerEvent<HTMLDivElement>) => {
    const bounds = track.current?.getBoundingClientRect();
    if (!bounds || bounds.width === 0) return;

    if (event.shiftKey) {
      // Fine mode: the pointer's movement counts for a tenth. Relative to the
      // last position, so toggling shift mid-drag never jumps the value.
      const delta = ((event.clientX - lastX.current) / bounds.width) * (max - min) * FINE_FACTOR;
      lastX.current = event.clientX;
      onChange(clampStep(value + delta));
      return;
    }

    lastX.current = event.clientX;
    const fraction = Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width));
    onChange(clampStep(min + fraction * (max - min)));
  };

  const commitTyped = () => {
    if (typing === null) return;
    const parsed = Number.parseFloat(typing.replace(",", "."));
    if (Number.isFinite(parsed)) onChange(clampStep(parsed));
    setTyping(null);
    onCommit?.();
  };

  const fill = max === min ? 0 : Math.min(1, Math.max(0, (value - min) / (max - min)));

  return (
    <div className="mb-3">
      <div className="mb-1 flex items-baseline justify-between gap-2">
        <span
          onDoubleClick={onReset}
          title={onReset ? t("controls.resetHint") : undefined}
          className={`text-[12px] text-secondary ${onReset ? "cursor-pointer" : ""}`}
        >
          {label}
        </span>
        <span className="font-technical text-[11px] text-tertiary">{format(value)}</span>
      </div>

      <div className="flex items-stretch gap-1.5">
        <div
          ref={track}
          role="slider"
          aria-label={label}
          aria-valuenow={value}
          aria-valuemin={min}
          aria-valuemax={max}
          tabIndex={0}
          onKeyDown={(event) => {
            // Shift jumps by ten steps - the keyboard's coarse mode, mirroring
            // how shift is the pointer's fine mode: both mean "the other rate".
            const by = event.shiftKey ? step * 10 : step;
            if (event.key === "ArrowLeft") onChange(clampStep(value - by));
            if (event.key === "ArrowRight") onChange(clampStep(value + by));
          }}
          onKeyUp={(event) => {
            if (event.key === "ArrowLeft" || event.key === "ArrowRight") onCommit?.();
          }}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            lastX.current = event.clientX;
            // A shift-click means "adjust from here", not "jump here".
            if (!event.shiftKey) commit(event);
          }}
          onPointerMove={(event) => {
            if (event.currentTarget.hasPointerCapture(event.pointerId)) commit(event);
          }}
          onPointerUp={(event) => {
            if (event.currentTarget.hasPointerCapture(event.pointerId)) {
              event.currentTarget.releasePointerCapture(event.pointerId);
            }
            onCommit?.();
          }}
          className="relative h-6 min-w-0 flex-1 cursor-ew-resize touch-none select-none
                     overflow-hidden rounded-md bg-sunken"
        >
          <div
            className="absolute inset-y-0 left-0 bg-tool-active-soft"
            style={{ width: `${fill * 100}%` }}
          />
          {/* The knob subtracts its own width as it travels, so it stays inside
              the track at both ends instead of hanging over the edge. */}
          <div
            className="absolute top-0 h-full w-1 rounded bg-tool-active"
            style={{ left: `calc(${fill * 100}% - ${fill * 4}px)` }}
          />
        </div>

        {/*
          The number field: the slider's exact twin in height, always present,
          for dialling in a value the track is too coarse for. A text input
          rather than type="number" because the spinner buttons are the part
          nobody wants - arrow keys step instead, and typing commits on Enter
          or blur, reverting on Escape.
        */}
        <input
          value={typing ?? String(Number(value.toFixed(4)))}
          inputMode="decimal"
          spellCheck={false}
          aria-label={t("controls.sliderValue", { label })}
          onChange={(event) => setTyping(event.target.value)}
          onFocus={(event) => event.target.select()}
          onBlur={commitTyped}
          onKeyDown={(event: ReactKeyboardEvent<HTMLInputElement>) => {
            if (event.key === "Enter") commitTyped();
            if (event.key === "Escape") {
              setTyping(null);
              event.currentTarget.blur();
            }
            if (event.key === "ArrowUp" || event.key === "ArrowDown") {
              event.preventDefault();
              const by = (event.shiftKey ? step * 10 : step) * (event.key === "ArrowUp" ? 1 : -1);
              setTyping(null);
              onChange(clampStep(value + by));
              onCommit?.();
            }
          }}
          className="h-6 w-14 shrink-0 rounded-md border border-hairline bg-sunken px-1.5
                     text-right font-technical text-[11px] text-primary outline-none
                     transition-colors focus:border-accent"
        />
      </div>
    </div>
  );
}

/**
 * A sliding on/off switch.
 *
 * A real `<button role="switch">` rather than a native checkbox: the platform
 * control cannot be styled into this shape, and it looks like a form field
 * dropped into a tool that has none. Being a button means keyboard and screen
 * reader behaviour come for free, which a restyled `<div>` would have thrown
 * away.
 *
 * The knob subtracts its own width as it travels so it stays inside the track
 * at both ends - the same trick the Slider knob uses.
 */
export function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  /** Shown behind a help icon, not inline - panels stay controls-first. */
  hint?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="mb-3 flex items-center justify-between gap-3">
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="truncate text-[12px] text-primary">{label}</span>
        {hint && <HelpTip text={hint} />}
      </span>

      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        onClick={() => onChange(!checked)}
        className={`relative h-5.5 w-9.5 shrink-0 cursor-pointer rounded-full
                    p-0.75 transition-colors duration-200 ${
                      checked ? "bg-tool-active" : "bg-tertiary/40"
                    }`}
      >
        <span
          className={`block h-4 w-4 rounded-full bg-white shadow-[0_1px_2px_rgba(0,0,0,0.3)]
                      transition-transform duration-200 ${
                        checked ? "translate-x-4" : "translate-x-0"
                      }`}
        />
      </button>
    </div>
  );
}

/** A labelled group of controls, with optional hover help on the heading. */
export function Group({
  title,
  help,
  children,
}: {
  title: string;
  /** Shown behind a help icon next to the heading. */
  help?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-5">
      <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
        {title}
        {help && <HelpTip text={help} />}
      </h3>
      {children}
    </section>
  );
}
