// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import {
  AdjustmentsHorizontalIcon,
  ArrowDownTrayIcon,
  ArrowUpTrayIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ChevronUpIcon,
  DocumentDuplicateIcon,
  EyeIcon,
  EyeSlashIcon,
  FilmIcon,
  FolderIcon,
  InformationCircleIcon,
  LockClosedIcon,
  MagnifyingGlassIcon,
  MinusIcon,
  MoonIcon,
  MusicalNoteIcon,
  PhotoIcon,
  PlusIcon,
  QuestionMarkCircleIcon,
  SparklesIcon,
  SpeakerWaveIcon,
  SpeakerXMarkIcon,
  SunIcon,
  TrashIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";
import {
  AudioLines,
  Blend,
  Hand,
  Magnet,
  Maximize,
  Merge,
  MousePointer2,
  Pause,
  Play,
  ScissorsLineDashed,
  SkipBack,
  SkipForward,
  SquareSplitHorizontal,
  StepBack,
  StepForward,
  Type,
} from "lucide-react";

import type { ReactNode } from "react";

/**
 * The icon set: Heroicons for everything generic, Lucide for the editor's
 * vocabulary, hand-authored glyphs for the few marks nobody draws.
 *
 * The generic half (folders, chevrons, trash, eyes) comes from Heroicons;
 * transport, tools, cut and merge come from Lucide, whose 24px stroke-2
 * grid matches the app's exactly. Both are MIT and tree-shaken to only the
 * imports named below, so nothing ships that is not used. Hand-made glyphs
 * remain only for the template slot and the Windows title-bar chrome.
 *
 * Custom glyphs are 24x24, stroked with `currentColor` at width 2, round
 * caps. Add to the LUCIDE or HERO map when a library has the right glyph;
 * draw on the same grid only when none does.
 */

