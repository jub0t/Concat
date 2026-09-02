// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The video effect and transition catalogues.
 *
 * Every effect carries two renderings of one idea:
 *
 * - `chain` builds the FFmpeg video filter fragment the *export* runs - the
 *   same design as `lib/filters.ts` for audio, and the ground truth for
 *   pixels.
 * - `preview` says how the *monitor* shows it live: a CSS filter where CSS is
 *   faithful, an SVG filter where it needs a matrix or a convolution, an
 *   overlay layer, a canvas redraw for geometry, or a deterministic jitter.
 *   Every effect previews in real time; the export is only ever a higher
 *   quality version of what was already on screen.
 *
 * Ids are forever: they are written into project files, so renaming one later
 * would orphan clips. Pick them like you mean them. Labels and blurbs are
 * display-only and read from the message catalog as getters, so the id is the
 * only identity anything stores.
 */

import { t } from "./i18n";
import type { FilterParam } from "./filters";

/**
 * How one effect draws itself in the monitor.
 *
 * `scale` on the builders is preview pixels per export pixel, so a 10px blur
 * looks the same on a small monitor as in the file.
 */
export type EffectPreview =
  | { kind: "css"; filter: (params: Record<string, number>, scale: number) => string }
  | {
      /** The inner content of an SVG `<filter>`, referenced via `url(#id)`. */
      kind: "svg";
      build: (params: Record<string, number>, scale: number) => string;
    }
  | {
      /** A full-frame layer composited over the picture. */
      kind: "overlay";
      style: (params: Record<string, number>) => Record<string, string | number>;
      /** Marks the animated grain layer, which carries its own CSS class. */
      grain?: boolean;
    }
  | { kind: "pixelate" }
  | { kind: "mirror" }
  | { kind: "fisheye" }
  | { kind: "jitter" };

/** One geometry redraw the preview's canvas pass performs, in order. */
export type CanvasOp =
  | { kind: "pixelate"; size: number }
  | { kind: "mirror" }
  | { kind: "fisheye"; strength: number };

/** Everything the monitor needs to draw a clip's effects live. */
export interface PreviewLook {
  /** The CSS `filter` property: functions and `url(#id)` refs, in order. */
  filter: string | null;
  /** SVG `<filter>` elements to mount, matching the `url()` refs above. */
  svgFilters: { id: string; content: string }[];
  /** Layers composited over the picture, in effect order. */
  overlays: { style: Record<string, string | number>; grain?: boolean }[];
  /** Geometry redraws the canvas pass performs, in order. */
  canvas: CanvasOp[];
  /** Position jitter, evaluated from the playhead clock. Null for none. */
  jitter: { amount: number; speed: number } | null;
}

/**
 * The colour cast of a black-body temperature, as an SVG colour matrix.
 *
 * Tanner Helland's approximation, normalised so the strongest channel stays
 * at unity - a tint, not a brightness change - which is also roughly what
 * FFmpeg's `colortemperature` does.
 */
function temperatureMatrix(kelvinIn: number): string {
  const t = Math.min(400, Math.max(10, kelvinIn / 100));
  let r: number;
  let g: number;
  let b: number;
  if (t <= 66) {
    r = 255;
    g = 99.4708 * Math.log(t) - 161.1196;
    b = t <= 19 ? 0 : 138.5177 * Math.log(t - 10) - 305.0448;
  } else {
    r = 329.6987 * Math.pow(t - 60, -0.1332047);
    g = 288.1222 * Math.pow(t - 60, -0.0755148);
    b = 255;
  }
  const clamp = (value: number) => Math.min(255, Math.max(0, value)) / 255;
  const peak = Math.max(clamp(r), clamp(g), clamp(b), 1e-6);
  const [nr, ng, nb] = [clamp(r) / peak, clamp(g) / peak, clamp(b) / peak];
  return (
    `<feColorMatrix type="matrix" values="` +
    `${nr.toFixed(4)} 0 0 0 0  0 ${ng.toFixed(4)} 0 0 0  0 0 ${nb.toFixed(4)} 0 0  0 0 0 1 0"/>`
  );
}

