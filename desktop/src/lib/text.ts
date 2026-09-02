// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Text overlays.
 *
 * A text clip has no media file behind it - it *is* its own content - which is
 * why the style lives on the clip rather than in the bin. Everything is stored
 * relative to the frame rather than in pixels, so a title composed against a
 * 1080p project still lands correctly when exported at 4K.
 */

import { t } from "./i18n";

export interface TextStyle {
  content: string;
  /** CSS family name. See `FONTS`. */
  fontFamily: string;
  /**
   * Cap height as a fraction of frame height.
   *
   * Relative, not points: a 48pt title means something different on a 720p
   * frame than a 4K one, and the project can be either.
   */
  fontSize: number;
  fontWeight: number;
  italic: boolean;
  color: string;
  align: "left" | "center" | "right";
  /** 0..1. */
  opacity: number;
  /** Outline width as a fraction of the font size. Zero for none. */
  strokeWidth: number;
  strokeColor: string;
  /** A soft drop shadow, which is what makes text readable over footage. */
  shadow: boolean;
  /** A solid plate behind the text. Empty string for none. */
  background: string;
  /** Extra space between lines, as a multiple of the font size. */
  lineHeight: number;
  /** Letter spacing as a fraction of the font size. */
  tracking: number;
}

/**
 * The fonts on offer.
 *
 * The two bundled faces are guaranteed - they ship with the app, so a project
 * opened on another machine looks the same. The system families below them are
 * a convenience and may fall back to something else elsewhere, which is why
 * they are grouped separately in the picker.
 */
export const FONTS: { label: string; value: string; bundled: boolean }[] = [
  { label: "Cabinet Grotesk", value: '"Cabinet Grotesk"', bundled: true },
  { label: "Synonym", value: '"Synonym"', bundled: true },
  { label: "System sans", value: "system-ui, sans-serif", bundled: false },
  { label: "Georgia", value: "Georgia, serif", bundled: false },
  { label: "Times", value: '"Times New Roman", serif', bundled: false },
  { label: "Courier", value: '"Courier New", monospace', bundled: false },
  { label: "Impact", value: "Impact, sans-serif", bundled: false },
  { label: "Comic Sans", value: '"Comic Sans MS", cursive', bundled: false },
];

/**
 * A font the user added from disk.
 *
 * The path is what gets saved, not the bytes: a project file that embedded
 * every face would grow without limit, and the same font on the same machine
 * is the overwhelmingly common case. Reopening a project on a machine that
 * lacks the file falls back to the default face rather than failing to open,
 * which is why `missing` is tracked rather than thrown.
 */
export interface CustomFont {
  /** The CSS family name, derived from the file name and made unique. */
  family: string;
  path: string;
  /** True once a load attempt has failed - shown in the picker, not fatal. */
  missing?: boolean;
}

/** Families already handed to the document, so reopening does not re-register. */
const registered = new Set<string>();

/**
 * Turns a font file into a usable CSS family.
 *
 * `FontFace` accepts the raw bytes, so this works for any face the webview can
 * parse - ttf, otf, woff, woff2 - without installing anything system-wide.
 * Registration is idempotent: the same family loaded twice is a no-op, which
 * matters because opening a project re-runs this for every font it names.
 */
export async function registerFont(
  font: CustomFont,
  read: (path: string) => Promise<ArrayBuffer>,
): Promise<boolean> {
  if (registered.has(font.family)) return true;

  try {
    const face = new FontFace(font.family, await read(font.path));
    await face.load();
    document.fonts.add(face);
    registered.add(font.family);
    return true;
  } catch {
    // A missing or unparseable file must not stop a project opening. The
    // picker shows the face as unavailable and text falls back to the default.
    return false;
  }
}

/**
 * A family name for a font file: its own name, made unique against what is
 * already loaded.
 *
 * Two different files called `Regular.otf` in different folders would
 * otherwise collide, and the second would silently render as the first.
 */
export function familyForPath(path: string, taken: readonly string[]): string {
  const base = (path.split(/[\\/]/).pop() ?? path).replace(/\.[a-z0-9]+$/i, "");
  const cleaned = base.replace(/[^\w\s-]/g, "").trim() || "Custom font";

  if (!taken.includes(cleaned)) return cleaned;
  for (let n = 2; ; n += 1) {
    const candidate = `${cleaned} ${n}`;
    if (!taken.includes(candidate)) return candidate;
  }
}

export const WEIGHTS: { label: string; value: number }[] = [
  { get label() { return t("text.weight.light"); }, value: 300 },
  { get label() { return t("text.weight.regular"); }, value: 400 },
  { get label() { return t("text.weight.medium"); }, value: 500 },
  { get label() { return t("text.weight.bold"); }, value: 700 },
  { get label() { return t("text.weight.black"); }, value: 900 },
];

export function defaultTextStyle(): TextStyle {
  return {
    content: "Your text",
    fontFamily: '"Cabinet Grotesk"',
    // Roughly a tenth of the frame: big enough to read on a phone, small
    // enough not to fill the screen before anyone has touched it.
    fontSize: 0.09,
    fontWeight: 700,
    italic: false,
    color: "#ffffff",
    align: "center",
    opacity: 1,
    strokeWidth: 0,
    strokeColor: "#000000",
    shadow: true,
    background: "",
    lineHeight: 1.2,
    tracking: 0,
  };
}

/**
 * The CSS for one text clip, given the height of the surface it is drawn on.
 *
 * Shared by the preview overlay and the export rasteriser, so a title cannot
 * look one way on screen and another in the file.
 */
export function textCss(style: TextStyle, surfaceHeight: number): React.CSSProperties {
  const size = style.fontSize * surfaceHeight;

  const shadow = style.shadow
    ? `0 ${(size * 0.04).toFixed(2)}px ${(size * 0.12).toFixed(2)}px rgba(0,0,0,0.55)`
    : undefined;

  return {
    fontFamily: style.fontFamily,
    fontSize: `${size}px`,
    fontWeight: style.fontWeight,
    fontStyle: style.italic ? "italic" : "normal",
    color: style.color,
    textAlign: style.align,
    opacity: style.opacity,
    lineHeight: style.lineHeight,
    letterSpacing: `${style.tracking * size}px`,
    textShadow: shadow,
    // `paint-order` puts the stroke behind the fill, so an outline thickens
    // outwards instead of eating into the letterforms.
    WebkitTextStrokeWidth: style.strokeWidth > 0 ? `${style.strokeWidth * size}px` : undefined,
    WebkitTextStrokeColor: style.strokeColor,
    paintOrder: "stroke fill",
    backgroundColor: style.background || undefined,
    padding: style.background ? `${size * 0.2}px ${size * 0.35}px` : undefined,
    borderRadius: style.background ? `${size * 0.12}px` : undefined,
    whiteSpace: "pre-wrap",
  };
}
