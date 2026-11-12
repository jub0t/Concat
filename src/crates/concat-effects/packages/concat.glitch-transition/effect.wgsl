// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Glitch: a broken signal. The frame is cut into horizontal bands of
// random heights that are re-dealt many times a second; each band is
// thrown sideways and shows one shot or the other, the incoming one more
// often as the cut goes on. Thrown bands lose register between their
// channels, and the whole thing calms to nothing at either end.

struct Params { amount: f32 }

// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}

// How many times the bands are re-dealt over the cut.
const DEALS: f32 = 18.0;

// The band `uv` falls in at deal `deal`, as a whole number: the frame cut
// at two scales so bands come thick and thin.
fn band(uv: vec2<f32>, deal: f32) -> f32 {
    let coarse = floor(uv.y * mix(6.0, 14.0, hash(vec2<f32>(deal, 1.0), 3.0)));
    let fine = floor(uv.y * 48.0);
    return select(coarse, 100.0 + fine, hash(vec2<f32>(coarse, deal), 7.0) > 0.7);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let amount = clamp(params.amount, 0.0, 100.0) / 100.0;
    let wild = amount * bell(p);
    let deal = floor(p * DEALS);
    let b = band(uv, deal);
    let roll = hash(vec2<f32>(b, deal), 11.0);
    // Only some bands are thrown, more in the thick of it.
    let thrown = step(1.0 - wild, hash(vec2<f32>(b, deal), 13.0));
    let shove = (roll - 0.5) * 0.3 * wild * thrown;
    let split = 0.012 * wild * thrown;
    // Which shot this band shows: the incoming one past a threshold that
    // falls as the cut goes on.
    let pick = step(hash(vec2<f32>(b, deal), 17.0), smoothstep(0.15, 0.85, p));
    let at = vec2<f32>(fract(uv.x + shove), uv.y);
    let r = mix(from_at(at + vec2<f32>(split, 0.0)), to_at(at + vec2<f32>(split, 0.0)), pick);
    let g = mix(from_at(at), to_at(at), pick);
    let bl = mix(from_at(at - vec2<f32>(split, 0.0)), to_at(at - vec2<f32>(split, 0.0)), pick);
    // A worn signal brightens a thrown band's edge a touch.
    let lift = 1.0 + 0.25 * thrown * wild;
    return vec4<f32>(vec3<f32>(r.r, g.g, bl.b) * lift, g.a);
}
