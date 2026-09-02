// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The video effect catalogue's export strings, pinned character for character.
 *
 * `buildEffectChain` writes the FFmpeg fragment the exporter runs; a typo here
 * ships as a silently broken or subtly different-looking export. There is no
 * second implementation to cross-check against, so these tests ARE the check:
 * every catalogue entry at its default, minimum and maximum settings, spelled
 * out inline. When one fails, the export string changed - confirm FFmpeg still
 * accepts the new spelling and that existing projects render the same picture
 * before updating the expectation.
 */
import { describe, expect, test } from "vitest";

import { buildEffectChain, EFFECTS, findEffect } from "./effects";

const one = (id: string, params: Record<string, number> = {}) =>
  buildEffectChain([{ id, params }]);

/** The definition's sliders, all pushed to one bound. */
const bound = (id: string, which: "min" | "max"): Record<string, number> =>
  Object.fromEntries(findEffect(id)!.params.map((param) => [param.key, param[which]]));

const DEFAULT_CHAINS: Record<string, string> = {
  "black-white": "hue=s=0",
  sepia: "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131",
  invert: "negate",
  sharpen: "unsharp=5:5:1.00:5:5:0",
  "gaussian-blur": "gblur=sigma=10.0",
  "box-blur": "boxblur=6:1",
  "motion-blur": "gblur=sigma=18.0:sigmaV=0.1",
  warm: "colortemperature=temperature=4600",
  cool: "colortemperature=temperature=8500",
  vibrance: "vibrance=intensity=0.70",
  "contrast-pop": "eq=contrast=1.25",
  vignette: "vignette=angle=0.775",
  "film-grain": "noise=alls=12:allf=t+u",
  glow:
    "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];" +
    "[glowa0][glowg0]blend=all_mode=screen:all_opacity=0.45",
  posterize: "lutrgb=r=trunc(val/64)*64:g=trunc(val/64)*64:b=trunc(val/64)*64",
  pixelate: "pixelize=width=16:height=16",
  mirror: "crop=iw/2:ih:0:0,split[mirl0][mirr0];[mirr0]hflip[mirf0];[mirl0][mirf0]hstack",
  fisheye: "lenscorrection=k1=-0.275:k2=-0.100:i=bilinear",
  shake: "crop=iw-24:ih-24:12+12*sin(t*13):12+12*cos(t*17)",
};

const MIN_CHAINS: Record<string, string> = {
  sharpen: "unsharp=5:5:0.20:5:5:0",
  "gaussian-blur": "gblur=sigma=1.0",
  "box-blur": "boxblur=1:1",
  "motion-blur": "gblur=sigma=2.0:sigmaV=0.1",
  warm: "colortemperature=temperature=3000",
  cool: "colortemperature=temperature=7000",
  vibrance: "vibrance=intensity=0.10",
  "contrast-pop": "eq=contrast=1.00",
  vignette: "vignette=angle=0.355",
  "film-grain": "noise=alls=2:allf=t+u",
  glow:
    "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];" +
    "[glowa0][glowg0]blend=all_mode=screen:all_opacity=0.10",
  posterize: "lutrgb=r=trunc(val/128)*128:g=trunc(val/128)*128:b=trunc(val/128)*128",
  pixelate: "pixelize=width=2:height=2",
  fisheye: "lenscorrection=k1=-0.028:k2=-0.010:i=bilinear",
  shake: "crop=iw-4:ih-4:2+2*sin(t*2):2+2*cos(t*3)",
};

const MAX_CHAINS: Record<string, string> = {
  sharpen: "unsharp=5:5:3.00:5:5:0",
  "gaussian-blur": "gblur=sigma=50.0",
  "box-blur": "boxblur=30:1",
  "motion-blur": "gblur=sigma=60.0:sigmaV=0.1",
  warm: "colortemperature=temperature=6000",
  cool: "colortemperature=temperature=11000",
  vibrance: "vibrance=intensity=2.00",
  "contrast-pop": "eq=contrast=2.00",
  vignette: "vignette=angle=1.300",
  "film-grain": "noise=alls=40:allf=t+u",
  glow:
    "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];" +
    "[glowa0][glowg0]blend=all_mode=screen:all_opacity=1.00",
  posterize: "lutrgb=r=trunc(val/32)*32:g=trunc(val/32)*32:b=trunc(val/32)*32",
  pixelate: "pixelize=width=64:height=64",
  fisheye: "lenscorrection=k1=-0.550:k2=-0.200:i=bilinear",
  shake: "crop=iw-80:ih-80:40+40*sin(t*30):40+40*cos(t*39)",
};

