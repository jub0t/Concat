// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Blinds: the frame in strips that each turn on their middle, the last
// shot on their fronts and the next on their backs. A strip edge-on is a
// thin dark line; one turned part way is squeezed and shaded as a slat
// would be in light from the front. They start one after another, so a
// wave runs through them.

struct Params { direction: f32, count: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let n = clamp(round(params.count), 3.0, 24.0);
    let down = round(clamp(params.direction, 0.0, 1.0)) > 0.5;
    let a = select(uv.y, uv.x, down);
    let index = min(floor(a * n), n - 1.0);
    let local = a * n - index;
    // Each strip turns over 60 % of the cut, the last starting at 40 %.
    let q = clamp((p - 0.4 * index / max(n - 1.0, 1.0)) / 0.6, 0.0, 1.0);
    let turn = q * q * (3.0 - 2.0 * q);
    let face = cos(turn * 3.14159265);
    let width = max(abs(face), 1e-3);
    let off = (local - 0.5) / width;
    if (abs(off) > 0.5) {
        // Between the slats: the dark behind them.
        let gap = select(to_at(uv), from_at(uv), face > 0.0);
        return vec4<f32>(gap.rgb * 0.04, gap.a);
    }
    let a2 = (index + 0.5 + off) / n;
    var at = vec2<f32>(a2, uv.y);
    if (!down) {
        at = vec2<f32>(uv.x, a2);
    }
    let shown = select(to_at(at), from_at(at), face > 0.0);
    let light = mix(0.35, 1.0, width);
    return vec4<f32>(shown.rgb * light, shown.a);
}