const GLYPHS = {
  // ── transport ──────────────────────────────────────────────────────────
  play: <path d="M8 5.2v13.6l11-6.8z" />,
  pause: (
    <>
      <path d="M9 5v14" />
      <path d="M15 5v14" />
    </>
  ),
  skipStart: (
    <>
      <path d="M6 5v14" />
      <path d="M19 5.5v13l-10-6.5z" />
    </>
  ),
  skipEnd: (
    <>
      <path d="M18 5v14" />
      <path d="M5 5.5v13l10-6.5z" />
    </>
  ),
  stepBack: (
    <>
      <path d="M5 5v14" />
      <path d="m16 6-7 6 7 6z" />
    </>
  ),
  stepForward: (
    <>
      <path d="M19 5v14" />
      <path d="m8 6 7 6-7 6z" />
    </>
  ),

  // ── tools ──────────────────────────────────────────────────────────────
  // arrow pointer
  select: (
    <>
      <path d="m4 3 7 17 2.5-6.5L20 11z" />
    </>
  ),
  // razor: two blades meeting at a cut line
  razor: (
    <>
      <circle cx="6" cy="18" r="2.5" />
      <circle cx="18" cy="18" r="2.5" />
      <path d="M7.8 16.2 18 4" />
      <path d="M16.2 16.2 6 4" />
    </>
  ),
  hand: (
    <>
      <path d="M9 11V5.5a1.5 1.5 0 0 1 3 0V11" />
      <path d="M12 11V4.5a1.5 1.5 0 0 1 3 0V11" />
      <path d="M15 11V6.5a1.5 1.5 0 0 1 3 0V15a6 6 0 0 1-6 6h-1a7 7 0 0 1-7-7v-2a1.5 1.5 0 0 1 3 0v1" />
    </>
  ),
  // two blocks parted by a cut line
  split: (
    <>
      <path d="M12 3v18" strokeDasharray="3 3" />
      <path d="M8 7H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h3" />
      <path d="M16 7h3a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-3" />
    </>
  ),
  // two blocks closing on a seam
  merge: (
    <>
      <path d="M12 3v18" />
      <path d="M3 7h5a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H3" />
      <path d="M21 7h-5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h5" />
    </>
  ),
  // horseshoe magnet, for snapping
  magnet: (
    <>
      <path d="M6 15V8a6 6 0 0 1 12 0v7" />
      <path d="M6 15a3 3 0 0 0 6 0V9" />
      <path d="M18 15a3 3 0 0 1-6 0V9" />
    </>
  ),

  // ── media kinds ────────────────────────────────────────────────────────
  film: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M7 4v16" />
      <path d="M17 4v16" />
      <path d="M3 12h18" />
    </>
  ),
  music: (
    <>
      <path d="M9 18V5l11-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="17" cy="16" r="3" />
    </>
  ),
  // framed picture with a horizon and a sun
  image: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <circle cx="8.5" cy="9.5" r="1.5" />
      <path d="m3 17 5-4 4 3 3-2 6 5" />
    </>
  ),
  // a serif capital T: the universal mark for type
  type: (
    <>
      <path d="M4 6.5V5h16v1.5" />
      <path d="M12 5v14" />
      <path d="M8.5 19h7" />
    </>
  ),
  // waveform bars
  waveform: (
    <>
      <path d="M3 12v1" />
      <path d="M7 8v8" />
      <path d="M11 5v14" />
      <path d="M15 9v6" />
      <path d="M19 11v2" />
    </>
  ),

  // ── actions ────────────────────────────────────────────────────────────
  plus: <path d="M12 5v14M5 12h14" />,
  minus: <path d="M5 12h14" />,
  close: <path d="M18 6 6 18M6 6l12 12" />,
  trash: (
    <>
      <path d="M3 6h18" />
      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
  // arrow down into a tray
  import: (
    <>
      <path d="M12 3v12" />
      <path d="m7 10 5 5 5-5" />
      <path d="M3 15v4a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-4" />
    </>
  ),
  // arrow up out of a tray
  export: (
    <>
      <path d="M12 15V3" />
      <path d="m7 8 5-5 5 5" />
      <path d="M3 15v4a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-4" />
    </>
  ),
  folder: (
    <path d="M4 20a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h4.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.7.9H20a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2Z" />
  ),

  // ── track state ────────────────────────────────────────────────────────
  eye: (
    <>
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  eyeOff: (
    <>
      <path d="M10.7 5.1A10.9 10.9 0 0 1 12 5c6.5 0 10 7 10 7a18 18 0 0 1-2.6 3.5" />
      <path d="M6.6 6.6A18 18 0 0 0 2 12s3.5 7 10 7a10.7 10.7 0 0 0 5.4-1.4" />
      <path d="m2 2 20 20" />
    </>
  ),
  volume: (
    <>
      <path d="M11 5 6 9H3v6h3l5 4z" />
      <path d="M16 8.5a4.5 4.5 0 0 1 0 7" />
      <path d="M19 5.5a8.5 8.5 0 0 1 0 13" />
    </>
  ),
  volumeOff: (
    <>
      <path d="M11 5 6 9H3v6h3l5 4z" />
      <path d="m16 9 5 6M21 9l-5 6" />
    </>
  ),
  lock: (
    <>
      <rect x="4" y="10" width="16" height="11" rx="2" />
      <path d="M8 10V7a4 4 0 0 1 8 0v3" />
    </>
  ),

  // ── window controls ────────────────────────────────────────────────────
  // Drawn at width 1.5 visually by being thinner shapes; Windows convention.
  winMinimize: <path d="M5 12h14" />,
  winMaximize: <rect x="5" y="5" width="14" height="14" rx="1.5" />,
  winRestore: (
    <>
      <rect x="4" y="8" width="12" height="12" rx="1.5" />
      <path d="M8 8V5.5A1.5 1.5 0 0 1 9.5 4h9A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H16" />
    </>
  ),

  // ── chrome ─────────────────────────────────────────────────────────────
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M2 12h2M20 12h2" />
      <path d="m4.93 4.93 1.41 1.41M17.66 17.66l1.41 1.41M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
    </>
  ),
  moon: <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z" />,
  check: <path d="m4 12.5 5 5 11-11" />,
  copy: (
    <>
      <rect x="9" y="9" width="12" height="12" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>
  ),
  chevronDown: <path d="m6 9 6 6 6-6" />,
  chevronUp: <path d="m6 15 6-6 6 6" />,
  chevronRight: <path d="m9 6 6 6-6 6" />,
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </>
  ),
  // settings: three sliders with staggered knobs. Deliberately not a cog:
  // the old ring-and-rays cog was a sun in all but name, and it sat right
  // next to the actual sun of the theme toggle.
  settings: (
    <>
      <path d="M4 6.5h16M4 12h16M4 17.5h16" />
      <circle cx="9.5" cy="6.5" r="2" fill="currentColor" stroke="none" />
      <circle cx="15" cy="12" r="2" fill="currentColor" stroke="none" />
      <circle cx="7.5" cy="17.5" r="2" fill="currentColor" stroke="none" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 16v-5" />
      <path d="M12 8h.01" />
    </>
  ),
  help: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M9.4 9.3a2.7 2.7 0 1 1 3.6 2.6c-.7.3-1 .8-1 1.6v.3" />
      <path d="M12 17h.01" />
    </>
  ),
  // a four-point star with a smaller companion, for "effects"
  // slot: a dashed placeholder box - the frame a template fills in later
  slot: (
    <>
      <rect x="3.5" y="5.5" width="17" height="13" rx="2.5" strokeDasharray="3.2 2.6" />
      <path d="M12 9.5v5M9.5 12h5" />
    </>
  ),
  sparkles: (
    <>
      <path d="M10 3.5l1.7 4.3 4.3 1.7-4.3 1.7L10 15.5l-1.7-4.3L4 9.5l4.3-1.7z" />
      <path d="M17.5 14l1 2.5 2.5 1-2.5 1-1 2.5-1-2.5-2.5-1 2.5-1z" />
    </>
  ),
  // a frame split on the diagonal, for "transitions"
  transition: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="M4.8 19.2 19.2 4.8" />
    </>
  ),
  // crossed arrows, for "fit to window"
  fit: (
    <>
      <path d="M3 8V5a2 2 0 0 1 2-2h3" />
      <path d="M16 3h3a2 2 0 0 1 2 2v3" />
      <path d="M21 16v3a2 2 0 0 1-2 2h-3" />
      <path d="M8 21H5a2 2 0 0 1-2-2v-3" />
    </>
  ),
} satisfies Record<string, ReactNode>;

