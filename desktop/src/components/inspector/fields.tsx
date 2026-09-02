// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";

import { Icon } from "../Icon";
import { clamp, cx, decimalsOf, roundTo, useElementSize, usePointerDrag } from "./base";
import styles from "./inspector.module.css";

/**
 * The three value pickers: a scrubbable number, a listbox, and a segmented
 * row. Ported from the wc-ui-rnd design study.
 *
 * All three take the app's editing contract rather than the study's plain
 * `onChange`: a gesture echoes live and *commits* once, so undo undoes the
 * drag instead of sixty frames of it. Discrete controls - the listbox, the
 * segments - change and commit in the same tick, which is why only the number
 * field carries an explicit `onCommit`.
 */

/* ── number field ─────────────────────────────────────────────────────────── */

export function NumberField({
  value,
  onChange,
  onCommit,
  min = Number.NEGATIVE_INFINITY,
  max = Number.POSITIVE_INFINITY,
  step = 1,
  /** Drag distance in px that advances the value by one `step`. */
  pxPerStep = 2,
  "aria-label": ariaLabel,
}: {
  value: number;
  onChange: (value: number) => void;
  onCommit?: () => void;
  min?: number;
  max?: number;
  step?: number;
  pxPerStep?: number;
  "aria-label"?: string;
}) {
  const digits = decimalsOf(step);
  const input = useRef<HTMLInputElement>(null);
  const origin = useRef(value);
  const reverting = useRef(false);
  // Null means "showing the value"; a string means "being typed into", and
  // the project is not touched until that entry commits.
  const [draft, setDraft] = useState<string | null>(null);
  const editing = draft !== null;

  const shown = roundTo(value, Math.max(digits, 3));

  const settle = (next: number, fine: boolean) =>
    onChange(roundTo(clamp(next, min, max), fine ? digits + 1 : digits));

  const beginDrag = usePointerDrag({
    disabled: editing,
    cursor: "ew-resize",
    onStart: () => {
      origin.current = value;
    },
    onMove: ({ dx, shiftKey, altKey, moved }) => {
      if (!moved) return;
      // Shift coarsens, Alt refines - the same grammar the fader's arrow keys
      // use, so one modifier means one thing across the panel.
      const scale = shiftKey ? 10 : altKey ? 0.1 : 1;
      settle(origin.current + (dx / pxPerStep) * step * scale, altKey);
    },
    onEnd: ({ moved }) => {
      // A press that never travelled is a click: fall through to text entry.
      if (moved) onCommit?.();
      else input.current?.focus();
    },
  });

  const commitTyped = (raw: string) => {
    const parsed = Number.parseFloat(raw.replace(",", ".").replace(/[^\d.eE+-]/g, ""));
    if (Number.isFinite(parsed)) {
      onChange(roundTo(clamp(parsed, min, max), Math.max(digits, 3)));
      onCommit?.();
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const scale = event.shiftKey ? 10 : event.altKey ? 0.1 : 1;
      const direction = event.key === "ArrowUp" ? 1 : -1;
      const typed = draft !== null ? Number.parseFloat(draft) : Number.NaN;
      const base = Number.isFinite(typed) ? typed : value;
      const next = roundTo(
        clamp(base + direction * step * scale, min, max),
        event.altKey ? digits + 1 : digits,
      );
      onChange(next);
      onCommit?.();
      if (draft !== null) setDraft(String(next));
    } else if (event.key === "Enter") {
      event.preventDefault();
      input.current?.blur();
    } else if (event.key === "Escape") {
      event.preventDefault();
      reverting.current = true;
      input.current?.blur();
    }
  };

  return (
    <div
      className={cx(styles.field, editing && styles.editing)}
      onPointerDown={(event: ReactPointerEvent<HTMLDivElement>) => {
        if (editing) return;
        event.preventDefault(); // withhold focus until the gesture resolves
        beginDrag(event);
      }}
    >
      <input
        ref={input}
        className={styles.fieldInput}
        value={draft ?? String(shown)}
        spellCheck={false}
        autoComplete="off"
        inputMode="decimal"
        role="spinbutton"
        aria-label={ariaLabel}
        aria-valuenow={value}
        aria-valuemin={Number.isFinite(min) ? min : undefined}
        aria-valuemax={Number.isFinite(max) ? max : undefined}
        onFocus={() => setDraft(String(shown))}
        onBlur={() => {
          if (draft !== null && !reverting.current) commitTyped(draft);
          reverting.current = false;
          setDraft(null);
        }}
        onKeyDown={onKeyDown}
        onChange={(event) => setDraft(event.target.value)}
      />
    </div>
  );
}