/**
 * A 128px monochrome noise tile as a data URI, generated once. The grain
 * overlay tiles and jiggles it; regenerating per frame would be film-accurate
 * and battery-hostile.
 */
let grainTile: string | null = null;
function grainDataUri(): string {
  if (grainTile !== null) return grainTile;
  if (typeof document === "undefined") return (grainTile = "");
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) return (grainTile = "");
  const image = context.createImageData(128, 128);
  for (let at = 0; at < image.data.length; at += 4) {
    const value = Math.floor(Math.random() * 256);
    image.data[at] = value;
    image.data[at + 1] = value;
    image.data[at + 2] = value;
    image.data[at + 3] = 255;
  }
  context.putImageData(image, 0, 0);
  return (grainTile = canvas.toDataURL());
}

export type EffectCategory = "basic" | "blur" | "color" | "stylize" | "distort";

/** An effect applied to a clip, with whatever parameters were set. */
export interface AppliedEffect {
  id: string;
  params: Record<string, number>;
  /** False bypasses without losing settings. Absent means enabled. */
  enabled?: boolean;
}

/** A transition on the cut into a clip. */
export interface ClipTransition {
  id: string;
  /** Seconds the transition covers. */
  duration: number;
}

export interface EffectDefinition {
  id: string;
  label: string;
  category: EffectCategory;
  /** One line on what it does, shown in the card tooltip. */
  blurb: string;
  /** CSS background for the preview tile, until real thumbnails exist. */
  swatch: string;
  params: FilterParam[];
  /** Builds the FFmpeg video filter fragment for these parameter values.
   * `index` is the fragment's position in the clip's chain: any filtergraph
   * labels MUST embed it, or stacking the same effect twice duplicates
   * labels and FFmpeg rejects the whole graph. */
  chain: (params: Record<string, number>, index: number) => string;
  /** How the monitor draws it live. Every effect has one - no export-only. */
  preview: EffectPreview;
}

/** The categories under the "Video Effects" dropdown, in display order. */
export const EFFECT_CATEGORIES: { id: EffectCategory; label: string }[] = [
  { id: "basic", get label() { return t("effects.category.basic"); } },
  { id: "blur", get label() { return t("effects.category.blur"); } },
  { id: "color", get label() { return t("effects.category.color"); } },
  { id: "stylize", get label() { return t("effects.category.stylize"); } },
  { id: "distort", get label() { return t("effects.category.distort"); } },
];

const percent = (value: number) => `${Math.round(value)}%`;
const pixels = (value: number) => `${Math.round(value)} px`;
const times = (value: number) => `${value.toFixed(2)}x`;
const kelvin = (value: number) => `${Math.round(value)} K`;

