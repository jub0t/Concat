// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Clip artwork: waveforms for audio, filmstrips for video.
 *
 * Both are expensive to produce and never change, so they are computed once
 * per media item and cached in a plain mutable `Map`. The timeline reads that
 * map directly from inside its draw loop, which means artwork appearing does
 * not require a React render - the next frame simply has it.
 *
 * Everything here fails soft. A file whose peaks cannot be decoded draws as a
 * flat clip, which is exactly what it looked like before, rather than taking
 * the timeline down with it.
 */


import {
  extractFilmstrip,
  extractPeaks,
  readArtwork as readArtworkFile,
  readMediaBytes,
  writeArtwork as writeArtworkFile,
} from "./engine";

/** Min/max sample pairs, bucketed at a fixed rate. */
export interface Peaks {
  min: Float32Array;
  max: Float32Array;
  /** How many buckets cover one second of audio. */
  bucketsPerSecond: number;
}

export interface MediaAssets {
  peaks: Map<string, Peaks>;
  /** A horizontal strip of evenly spaced frames, as one decoded image. */
  strips: Map<string, ImageBitmap>;
  /** How many frames are in each strip. */
  stripFrames: Map<string, number>;
  /** Guards against starting the same work twice. */
  pending: Set<string>;
  /** Notified when anything lands, for views that do not redraw continuously. */
  listeners: Set<() => void>;
}

export function createAssets(): MediaAssets {
  return {
    peaks: new Map(),
    strips: new Map(),
    stripFrames: new Map(),
    pending: new Set(),
    listeners: new Set(),
  };
}

/**
 * Subscribes to artwork arriving. Returns an unsubscribe function.
 *
 * The timeline does not need this - it repaints every frame regardless - but
 * anything that draws once and then sits there, like a bin thumbnail, does.
 */
export function subscribeAssets(assets: MediaAssets, listener: () => void): () => void {
  assets.listeners.add(listener);
  return () => assets.listeners.delete(listener);
}

function announce(assets: MediaAssets): void {
  for (const listener of assets.listeners) listener();
}

/** Frames per filmstrip. Enough to read the shot, few enough to stay cheap. */
const STRIP_FRAMES = 24;
/**
 * Strip frames are drawn at ~32-72 CSS pixels tall, so they need the
 * display's real pixel density or a Retina screen upscales them into mush.
 * Capped at the engine's 240 limit; the ratio is read once because a strip
 * cached at one density is not worth regenerating on a monitor change.
 */
const STRIP_HEIGHT = Math.min(240, Math.round(72 * (window.devicePixelRatio || 1)));

/**
 * Starts producing artwork for one media item if it is not already cached.
 *
 * Fire and forget - the caller does not await this. Results land in the maps
 * and the timeline picks them up on its next frame.
 */
export function requestAssets(
  assets: MediaAssets,
  media: {
    id: string;
    path: string;
    kind: "video" | "audio" | "image";
    duration: number | null;
    hasAudio: boolean;
  },
  /** Project folder holding the on-disk artwork cache; null skips caching. */
  projectPath: string | null = null,
): void {
  if (assets.pending.has(media.id)) return;

  // Video peaks are deliberately NOT loaded here. Even engine-side, decoding
  // a video's whole audio track is real work - so it happens only on demand,
  // via requestVideoPeaks, when a detached audio clip needs drawing.
  const wantsPeaks = media.kind === "audio" && !assets.peaks.has(media.id);
  const wantsStrip = media.kind !== "audio" && !assets.strips.has(media.id);
  if (!wantsPeaks && !wantsStrip) return;

  assets.pending.add(media.id);

  const jobs: Promise<unknown>[] = [];
  if (wantsPeaks) {
    jobs.push(
      loadPeaks(media.path, projectPath).then((peaks) => {
        if (peaks) assets.peaks.set(media.id, peaks);
      }),
    );
  }
  if (wantsStrip) {
    jobs.push(
      media.kind === "image"
        ? // A still is a one-frame filmstrip. Storing it that way means the
          // timeline and the bin draw it with the code they already have, and
          // a long still clip tiles the same frame instead of stretching it.
          loadImage(media.path).then((bitmap) => {
            if (bitmap) {
              assets.strips.set(media.id, bitmap);
              assets.stripFrames.set(media.id, 1);
            }
          })
        : loadStrip(media.path, media.duration, projectPath).then((strip) => {
            if (strip) {
              assets.strips.set(media.id, strip);
              assets.stripFrames.set(media.id, STRIP_FRAMES);
            }
          }),
    );
  }

  void Promise.allSettled(jobs).then(() => {
    assets.pending.delete(media.id);
    announce(assets);
  });
}

/**
 * Waveform for a video's audio track, on demand.
 *
 * Called when a detached audio clip exists (or is being created) for a video,
 * which is the only time a video needs peaks. Expensive on first run - the
 * engine decodes the whole audio track - but the result lands in the disk
 * cache, so a reopened project pays only a file read.
 */