/* ── select ───────────────────────────────────────────────────────────────── */

export interface SelectOption<T> {
  value: T;
  label: string;
  /** Heading this option sits under. Consecutive options share one heading. */
  group?: string;
  /** Rendered after the label in a dimmer voice, e.g. "(file missing)". */
  note?: string;
  /** The face to draw the label in, for a font picker that shows its fonts. */
  fontFamily?: string;
}

const TYPEAHEAD_WINDOW = 600;
const OPEN_KEYS = ["ArrowDown", "ArrowUp", "Enter", " "];

/**
 * A listbox, not a native `<select>`.
 *
 * The native control cannot show a font in its own face, group with the app's
 * own headings, or be styled to match anything around it. This is keyboard
 * complete - arrows, Home/End, typeahead, Escape - and flips above the
 * trigger when there is no room below it, which in a right-hand panel there
 * often is not.
 */
export function Select<T extends string | number>({
  options,
  value,
  onChange,
  "aria-label": ariaLabel,
}: {
  options: readonly SelectOption<T>[];
  value: T;
  onChange: (value: T) => void;
  "aria-label"?: string;
}) {
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const typeahead = useRef({ query: "", at: 0 });

  const [box, setBox] = useState<{ left: number; top: number; width: number } | null>(null);
  const [active, setActive] = useState(0);
  const open = box !== null;

  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const close = (refocus: boolean) => {
    setBox(null);
    if (refocus) trigger.current?.focus();
  };

  const openList = () => {
    const rect = trigger.current?.getBoundingClientRect();
    if (!rect) return;
    setActive(selectedIndex >= 0 ? selectedIndex : 0);
    // Opened downwards from the trigger; the layout effect below flips it up
    // before paint if the list turns out not to fit there.
    setBox({ left: rect.left, top: rect.bottom + 4, width: rect.width });
  };

  const commit = (index: number) => {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    close(true);
  };

  // Correct the placement and hand the list focus once it is in the DOM but
  // before paint, so a list that has to open upwards never flashes downwards.
  useLayoutEffect(() => {
    if (!open) return;
    const anchor = trigger.current;
    const element = list.current;
    if (!anchor || !element) return;
    const rect = anchor.getBoundingClientRect();
    const height = element.offsetHeight;
    const up = rect.bottom + height + 8 > window.innerHeight && rect.top > height + 8;
    const top = up ? rect.top - height - 4 : rect.bottom + 4;
    setBox((current) =>
      current && current.top === top ? current : { left: rect.left, top, width: rect.width },
    );
    element.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!root.current?.contains(target) && !list.current?.contains(target)) setBox(null);
    };
    // The list is placed against the trigger's box, so anything that moves the
    // trigger - the panel scrolling, the window resizing - orphans it. Closing
    // is the honest response; a list that follows would still be a list nobody
    // meant to leave open.
    const dismiss = () => setBox(null);
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    list.current?.querySelector<HTMLElement>('[data-active="true"]')?.scrollIntoView({
      block: "nearest",
    });
  }, [open, active]);

  const onListKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    const last = options.length - 1;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setActive((current) => (current >= last ? 0 : current + 1));
        return;
      case "ArrowUp":
        event.preventDefault();
        setActive((current) => (current <= 0 ? last : current - 1));
        return;
      case "Home":
        event.preventDefault();
        setActive(0);
        return;
      case "End":
        event.preventDefault();
        setActive(last);
        return;
      case "Enter":
      case " ":
        event.preventDefault();
        commit(active);
        return;
      case "Escape":
        event.preventDefault();
        close(true);
        return;
      case "Tab":
        close(false);
        return;
      default:
        break;
    }

    // Typeahead: printable keys jump to the first option with that prefix.
    if (event.key.length !== 1 || event.metaKey || event.ctrlKey) return;
    const now = Date.now();
    const state = typeahead.current;
    state.query = now - state.at > TYPEAHEAD_WINDOW ? event.key : state.query + event.key;
    state.at = now;
    const query = state.query.toLowerCase();
    const match = options.findIndex((option) => option.label.toLowerCase().startsWith(query));
    if (match >= 0) setActive(match);
  };

  return (
    <div ref={root} className={styles.select}>
      <button
        ref={trigger}
        type="button"
        className={cx(styles.trigger, open && styles.open)}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? close(true) : openList())}
        onKeyDown={(event) => {
          if (OPEN_KEYS.includes(event.key)) {
            event.preventDefault();
            openList();
          }
        }}
      >
        <span className={styles.triggerLabel} style={{ fontFamily: selected?.fontFamily }}>
          {selected?.label ?? String(value)}
        </span>
        <Icon name="chevronDown" size={12} className={styles.chevron} />
      </button>

      {box &&
        createPortal(
        <ul
          ref={list}
          role="listbox"
          tabIndex={-1}
          aria-label={ariaLabel}
          aria-activedescendant={`${id}-option-${active}`}
          className={cx(styles.vars, styles.list)}
          style={{ left: box.left, top: box.top, minWidth: box.width }}
          onKeyDown={onListKeyDown}
        >
          {options.map((option, index) => (
            <li key={String(option.value)}>
              {option.group !== undefined && option.group !== options[index - 1]?.group && (
                <div className={styles.optionGroup} aria-hidden>
                  {option.group}
                </div>
              )}
              <div
                id={`${id}-option-${index}`}
                role="option"
                aria-selected={option.value === value}
                data-active={index === active}
                className={styles.option}
                onPointerEnter={() => setActive(index)}
                onClick={() => commit(index)}
              >
                <span className={styles.optionLabel} style={{ fontFamily: option.fontFamily }}>
                  {option.label}
                  {option.note ? ` ${option.note}` : ""}
                </span>
                {option.value === value && (
                  <Icon name="check" size={12} className={styles.check} />
                )}
              </div>
            </li>
          ))}
        </ul>,
          document.body,
        )}
    </div>
  );
}