export const EFFECTS: EffectDefinition[] = [
  // ── basic ────────────────────────────────────────────────────────────────
  {
    id: "black-white",
    get label() { return t("effects.black-white.label"); },
    category: "basic",
    get blurb() { return t("effects.black-white.blurb"); },
    swatch: "linear-gradient(135deg, #e8e8e8 0%, #6b6b6b 55%, #1c1c1c 100%)",
    params: [],
    chain: () => "hue=s=0",
    preview: { kind: "css", filter: () => "grayscale(1)" },
  },
  {
    id: "sepia",
    get label() { return t("effects.sepia.label"); },
    category: "basic",
    get blurb() { return t("effects.sepia.blurb"); },
    swatch: "linear-gradient(135deg, #e9d3ae 0%, #a9773f 60%, #513a1c 100%)",
    params: [],
    // The standard sepia matrix, the same one the CSS filter defines.
    chain: () =>
      "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131",
    preview: { kind: "css", filter: () => "sepia(1)" },
  },
  {
    id: "invert",
    get label() { return t("effects.invert.label"); },
    category: "basic",
    get blurb() { return t("effects.invert.blurb"); },
    swatch: "linear-gradient(135deg, #00d0ff 0%, #7a00c8 55%, #ffe600 100%)",
    params: [],
    chain: () => "negate",
    preview: { kind: "css", filter: () => "invert(1)" },
  },
  {
    id: "sharpen",
    get label() { return t("effects.sharpen.label"); },
    category: "basic",
    get blurb() { return t("effects.sharpen.blurb"); },
    swatch: "linear-gradient(135deg, #cfd8dc 0%, #607d8b 55%, #263238 100%)",
    params: [
      { key: "amount", get label() { return t("effects.sharpen.param.amount"); }, min: 0.2, max: 3, step: 0.1, default: 1, format: times },
    ],
    chain: ({ amount = 1 }) => `unsharp=5:5:${amount.toFixed(2)}:5:5:0`,
    preview: {
      kind: "svg",
      // An unsharp kernel: edges subtract, the centre compensates, sum one.
      build: ({ amount = 1 }) => {
        const a = (amount * 0.55).toFixed(3);
        const c = (1 + amount * 0.55 * 4).toFixed(3);
        return `<feConvolveMatrix order="3" divisor="1" preserveAlpha="true" kernelMatrix="0 -${a} 0 -${a} ${c} -${a} 0 -${a} 0"/>`;
      },
    },
  },
  // ── blur ─────────────────────────────────────────────────────────────────
  {
    id: "gaussian-blur",
    get label() { return t("effects.gaussian-blur.label"); },
    category: "blur",
    get blurb() { return t("effects.gaussian-blur.blurb"); },
    swatch: "linear-gradient(135deg, #b3c7f9 0%, #7f9cf5 55%, #4c6ef5 100%)",
    params: [
      { key: "radius", get label() { return t("effects.gaussian-blur.param.radius"); }, min: 1, max: 50, step: 1, default: 10, format: pixels },
    ],
    chain: ({ radius = 10 }) => `gblur=sigma=${radius.toFixed(1)}`,
    preview: { kind: "css", filter: ({ radius = 10 }, scale) => `blur(${(radius * scale).toFixed(2)}px)` },
  },
  {
    id: "box-blur",
    get label() { return t("effects.box-blur.label"); },
    category: "blur",
    get blurb() { return t("effects.box-blur.blurb"); },
    swatch: "linear-gradient(135deg, #a5d8ff 0%, #4dabf7 55%, #1971c2 100%)",
    params: [
      { key: "radius", get label() { return t("effects.box-blur.param.radius"); }, min: 1, max: 30, step: 1, default: 6, format: pixels },
    ],
    chain: ({ radius = 6 }) => `boxblur=${Math.round(radius)}:1`,
    preview: { kind: "css", filter: ({ radius = 6 }, scale) => `blur(${(radius * 0.7 * scale).toFixed(2)}px)` },
  },
  {
    id: "motion-blur",
    get label() { return t("effects.motion-blur.label"); },
    category: "blur",
    get blurb() { return t("effects.motion-blur.blurb"); },
    swatch: "linear-gradient(90deg, #91a7ff 0%, #5c7cfa 45%, #91a7ff 100%)",
    params: [
      { key: "length", get label() { return t("effects.motion-blur.param.length"); }, min: 2, max: 60, step: 1, default: 18, format: pixels },
    ],
    // Gaussian in one axis only is the streak; the tiny vertical sigma keeps
    // the filter happy without visibly blurring that axis.
    chain: ({ length = 18 }) => `gblur=sigma=${length.toFixed(1)}:sigmaV=0.1`,
    preview: {
      kind: "svg",
      // The same one-axis gaussian, which SVG spells as two deviations.
      build: ({ length = 18 }, scale) =>
        `<feGaussianBlur stdDeviation="${(length * scale).toFixed(2)} 0.01"/>`,
    },
  },
  // ── color ────────────────────────────────────────────────────────────────
  {
    id: "warm",
    get label() { return t("effects.warm.label"); },
    category: "color",
    get blurb() { return t("effects.warm.blurb"); },
    swatch: "linear-gradient(135deg, #ffd8a8 0%, #ff922b 55%, #d9480f 100%)",
    params: [
      {
        key: "temperature",
        get label() { return t("effects.warm.param.temperature"); },
        min: 3000,
        max: 6000,
        step: 100,
        default: 4600,
        format: kelvin,
      },
    ],
    chain: ({ temperature = 4600 }) =>
      `colortemperature=temperature=${Math.round(temperature)}`,
    preview: { kind: "svg", build: ({ temperature = 4600 }) => temperatureMatrix(temperature) },
  },
  {
    id: "cool",
    get label() { return t("effects.cool.label"); },
    category: "color",
    get blurb() { return t("effects.cool.blurb"); },
    swatch: "linear-gradient(135deg, #99e9f2 0%, #22b8cf 55%, #0b7285 100%)",
    params: [
      {
        key: "temperature",
        get label() { return t("effects.cool.param.temperature"); },
        min: 7000,
        max: 11000,
        step: 100,
        default: 8500,
        format: kelvin,
      },
    ],
    chain: ({ temperature = 8500 }) =>
      `colortemperature=temperature=${Math.round(temperature)}`,
    preview: { kind: "svg", build: ({ temperature = 8500 }) => temperatureMatrix(temperature) },
  },
  {
    id: "vibrance",
    get label() { return t("effects.vibrance.label"); },
    category: "color",
    get blurb() { return t("effects.vibrance.blurb"); },
    swatch: "linear-gradient(135deg, #ff6b6b 0%, #fcc419 40%, #51cf66 70%, #339af0 100%)",
    params: [
      { key: "intensity", get label() { return t("effects.vibrance.param.intensity"); }, min: 0.1, max: 2, step: 0.05, default: 0.7, format: times },
    ],
    chain: ({ intensity = 0.7 }) => `vibrance=intensity=${intensity.toFixed(2)}`,
    preview: { kind: "css", filter: ({ intensity = 0.7 }) => `saturate(${(1 + intensity * 0.45).toFixed(2)})` },
  },
  {
    id: "contrast-pop",
    get label() { return t("effects.contrast-pop.label"); },
    category: "color",
    get blurb() { return t("effects.contrast-pop.blurb"); },
    swatch: "linear-gradient(135deg, #f8f9fa 0%, #868e96 45%, #212529 100%)",
    params: [
      { key: "contrast", get label() { return t("effects.contrast-pop.param.contrast"); }, min: 1, max: 2, step: 0.05, default: 1.25, format: times },
    ],
    chain: ({ contrast = 1.25 }) => `eq=contrast=${contrast.toFixed(2)}`,
    preview: { kind: "css", filter: ({ contrast = 1.25 }) => `contrast(${contrast.toFixed(2)})` },
  },
  // ── stylize ──────────────────────────────────────────────────────────────
  {
    id: "vignette",
    get label() { return t("effects.vignette.label"); },
    category: "stylize",
    get blurb() { return t("effects.vignette.blurb"); },
    swatch: "radial-gradient(circle at 50% 50%, #ced4da 0%, #495057 60%, #16191c 100%)",
    params: [
      { key: "strength", get label() { return t("effects.vignette.param.strength"); }, min: 10, max: 100, step: 1, default: 50, format: percent },
    ],
    // The filter's angle runs 0..PI/2, wider being darker corners; the slider
    // maps into the range that reads as a vignette rather than a tunnel.
    chain: ({ strength = 50 }) =>
      `vignette=angle=${(0.25 + (strength / 100) * 1.05).toFixed(3)}`,
    preview: {
      kind: "overlay",
      style: ({ strength = 50 }) => ({
        background: `radial-gradient(ellipse at center, transparent ${Math.round(
          72 - strength * 0.45,
        )}%, rgba(0,0,0,${(0.35 + strength * 0.006).toFixed(3)}) 100%)`,
      }),
    },
  },
  {
    id: "film-grain",
    get label() { return t("effects.film-grain.label"); },
    category: "stylize",
    get blurb() { return t("effects.film-grain.blurb"); },
    swatch: "linear-gradient(135deg, #dee2e6 0%, #adb5bd 50%, #495057 100%)",
    params: [
      { key: "amount", get label() { return t("effects.film-grain.param.amount"); }, min: 2, max: 40, step: 1, default: 12, format: percent },
    ],
    // Temporal (t) so the grain dances like film instead of sitting still.
    chain: ({ amount = 12 }) => `noise=alls=${Math.round(amount)}:allf=t+u`,
    preview: {
      kind: "overlay",
      grain: true,
      style: ({ amount = 12 }) => ({
        backgroundImage: `url(${grainDataUri()})`,
        backgroundRepeat: "repeat",
        mixBlendMode: "overlay",
        opacity: Math.min(1, amount / 40),
      }),
    },
  },
  {
    id: "glow",
    get label() { return t("effects.glow.label"); },
    category: "stylize",
    get blurb() { return t("effects.glow.blurb"); },
    swatch: "radial-gradient(circle at 50% 40%, #fff9db 0%, #ffe066 45%, #e8590c 100%)",
    params: [
      { key: "amount", get label() { return t("effects.glow.param.amount"); }, min: 10, max: 100, step: 1, default: 45, format: percent },
    ],
    // Screen-blend a blurred copy over itself - the classic bloom. Labels
    // carry the chain index so two glows cannot collide in one graph.
    chain: ({ amount = 45 }, index) =>
      `split[glowa${index}][glowb${index}];[glowb${index}]gblur=sigma=18[glowg${index}];` +
      `[glowa${index}][glowg${index}]blend=all_mode=screen:all_opacity=${(amount / 100).toFixed(2)}`,
    preview: {
      kind: "svg",
      // The same bloom: blur a copy, weight it, screen it over the source.
      build: ({ amount = 45 }, scale) => {
        const weight = (amount / 100).toFixed(3);
        return (
          `<feGaussianBlur in="SourceGraphic" stdDeviation="${(18 * scale).toFixed(2)}" result="wolfglow-blur"/>` +
          `<feColorMatrix in="wolfglow-blur" type="matrix" values="${weight} 0 0 0 0  0 ${weight} 0 0 0  0 0 ${weight} 0 0  0 0 0 1 0" result="wolfglow-dim"/>` +
          `<feBlend in="SourceGraphic" in2="wolfglow-dim" mode="screen"/>`
        );
      },
    },
  },
  {
    id: "posterize",
    get label() { return t("effects.posterize.label"); },
    category: "stylize",
    get blurb() { return t("effects.posterize.blurb"); },
    swatch:
      "linear-gradient(135deg, #e64980 0%, #e64980 33%, #7950f2 33%, #7950f2 66%, #1098ad 66%, #1098ad 100%)",
    params: [
      { key: "levels", get label() { return t("effects.posterize.param.levels"); }, min: 2, max: 8, step: 1, default: 4, format: (v) => String(Math.round(v)) },
    ],
    chain: ({ levels = 4 }) => {
      const size = Math.round(256 / Math.max(2, Math.round(levels)));
      const band = `trunc(val/${size})*${size}`;
      return `lutrgb=r=${band}:g=${band}:b=${band}`;
    },
    preview: {
      kind: "svg",
      // Discrete transfer with the same band values the lut produces.
      build: ({ levels = 4 }) => {
        const count = Math.max(2, Math.round(levels));
        const size = Math.round(256 / count);
        const table = Array.from({ length: count }, (_, index) =>
          ((index * size) / 255).toFixed(4),
        ).join(" ");
        return (
          `<feComponentTransfer>` +
          `<feFuncR type="discrete" tableValues="${table}"/>` +
          `<feFuncG type="discrete" tableValues="${table}"/>` +
          `<feFuncB type="discrete" tableValues="${table}"/>` +
          `</feComponentTransfer>`
        );
      },
    },
  },
  // ── distort ──────────────────────────────────────────────────────────────
  {
    id: "pixelate",
    get label() { return t("effects.pixelate.label"); },
    category: "distort",
    get blurb() { return t("effects.pixelate.blurb"); },
    swatch:
      "repeating-linear-gradient(0deg, #74c0fc 0 6px, #4dabf7 6px 12px), repeating-linear-gradient(90deg, #74c0fc80 0 6px, #4dabf780 6px 12px)",
    params: [
      { key: "size", get label() { return t("effects.pixelate.param.size"); }, min: 2, max: 64, step: 1, default: 16, format: pixels },
    ],
    chain: ({ size = 16 }) =>
      `pixelize=width=${Math.round(size)}:height=${Math.round(size)}`,
    preview: { kind: "pixelate" },
  },
  {
    id: "mirror",
    get label() { return t("effects.mirror.label"); },
    category: "distort",
    get blurb() { return t("effects.mirror.blurb"); },
    swatch: "linear-gradient(90deg, #63e6be 0%, #0ca678 50%, #63e6be 100%)",
    params: [],
    chain: (_params, index) =>
      `crop=iw/2:ih:0:0,split[mirl${index}][mirr${index}];[mirr${index}]hflip[mirf${index}];` +
      `[mirl${index}][mirf${index}]hstack`,
    preview: { kind: "mirror" },
  },
  {
    id: "fisheye",
    get label() { return t("effects.fisheye.label"); },
    category: "distort",
    get blurb() { return t("effects.fisheye.blurb"); },
    swatch: "radial-gradient(circle at 50% 50%, #d0bfff 0%, #9775fa 55%, #5f3dc4 100%)",
    params: [
      { key: "strength", get label() { return t("effects.fisheye.param.strength"); }, min: 5, max: 100, step: 1, default: 50, format: percent },
    ],
    // Negative correction coefficients produce barrel distortion - the bulge.
    chain: ({ strength = 50 }) => {
      const k = strength / 100;
      return `lenscorrection=k1=${(-0.55 * k).toFixed(3)}:k2=${(-0.2 * k).toFixed(3)}:i=bilinear`;
    },
    preview: { kind: "fisheye" },
  },
  {
    id: "shake",
    get label() { return t("effects.shake.label"); },
    category: "distort",
    get blurb() { return t("effects.shake.blurb"); },
    swatch: "linear-gradient(105deg, #ffc9c9 0%, #ff8787 40%, #fa5252 60%, #ffc9c9 100%)",
    params: [
      { key: "amount", get label() { return t("effects.shake.param.amount"); }, min: 2, max: 40, step: 1, default: 12, format: pixels },
      { key: "speed", get label() { return t("effects.shake.param.speed"); }, min: 2, max: 30, step: 1, default: 13, format: times },
    ],
    // A jittering crop window; the decoder's guard scale stretches the
    // slightly smaller window back to full size afterwards.
    chain: ({ amount = 12, speed = 13 }) => {
      const a = Math.round(amount);
      const f = Math.round(speed);
      return (
        `crop=iw-${2 * a}:ih-${2 * a}` +
        `:${a}+${a}*sin(t*${f}):${a}+${a}*cos(t*${Math.round(f * 1.3)})`
      );
    },
    preview: { kind: "jitter" },
  },
];

