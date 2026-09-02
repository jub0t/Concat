// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The audio filter catalogue's export strings, pinned character for character.
 *
 * `buildChain` is the single description of every filter: the exporter and the
 * preview both run exactly this string through FFmpeg, so there is nothing to
 * cross-check it against - these expectations are the check. Every catalogue
 * entry is spelled out at its default, minimum and maximum settings. When one
 * fails, the export sound changed - confirm FFmpeg accepts the new spelling
 * (aphaser caps speed at 2, atempo floors at 0.5) and that existing projects
 * still sound the same before updating the expectation.
 */
import { describe, expect, test } from "vitest";

import { buildChain, chainKey, FILTERS, findFilter } from "./filters";

const one = (id: string, params: Record<string, number> = {}) =>
  buildChain([{ id, params }]);

/** The definition's sliders, all pushed to one bound. */
const bound = (id: string, which: "min" | "max"): Record<string, number> =>
  Object.fromEntries(findFilter(id)!.params.map((param) => [param.key, param[which]]));

// The Voice category is built on the pitchShift helper: asetrate moves pitch
// and formants together, atempo puts the duration back. The rates below are
// 48000 * 2^(semitones/12) and its reciprocal, to the helper's own precision.
const DEFAULT_CHAINS: Record<string, string> = {
  sweet:
    "aresample=48000,asetrate=59334.357554,aresample=48000,atempo=0.80897480," +
    "highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4," +
    "equalizer=f=4200:t=q:w=0.8:g=4.180,equalizer=f=10500:t=q:w=0.7:g=5.550," +
    "compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1," +
    "deesser=i=0.388:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0432," +
    "alimiter=limit=0.94:attack=5:release=60",
  deep:
    "aresample=48000,asetrate=40363.027932,aresample=48000,atempo=1.18920712," +
    "highpass=f=55,equalizer=f=140:t=q:w=1.0:g=3.00," +
    "equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
  chipmunk:
    "aresample=48000,asetrate=71918.739690,aresample=48000,atempo=0.66741993," +
    "highpass=f=120,alimiter=limit=0.94",
  robot:
    "aphaser=type=t:speed=1.20:decay=0.6,flanger=delay=2:depth=4:speed=0.8," +
    "equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
  bass: "bass=g=5.00:f=110:w=0.6,alimiter=limit=0.95",
  treble: "treble=g=4.00:f=9000:w=0.7,alimiter=limit=0.95",
  telephone:
    "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=4.50,alimiter=limit=0.92",
  echo: "aecho=0.8:0.85:250:0.40",
  room: "aecho=0.8:0.88:24|43|70|101:0.320|0.240|0.170|0.110",
};

const MIN_CHAINS: Record<string, string> = {
  sweet:
    "aresample=48000,asetrate=51445.126202,aresample=48000,atempo=0.93303299," +
    "highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4," +
    "equalizer=f=4200:t=q:w=0.8:g=0.800,equalizer=f=10500:t=q:w=0.7:g=1.000," +
    "compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1," +
    "deesser=i=0.180:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0120," +
    "alimiter=limit=0.94:attack=5:release=60",
  deep:
    "aresample=48000,asetrate=30238.105197,aresample=48000,atempo=1.58740105," +
    "highpass=f=55,equalizer=f=140:t=q:w=1.0:g=0.00," +
    "equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
  chipmunk:
    "aresample=48000,asetrate=57081.941520,aresample=48000,atempo=0.84089642," +
    "highpass=f=120,alimiter=limit=0.94",
  robot:
    "aphaser=type=t:speed=0.56:decay=0.6,flanger=delay=2:depth=4:speed=0.8," +
    "equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
  bass: "bass=g=0.00:f=110:w=0.6,alimiter=limit=0.95",
  treble: "treble=g=0.00:f=9000:w=0.7,alimiter=limit=0.95",
  telephone:
    "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=3.00,alimiter=limit=0.92",
  echo: "aecho=0.8:0.85:50:0.10",
  room: "aecho=0.8:0.88:9|16|27|39:0.320|0.240|0.170|0.110",
};