/* ── segmented control ────────────────────────────────────────────────────── */

/** Single-select row with a thumb that slides to the active segment. */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  "aria-label": ariaLabel,
}: {
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  "aria-label"?: string;
}) {
  const [ref, size] = useElementSize<HTMLDivElement>();
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);
  const [thumb, setThumb] = useState<{ left: number; width: number } | null>(null);
  const index = options.findIndex((option) => option.value === value);

  useLayoutEffect(() => {
    const element = index >= 0 ? buttons.current[index] : null;
    const next = element ? { left: element.offsetLeft, width: element.offsetWidth } : null;
    // Same geometry, same object: `options` is rebuilt per render wherever its
    // labels are translated, so re-running this effect must not restart it.
    setThumb((current) =>
      current?.left === next?.left && current?.width === next?.width ? current : next,
    );
  }, [index, options, size.width]);

  const move = (direction: 1 | -1) => {
    const count = options.length;
    const next = ((index < 0 ? 0 : index) + direction + count) % count;
    onChange(options[next].value);
    buttons.current[next]?.focus();
  };

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={ariaLabel}
      className={styles.segGroup}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight" || event.key === "ArrowDown") {
          event.preventDefault();
          move(1);
        } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
          event.preventDefault();
          move(-1);
        }
      }}
    >
      {thumb && (
        <span
          className={styles.segThumb}
          aria-hidden
          style={{ transform: `translateX(${thumb.left}px)`, width: thumb.width }}
        />
      )}
      {options.map((option, position) => {
        const on = option.value === value;
        return (
          <button
            key={option.value}
            ref={(element) => {
              buttons.current[position] = element;
            }}
            type="button"
            role="radio"
            aria-checked={on}
            // Roving tabindex keeps the group a single tab stop.
            tabIndex={on || (index < 0 && position === 0) ? 0 : -1}
            className={cx(styles.segment, on && styles.segmentOn)}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
