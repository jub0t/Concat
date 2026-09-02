// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Turning a title into pixels for the exporter.
 *
 * The engine composites RGBA frames; it knows nothing about fonts. So a text
 * clip goes into an export as a full-frame transparent PNG, drawn here at the
 * exact output size with the same rules `textCss` gives the preview overlay -
 * same fraction-of-frame-height sizing, same stroke-behind-fill, same plate.
 * One PNG per title, full frame rather than tight bounds, so the clip's
 * offsets are already baked in and the exporter places it edge to edge with
 * an identity transform.
 */

import type { TextStyle } from "./text";

/** Mirrors `textCss`'s shadow: offset 4% of the size, blur 12%. */
const SHADOW = { offset: 0.04, blur: 0.12, color: "rgba(0,0,0,0.55)" };
/** Mirrors `textCss`'s plate: padding 20%/35% of the size, radius 12%. */
const PLATE = { padY: 0.2, padX: 0.35, radius: 0.12 };

/**
 * Draws one title into a `width` x `height` transparent PNG.
 *
 * Known divergence from the DOM: the preview wraps a line longer than 92% of
 * the frame, canvas does not - a title that long is already unreadable, so
 * explicit line breaks are the layout and overflow just runs off the frame.
 */
export async function rasterizeTitle(
  style: TextStyle,
  offsetX: number,
  offsetY: number,
  width: number,
  height: number,
): Promise<Uint8Array> {
  const size = style.fontSize * height;
  const font = `${style.italic ? "italic " : ""}${style.fontWeight} ${size}px ${style.fontFamily}`;

  // Make sure the face is actually resident before measuring: a fallback font
  // would bake the wrong glyphs into the file, which is worse than a slow
  // start. Best-effort - an unknown family falls back exactly like CSS does.
  await document.fonts.load(font, style.content).catch(() => undefined);

  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("no 2d canvas for the title rasteriser");

  context.font = font;
  context.textBaseline = "middle";
  // `letterSpacing` is newer canvas API; where it is missing the export just
  // loses tracking, matching nothing worse than an older browser.
  if ("letterSpacing" in context) context.letterSpacing = `${style.tracking * size}px`;

  const lines = style.content.split("\n");
  const lineHeight = style.lineHeight * size;
  const blockWidth = Math.max(...lines.map((line) => context.measureText(line).width), 1);
  const blockHeight = lines.length * lineHeight;

  const centerX = width / 2 + offsetX * width;
  const centerY = height / 2 + offsetY * height;

  context.globalAlpha = style.opacity;

  if (style.background) {
    const padX = size * PLATE.padX;
    const padY = size * PLATE.padY;
    context.fillStyle = style.background;
    context.beginPath();
    context.roundRect(
      centerX - blockWidth / 2 - padX,
      centerY - blockHeight / 2 - padY,
      blockWidth + padX * 2,
      blockHeight + padY * 2,
      size * PLATE.radius,
    );
    context.fill();
  }

  // The shadow goes on after the plate: `textCss` shadows the letters, never
  // the plate behind them.
  if (style.shadow) {
    context.shadowColor = SHADOW.color;
    context.shadowOffsetY = size * SHADOW.offset;
    context.shadowBlur = size * SHADOW.blur;
  }

  context.textAlign = style.align;
  const alignX =
    style.align === "left"
      ? centerX - blockWidth / 2
      : style.align === "right"
        ? centerX + blockWidth / 2
        : centerX;

  for (const [index, line] of lines.entries()) {
    const y = centerY - blockHeight / 2 + (index + 0.5) * lineHeight;
    // Stroke first, fill over it - the canvas spelling of
    // `paint-order: stroke fill`, so an outline thickens outwards instead of
    // eating into the letterforms.
    if (style.strokeWidth > 0) {
      context.strokeStyle = style.strokeColor;
      context.lineWidth = style.strokeWidth * size;
      context.lineJoin = "round";
      context.strokeText(line, alignX, y);
    }
    context.fillStyle = style.color;
    context.fillText(line, alignX, y);
  }

  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (result) => (result ? resolve(result) : reject(new Error("title PNG encode failed"))),
      "image/png",
    );
  });
  return new Uint8Array(await blob.arrayBuffer());
}