const MAX_CHAINS: Record<string, string> = {
  sweet:
    "aresample=48000,asetrate=76195.250494,aresample=48000,atempo=0.62996052," +
    "highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4," +
    "equalizer=f=4200:t=q:w=0.8:g=6.000,equalizer=f=10500:t=q:w=0.7:g=8.000," +
    "compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1," +
    "deesser=i=0.500:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0600," +
    "alimiter=limit=0.94:attack=5:release=60",
  deep:
    "aresample=48000,asetrate=45305.967009,aresample=48000,atempo=1.05946309," +
    "highpass=f=55,equalizer=f=140:t=q:w=1.0:g=8.00," +
    "equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
  chipmunk:
    // +12 semitones is exactly double: the one point the maths comes out round.
    "aresample=48000,asetrate=96000.000000,aresample=48000,atempo=0.50000000," +
    "highpass=f=120,alimiter=limit=0.94",
  robot:
    // Depth 10 must land exactly on aphaser's speed cap of 2, never above it -
    // above the cap the filter refuses and the whole export fails.
    "aphaser=type=t:speed=2.00:decay=0.6,flanger=delay=2:depth=4:speed=0.8," +
    "equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
  bass: "bass=g=12.00:f=110:w=0.6,alimiter=limit=0.95",
  treble: "treble=g=12.00:f=9000:w=0.7,alimiter=limit=0.95",
  telephone:
    "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=8.00,alimiter=limit=0.92",
  echo: "aecho=0.8:0.85:1000:0.90",
  room: "aecho=0.8:0.88:46|82|134|194:0.320|0.240|0.170|0.110",
};

describe("the filter catalogue's export strings", () => {
  test("this suite knows every filter - a new filter must pin its strings here", () => {
    const ids = FILTERS.map((filter) => filter.id).sort();
    expect(Object.keys(DEFAULT_CHAINS).sort()).toEqual(ids);
    expect(Object.keys(MIN_CHAINS).sort()).toEqual(ids);
    expect(Object.keys(MAX_CHAINS).sort()).toEqual(ids);
  });

  test("every filter at its default settings", () => {
    for (const filter of FILTERS) {
      // No params passed: resolveParams must fill every default.
      expect(one(filter.id), filter.id).toBe(DEFAULT_CHAINS[filter.id]);
    }
  });

  test("every filter at its sliders' minimum", () => {
    for (const [id, expected] of Object.entries(MIN_CHAINS)) {
      expect(one(id, bound(id, "min")), id).toBe(expected);
    }
  });

  test("every filter at its sliders' maximum", () => {
    for (const [id, expected] of Object.entries(MAX_CHAINS)) {
      expect(one(id, bound(id, "max")), id).toBe(expected);
    }
  });
});

describe("buildChain composition", () => {
  test("stacked filters join with commas in applied order", () => {
    // EQ into a limiter is a different sound from a limiter into EQ; the
    // array's order is the contract.
    expect(
      buildChain([
        { id: "bass", params: {} },
        { id: "echo", params: {} },
      ]),
    ).toBe("bass=g=5.00:f=110:w=0.6,alimiter=limit=0.95,aecho=0.8:0.85:250:0.40");
  });

  test("a bypassed filter contributes nothing", () => {
    expect(
      buildChain([
        { id: "bass", params: {}, enabled: false },
        { id: "echo", params: {} },
      ]),
    ).toBe("aecho=0.8:0.85:250:0.40");
  });

  test("an unknown id is skipped, not exported as garbage", () => {
    expect(
      buildChain([
        { id: "from-the-future", params: {} },
        { id: "echo", params: {} },
      ]),
    ).toBe("aecho=0.8:0.85:250:0.40");
  });

  test("no audible filters means null, never an empty string", () => {
    expect(buildChain([])).toBeNull();
    expect(buildChain([{ id: "bass", params: {}, enabled: false }])).toBeNull();
  });
});

describe("chainKey", () => {
  test("identifies exactly one rendered sound", () => {
    const filters = [{ id: "echo", params: { delay: 0.5 } }];
    expect(chainKey("m1", 1.25, 4, filters)).toBe(
      "m1|1.250|4.000|aecho=0.8:0.85:500:0.40",
    );
    // A bypassed chain keys the same as no chain: both play the raw file.
    expect(chainKey("m1", 0, 4, [{ id: "echo", params: {}, enabled: false }])).toBe(
      chainKey("m1", 0, 4, []),
    );
  });
});