describe("the effect catalogue's export strings", () => {
  test("this suite knows every effect - a new effect must pin its strings here", () => {
    expect(Object.keys(DEFAULT_CHAINS).sort()).toEqual(
      EFFECTS.map((effect) => effect.id).sort(),
    );
    const parameterised = EFFECTS.filter((effect) => effect.params.length > 0).map(
      (effect) => effect.id,
    );
    expect(Object.keys(MIN_CHAINS).sort()).toEqual([...parameterised].sort());
    expect(Object.keys(MAX_CHAINS).sort()).toEqual([...parameterised].sort());
  });

  test("every effect at its default settings", () => {
    for (const effect of EFFECTS) {
      // No params passed: resolveEffectParams must fill every default.
      expect(one(effect.id), effect.id).toBe(DEFAULT_CHAINS[effect.id]);
    }
  });

  test("every effect at its sliders' minimum", () => {
    for (const [id, expected] of Object.entries(MIN_CHAINS)) {
      expect(one(id, bound(id, "min")), id).toBe(expected);
    }
  });

  test("every effect at its sliders' maximum", () => {
    for (const [id, expected] of Object.entries(MAX_CHAINS)) {
      expect(one(id, bound(id, "max")), id).toBe(expected);
    }
  });
});

describe("buildEffectChain composition", () => {
  test("stacked effects join with commas in applied order", () => {
    expect(
      buildEffectChain([
        { id: "gaussian-blur", params: {} },
        { id: "black-white", params: {} },
      ]),
    ).toBe("gblur=sigma=10.0,hue=s=0");
    // The reverse order is a different picture and a different string.
    expect(
      buildEffectChain([
        { id: "black-white", params: {} },
        { id: "gaussian-blur", params: {} },
      ]),
    ).toBe("hue=s=0,gblur=sigma=10.0");
  });

  test("stacking one labelled effect twice keeps its graph labels distinct", () => {
    // Labels embed the emitted index. With the old fixed labels, two glows
    // (or two mirrors) on one clip duplicated [glowa]/[mirl] in a single
    // filtergraph and FFmpeg rejected it - the whole export failed.
    const chain = buildEffectChain([
      { id: "glow", params: {} },
      { id: "glow", params: {} },
    ]);
    expect(chain).toContain("[glowa0]");
    expect(chain).toContain("[glowa1]");
    // A bypassed entry does not consume an index: labels follow emission.
    const skipped = buildEffectChain([
      { id: "sepia", params: {}, enabled: false },
      { id: "mirror", params: {} },
    ]);
    expect(skipped).toContain("[mirl0]");
  });

  test("a bypassed effect contributes nothing", () => {
    expect(
      buildEffectChain([
        { id: "invert", params: {} },
        { id: "sepia", params: {}, enabled: false },
        { id: "black-white", params: {} },
      ]),
    ).toBe("negate,hue=s=0");
  });

  test("an unknown id is skipped, not exported as garbage", () => {
    // A project written by a newer WolfCut may carry effects this build does
    // not know. The chain must stay valid for the effects it does know.
    expect(
      buildEffectChain([
        { id: "from-the-future", params: {} },
        { id: "invert", params: {} },
      ]),
    ).toBe("negate");
  });

  test("no effects means null, never an empty string", () => {
    expect(buildEffectChain(undefined)).toBeNull();
    expect(buildEffectChain([])).toBeNull();
    expect(buildEffectChain([{ id: "invert", params: {}, enabled: false }])).toBeNull();
  });

  test("stray parameter keys are dropped, missing ones default", () => {
    expect(one("sharpen", { amount: 2, bogus: 99 })).toBe("unsharp=5:5:2.00:5:5:0");
    expect(one("shake", { amount: 20 })).toBe(
      "crop=iw-40:ih-40:20+20*sin(t*13):20+20*cos(t*17)",
    );
  });
});