export function findEffect(id: string): EffectDefinition | null {
  return EFFECTS.find((effect) => effect.id === id) ?? null;
}

/** Fills in any parameter the clip did not set. */
export function resolveEffectParams(
  definition: EffectDefinition,
  params: Record<string, number>,
): Record<string, number> {
  const resolved: Record<string, number> = {};
  for (const param of definition.params) {
    resolved[param.key] = params[param.key] ?? param.default;
  }
  return resolved;
}

/**
 * The complete FFmpeg video filter string for a clip, or null if it has none.
 * Effects apply in the order they were added, exactly like audio filters.
 */
export function buildEffectChain(effects: readonly AppliedEffect[] | undefined): string | null {
  if (!effects) return null;
  const fragments: string[] = [];
  for (const applied of effects) {
    if (applied.enabled === false) continue;
    const definition = findEffect(applied.id);
    if (!definition) continue;
    // The emitted position, not the list position: labels stay stable when
    // a bypassed effect sits earlier in the list.
    fragments.push(definition.chain(resolveEffectParams(definition, applied.params), fragments.length));
  }
  return fragments.length > 0 ? fragments.join(",") : null;
}

/**
 * Everything the monitor needs to draw a clip's effects live, assembled from
 * each effect's `preview` in applied order.
 *
 * `scale` is preview pixels per export pixel. CSS/SVG filters keep their
 * relative order inside the one `filter` property (CSS applies the list in
 * sequence); overlays, canvas geometry and jitter compose on top in their own
 * layers - close enough that the export only ever reads as a sharper version
 * of the monitor, never a different picture.
 */