export function requestVideoPeaks(
  assets: MediaAssets,
  media: { id: string; path: string; kind: "video" | "audio" | "image"; hasAudio: boolean },
  projectPath: string | null = null,
): void {
  if (media.kind !== "video" || !media.hasAudio) return;
  if (assets.peaks.has(media.id)) return;

  const pendingKey = `${media.id}:peaks`;
  if (assets.pending.has(pendingKey)) return;
  assets.pending.add(pendingKey);

  void loadPeaks(media.path, projectPath)
    .then((peaks) => {
      if (peaks) assets.peaks.set(media.id, peaks);
    })
    .finally(() => {
      assets.pending.delete(pendingKey);
      announce(assets);
    });
}

/**
 * The on-disk artwork cache, kept in the project's folder.
 *
 * Strips and peaks survive a relaunch there, which is the difference between
 * a reopened project showing its artwork immediately and every launch paying
 * for ffmpeg runs and full audio decodes again. Reads and writes both fail
 * soft: a miss or a failed write only means generating now, caching later.
 */
function artworkKey(path: string): string {
  // FNV-1a over the absolute path. Collisions are astronomically unlikely at
  // bin sizes, and a collision's worst case is a wrong thumbnail.
  let hash = 0x811c9dc5;
  for (let index = 0; index < path.length; index += 1) {
    hash ^= path.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

async function readArtwork(project: string | null, key: string): Promise<ArrayBuffer | null> {
  if (!project) return null;
  try {
    return await readArtworkFile(project, key);
  } catch {
    return null;
  }
}

function writeArtwork(project: string | null, key: string, bytes: Uint8Array): void {
  if (!project) return;
  // Serialising the payload for IPC is main-thread work, and nothing about a
  // cache write is urgent - it waits for an idle moment so it never competes
  // with playback.
  const send = () => void writeArtworkFile(project, key, bytes).catch(() => undefined);
  if (typeof window.requestIdleCallback === "function") window.requestIdleCallback(() => send());
  else window.setTimeout(send, 500);
}

/** [bucketsPerSecond f32][count u32][min f32...][max f32...], little-endian. */
export function encodePeaks(peaks: Peaks): Uint8Array {
  const count = peaks.min.length;
  const buffer = new ArrayBuffer(8 + count * 8);
  const view = new DataView(buffer);
  view.setFloat32(0, peaks.bucketsPerSecond, true);
  view.setUint32(4, count, true);
  new Float32Array(buffer, 8, count).set(peaks.min);
  new Float32Array(buffer, 8 + count * 4, count).set(peaks.max);
  return new Uint8Array(buffer);
}

export function decodePeaks(bytes: ArrayBuffer): Peaks | null {
  if (bytes.byteLength < 8) return null;
  const view = new DataView(bytes);
  const bucketsPerSecond = view.getFloat32(0, true);
  const count = view.getUint32(4, true);
  if (!Number.isFinite(bucketsPerSecond) || bucketsPerSecond <= 0) return null;
  if (bytes.byteLength !== 8 + count * 8) return null;
  return {
    min: new Float32Array(bytes, 8, count),
    max: new Float32Array(bytes, 8 + count * 4, count),
    bucketsPerSecond,
  };
}

/**
 * Asks the engine for a file's peaks.
 *
 * The decode, the bucketing and the disk cache all live host-side now - the
 * engine streams FFmpeg's output straight into buckets, so neither the file
 * nor its samples ever cross the IPC boundary. What arrives here is the
 * encoded buckets, ready for `decodePeaks`.
 */
async function loadPeaks(path: string, project: string | null): Promise<Peaks | null> {
  try {
    return decodePeaks(await extractPeaks(path, project));
  } catch (cause) {
    console.warn(`WolfCut: no waveform for ${path}`, cause);
    return null;
  }
}

/** Decodes a still. No ffmpeg involved - the browser already reads these. */
async function loadImage(path: string): Promise<ImageBitmap | null> {
  try {
    const bytes = await readMediaBytes(path);
    return await createImageBitmap(new Blob([bytes]));
  } catch (cause) {
    console.warn(`WolfCut: could not decode ${path}`, cause);
    return null;
  }
}

/** Asks the engine for a strip of frames and decodes it into an ImageBitmap. */
async function loadStrip(
  path: string,
  duration: number | null,
  project: string | null,
): Promise<ImageBitmap | null> {
  if (!duration || duration <= 0) return null;

  // The key carries the strip's shape, so a density or frame-count change
  // regenerates rather than serving yesterday's resolution.
  const key = `${artworkKey(path)}-f${STRIP_FRAMES}-h${STRIP_HEIGHT}.strip.jpg`;
  const cached = await readArtwork(project, key);
  if (cached) {
    try {
      return await createImageBitmap(new Blob([cached], { type: "image/jpeg" }));
    } catch {
      // A corrupt cache entry falls through to regeneration.
    }
  }

  try {
    const bytes = await extractFilmstrip(path, STRIP_FRAMES, STRIP_HEIGHT);
    writeArtwork(project, key, new Uint8Array(bytes));
    // createImageBitmap decodes off the main thread, so a slow JPEG does not
    // stall the timeline.
    return await createImageBitmap(new Blob([bytes], { type: "image/jpeg" }));
  } catch (cause) {
    console.warn(`WolfCut: no filmstrip for ${path}`, cause);
    return null;
  }
}