/** Every glyph the app can draw. A typo is a compile error. */
export type IconName = keyof typeof GLYPHS;

/**
 * Heroicons stand in for every generic glyph - drawn by people who draw
 * icons for a living. The hand-made set below remains for what no library
 * carries: the editor's own vocabulary (razor, slot, waveform, transport,
 * window controls). Same names, same call sites; only the pixels changed.
 */
const HERO: Partial<Record<IconName, typeof PlusIcon>> = {
  plus: PlusIcon,
  minus: MinusIcon,
  close: XMarkIcon,
  trash: TrashIcon,
  import: ArrowDownTrayIcon,
  export: ArrowUpTrayIcon,
  folder: FolderIcon,
  eye: EyeIcon,
  eyeOff: EyeSlashIcon,
  volume: SpeakerWaveIcon,
  volumeOff: SpeakerXMarkIcon,
  lock: LockClosedIcon,
  sun: SunIcon,
  moon: MoonIcon,
  check: CheckIcon,
  copy: DocumentDuplicateIcon,
  chevronDown: ChevronDownIcon,
  chevronUp: ChevronUpIcon,
  chevronRight: ChevronRightIcon,
  search: MagnifyingGlassIcon,
  settings: AdjustmentsHorizontalIcon,
  info: InformationCircleIcon,
  help: QuestionMarkCircleIcon,
  sparkles: SparklesIcon,
  film: FilmIcon,
  music: MusicalNoteIcon,
  image: PhotoIcon,
};

/**
 * Lucide covers the editor vocabulary Heroicons lacks - transport, tools,
 * cut/merge - on the same 24px stroke-2 grid the custom glyphs use, so the
 * two faces sit together without adjustment. What stays hand-drawn is only
 * what no library carries: the template slot and the Windows chrome.
 */
const LUCIDE: Partial<Record<IconName, typeof Play>> = {
  play: Play,
  pause: Pause,
  skipStart: SkipBack,
  skipEnd: SkipForward,
  stepBack: StepBack,
  stepForward: StepForward,
  select: MousePointer2,
  razor: ScissorsLineDashed,
  hand: Hand,
  split: SquareSplitHorizontal,
  merge: Merge,
  magnet: Magnet,
  type: Type,
  waveform: AudioLines,
  // Two discs blending into one another: the cross-fade itself.
  transition: Blend,
  fit: Maximize,
};

export function Icon({
  name,
  size = 16,
  className,
  strokeWidth = 2,
}: {
  name: IconName;
  size?: number;
  className?: string;
  strokeWidth?: number;
}) {
  const Lucide = LUCIDE[name];
  if (Lucide) {
    // Lucide is authored at the same 24px stroke-2 as the custom glyphs, so
    // the requested stroke passes straight through.
    return (
      <Lucide size={size} strokeWidth={strokeWidth} aria-hidden className={className} />
    );
  }
  const Hero = HERO[name];
  if (Hero) {
    return (
      <Hero
        width={size}
        height={size}
        // Heroicons are drawn at 1.5; scaled towards the app's 2 so they
        // sit next to the custom glyphs without reading as a lighter face.
        strokeWidth={Math.max(1.6, strokeWidth - 0.2)}
        aria-hidden="true"
        className={className}
      />
    );
  }
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      {GLYPHS[name]}
    </svg>
  );
}

/**
 * The standard square icon button.
 *
 * Active means a lime glyph on a slightly darker surface - quieter than a
 * filled accent square, and the same lime the timeline uses for selection, so
 * "this is on" reads as one colour everywhere. Transparent when idle.
 */
export function IconButton({
  icon,
  label,
  onClick,
  active = false,
  disabled = false,
  size = 9,
  tone = "default",
}: {
  icon: IconName;
  label: string;
  onClick?: () => void;
  active?: boolean;
  disabled?: boolean;
  /** Height and width in Tailwind spacing units. */
  size?: 7 | 9;
  tone?: "default" | "danger" | "go";
}) {
  // "go" idles in the same lime as active states, not the success green:
  // the transport's play button is the one "go" and a green glyph beside
  // lime-active tools read as two systems.
  const idle =
    tone === "danger"
      ? "text-danger hover:bg-hover"
      : tone === "go"
        ? "text-tool-active hover:bg-hover"
        : "text-primary hover:bg-hover";

  // Active is the one lime everywhere - including the transport's play
  // button, which used to fill green and read as a different system.
  const on = "bg-tool-active-bg text-tool-active";

  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={`flex ${size === 9 ? "h-9 w-9" : "h-7 w-7"} shrink-0 cursor-pointer
                  items-center justify-center rounded-lg transition-colors duration-150
                  disabled:cursor-not-allowed disabled:opacity-30
                  ${active ? on : idle}`}
    >
      <Icon name={icon} size={size === 9 ? 16 : 14} />
    </button>
  );
}
