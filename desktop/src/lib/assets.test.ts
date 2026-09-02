// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

// @vitest-environment jsdom
/**
 * The waveform cache's wire format.
 *
 * `encodePeaks`/`decodePeaks` are the two halves of the on-disk artwork cache:
 * what encode writes today, decode must read on every future launch. A break
 * in the round-trip does not crash anything - loadPeaks falls back to a full
 * re-decode - but it silently voids the cache, and a decode that misreads
 * lengths would draw garbage waveforms. When a layout expectation here fails,
 * remember the old files on disk: decode must keep reading yesterday's bytes,
 * or the cache key (in `loadPeaks`) must change so stale entries miss.
 *
 * jsdom because `assets.ts` reads `window.devicePixelRatio` at module scope.
 */
import { describe, expect, test } from "vitest";

import { decodePeaks, encodePeaks, type Peaks } from "./assets";

function peaks(min: number[], max: number[], bucketsPerSecond = 200): Peaks {
  return {
    min: Float32Array.from(min),
    max: Float32Array.from(max),
    bucketsPerSecond,
  };
}

const roundTrip = (source: Peaks) => decodePeaks(encodePeaks(source).buffer);

describe("the peaks round-trip", () => {
  test("decode(encode(x)) preserves every bucket exactly", () => {
    // Samples are already Float32 on both sides, so the only quantisation
    // happened before encode ever saw them: the trip itself must be lossless.
    const source = peaks([-1, -0.5, 0, -0.001], [1, 0.5, 0, 0.001]);
    const decoded = roundTrip(source);
    expect(decoded).not.toBeNull();
    expect(decoded!.min).toEqual(source.min);
    expect(decoded!.max).toEqual(source.max);
    expect(decoded!.bucketsPerSecond).toBe(200);
  });

  test("a fractional bucket rate survives to Float32 precision", () => {
    // loadPeaks computes sampleRate / bucketSize, rarely a round number.
    const rate = 44100 / 220; // 200.4545...
    const decoded = roundTrip(peaks([0], [0], rate));
    expect(decoded!.bucketsPerSecond).toBe(Math.fround(rate));
    expect(decoded!.bucketsPerSecond).toBeCloseTo(rate, 4);
  });

  test("many arbitrary values survive within Float32 quantisation", () => {
    // A deterministic pseudo-random spread across the full sample range.
    const values = Array.from({ length: 1000 }, (_, index) =>
      Math.sin(index * 12.9898) * ((index % 7) / 7),
    );
    const source = peaks(values.map((value) => -Math.abs(value)), values);
    const decoded = roundTrip(source);
    expect(decoded!.min).toEqual(source.min);
    expect(decoded!.max).toEqual(source.max);
    for (let index = 0; index < values.length; index += 1) {
      expect(Math.abs(decoded!.max[index] - values[index])).toBeLessThanOrEqual(
        Math.abs(Math.fround(values[index]) - values[index]) + Number.EPSILON,
      );
    }
  });

  test("the extremes 0, 1 and -1 come back exact", () => {
    const decoded = roundTrip(peaks([-1, 0], [1, 0]));
    expect(Array.from(decoded!.min)).toEqual([-1, 0]);
    expect(Array.from(decoded!.max)).toEqual([1, 0]);
  });

  test("a single bucket round-trips", () => {
    const decoded = roundTrip(peaks([-0.25], [0.75]));
    expect(decoded!.min).toHaveLength(1);
    expect(decoded!.min[0]).toBeCloseTo(-0.25, 10);
    expect(decoded!.max[0]).toBeCloseTo(0.75, 10);
  });

  test("empty peaks round-trip as a valid header-only file", () => {
    const bytes = encodePeaks(peaks([], []));
    expect(bytes.byteLength).toBe(8);
    const decoded = decodePeaks(bytes.buffer);
    expect(decoded).not.toBeNull();
    expect(decoded!.min).toHaveLength(0);
    expect(decoded!.max).toHaveLength(0);
  });
});

describe("the encoded layout", () => {
  test("[bucketsPerSecond f32][count u32][min f32...][max f32...], little-endian", () => {
    // This is the on-disk contract. Changing it strands every existing cache
    // file, so a failure here demands either a compatible decode or a new key.
    const bytes = encodePeaks(peaks([-0.5, -1], [0.5, 1], 200));
    expect(bytes.byteLength).toBe(8 + 2 * 8);
    const view = new DataView(bytes.buffer);
    expect(view.getFloat32(0, true)).toBe(200);
    expect(view.getUint32(4, true)).toBe(2);
    expect(view.getFloat32(8, true)).toBe(-0.5); // min block first
    expect(view.getFloat32(12, true)).toBe(-1);
    expect(view.getFloat32(16, true)).toBe(0.5); // then the max block
    expect(view.getFloat32(20, true)).toBe(1);
  });
});

describe("decodePeaks fails soft on bad files", () => {
  const valid = () => encodePeaks(peaks([-1], [1])).buffer;

  test("a file too short for the header is null", () => {
    expect(decodePeaks(new ArrayBuffer(0))).toBeNull();
    expect(decodePeaks(new ArrayBuffer(7))).toBeNull();
  });

  test("a truncated or padded body is null, never a misread", () => {
    const truncated = valid().slice(0, 12);
    expect(decodePeaks(truncated)).toBeNull();
    const padded = new Uint8Array(24);
    padded.set(new Uint8Array(valid()));
    expect(decodePeaks(padded.buffer)).toBeNull();
  });

  test("a count that overruns the file is null", () => {
    const bytes = new Uint8Array(valid());
    new DataView(bytes.buffer).setUint32(4, 1000, true);
    expect(decodePeaks(bytes.buffer)).toBeNull();
  });

  test("a zero, negative or NaN bucket rate is null", () => {
    for (const rate of [0, -200, Number.NaN]) {
      const bytes = new Uint8Array(valid());
      new DataView(bytes.buffer).setFloat32(0, rate, true);
      expect(decodePeaks(bytes.buffer)).toBeNull();
    }
  });
});
