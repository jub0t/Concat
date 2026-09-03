#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

// Renders one preview thumbnail per video effect by running each effect's
// real FFmpeg chain - the same `buildEffectChain` the exporter uses, at its
// default parameters - over one source image. The catalogue then shows what
// an effect actually does instead of a gradient standing in for it.
//
//   node scripts/generate-effect-previews.mjs <image> [outDir]
//
// Defaults land in desktop/src/assets/effect-previews/<effect-id>.webp,
// where the media bin picks them up by id (Vite globs the directory).
// Re-run whenever the source image or the effect list changes.
//
// The image is scaled to tile size *before* the chain runs, so pixel-sized
// effects (blur radii, pixelation) read at preview scale instead of
// vanishing; WebP at quality 80 keeps the whole set a few kilobytes each.

import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const [, , image, outDirArg] = process.argv;
if (!image) {
  console.error("usage: generate-effect-previews.mjs <image> [outDir]");
  process.exit(2);
}
const outDir = outDirArg ?? path.join(root, "desktop/src/assets/effect-previews");
const TILE = { width: 320, height: 180 };

// The effect catalogue is TypeScript inside the app; esbuild (already there
// as Vite's own dependency) bundles it into an importable module so this
// script reads the same source of truth the app ships.
const require = createRequire(path.join(root, "desktop/package.json"));
const esbuild = require("esbuild");
const scratch = mkdtempSync(path.join(tmpdir(), "concat-previews-"));
const bundled = path.join(scratch, "effects.mjs");
await esbuild.build({
  entryPoints: [path.join(root, "desktop/src/lib/effects.ts")],
  bundle: true,
  format: "esm",
  outfile: bundled,
});
const { EFFECTS, buildEffectChain } = await import(pathToFileURL(bundled));

// WebP when this ffmpeg can encode it, JPEG otherwise - at 320x180 the
// difference is a few kilobytes per tile, not worth failing over.
const encoders = execFileSync("ffmpeg", ["-hide_banner", "-encoders"], {
  stdio: ["ignore", "pipe", "ignore"],
}).toString();
const format = encoders.includes("libwebp")
  ? { args: ["-c:v", "libwebp", "-quality", "80"], extension: "webp" }
  : { args: ["-c:v", "mjpeg", "-q:v", "4"], extension: "jpg" };

mkdirSync(outDir, { recursive: true });
const failed = [];
for (const effect of EFFECTS) {
  // Default parameters, position 0 in the chain - a fresh apply.
  const chain = buildEffectChain([{ id: effect.id, params: {} }]);
  const fit =
    `scale=${TILE.width}:${TILE.height}:force_original_aspect_ratio=increase,` +
    `crop=${TILE.width}:${TILE.height}`;
  const filter = chain ? `${fit},${chain}` : fit;
  const out = path.join(outDir, `${effect.id}.${format.extension}`);
  try {
    execFileSync(
      "ffmpeg",
      ["-y", "-v", "error", "-i", image, "-vf", filter,
       "-frames:v", "1", ...format.args, out],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
    console.log(`  ${effect.id}.${format.extension}`);
  } catch (error) {
    failed.push(effect.id);
    console.error(`FAILED ${effect.id}: ${error.stderr?.toString().trim() || error.message}`);
  }
}
rmSync(scratch, { recursive: true, force: true });

console.log(`\n${EFFECTS.length - failed.length}/${EFFECTS.length} previews written to ${outDir}`);
if (failed.length > 0) {
  console.error(`failed: ${failed.join(", ")}`);
  process.exit(1);
}