export function buildPreviewLook(
  effects: readonly AppliedEffect[] | undefined,
  scale: number,
): PreviewLook {
  const look: PreviewLook = {
    filter: null,
    svgFilters: [],
    overlays: [],
    canvas: [],
    jitter: null,
  };
  if (!effects || effects.length === 0) return look;

  const filterParts: string[] = [];
  effects.forEach((applied, index) => {
    if (applied.enabled === false) return;
    const definition = findEffect(applied.id);
    if (!definition) return;
    const params = resolveEffectParams(definition, applied.params);
    const preview = definition.preview;

    switch (preview.kind) {
      case "css":
        filterParts.push(preview.filter(params, scale));
        break;
      case "svg": {
        const id = `wolffx-${index}-${definition.id}`;
        look.svgFilters.push({ id, content: preview.build(params, scale) });
        filterParts.push(`url(#${id})`);
        break;
      }
      case "overlay":
        look.overlays.push({ style: preview.style(params), grain: preview.grain });
        break;
      case "pixelate":
        look.canvas.push({ kind: "pixelate", size: Math.max(2, params.size ?? 16) });
        break;
      case "mirror":
        look.canvas.push({ kind: "mirror" });
        break;
      case "fisheye":
        look.canvas.push({ kind: "fisheye", strength: (params.strength ?? 50) / 100 });
        break;
      case "jitter":
        // Several shakes do not add up to a bigger shake; the strongest wins.
        if (!look.jitter || (params.amount ?? 12) > look.jitter.amount) {
          look.jitter = { amount: params.amount ?? 12, speed: params.speed ?? 13 };
        }
        break;
    }
  });

  look.filter = filterParts.length > 0 ? filterParts.join(" ") : null;
  return look;
}

export type TransitionCategory = "basic" | "motion";

export interface TransitionDefinition {
  id: string;
  label: string;
  category: TransitionCategory;
  /** One line on what it does, shown in the card tooltip. */
  blurb: string;
  /**
   * False while the engine cannot render it - the card stays browsable with
   * a "Soon" badge, exactly as the whole catalogue once was. The motion
   * transitions wait on animated transforms in the engine.
   */
  implemented: boolean;
  /** Seconds a freshly applied transition covers. */
  defaultDuration: number;
}

/** The categories under the "Transitions" dropdown, in display order. */
export const TRANSITION_CATEGORIES: { id: TransitionCategory; label: string }[] = [
  { id: "basic", get label() { return t("transitions.category.basic"); } },
  { id: "motion", get label() { return t("transitions.category.motion"); } },
];

export const TRANSITIONS: TransitionDefinition[] = [
  {
    id: "cross-fade",
    get label() { return t("transitions.cross-fade.label"); },
    category: "basic",
    get blurb() { return t("transitions.cross-fade.blurb"); },
    implemented: true,
    defaultDuration: 1,
  },
  {
    id: "fade-black",
    get label() { return t("transitions.fade-black.label"); },
    category: "basic",
    get blurb() { return t("transitions.fade-black.blurb"); },
    implemented: true,
    defaultDuration: 1,
  },
  {
    id: "fade-white",
    get label() { return t("transitions.fade-white.label"); },
    category: "basic",
    get blurb() { return t("transitions.fade-white.blurb"); },
    implemented: true,
    defaultDuration: 1,
  },
  {
    id: "wipe-left",
    get label() { return t("transitions.wipe-left.label"); },
    category: "motion",
    get blurb() { return t("transitions.wipe-left.blurb"); },
    implemented: false,
    defaultDuration: 0.8,
  },
  {
    id: "wipe-right",
    get label() { return t("transitions.wipe-right.label"); },
    category: "motion",
    get blurb() { return t("transitions.wipe-right.blurb"); },
    implemented: false,
    defaultDuration: 0.8,
  },
  {
    id: "push",
    get label() { return t("transitions.push.label"); },
    category: "motion",
    get blurb() { return t("transitions.push.blurb"); },
    implemented: false,
    defaultDuration: 0.8,
  },
  {
    id: "zoom",
    get label() { return t("transitions.zoom.label"); },
    category: "motion",
    get blurb() { return t("transitions.zoom.blurb"); },
    implemented: false,
    defaultDuration: 0.8,
  },
];

export function findTransition(id: string): TransitionDefinition | null {
  return TRANSITIONS.find((transition) => transition.id === id) ?? null;
}
