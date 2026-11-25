// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Old Film: an aged print on a tired projector, every frame of it a
// function of the frame's own time at 24 a second. The gate weaves a
// little, the lamp flickers, the colour fades towards a warm mono, grain
// is laid in log where film's is, the corners darken; scratches run down
// the frame for a few frames each, broken along their length, and specks
// of dust land and are gone.

struct Params { age: f32, scratches: f32, grain: f32 }

// One scratch, `k` of three: how much of it covers the pixel at `px`
// (pixels), and whether it is light (1) or dark (0).
fn scratch(px: vec2<f32>, k: f32, s: f32) -> vec2<f32> {
    let life = floor(frame.time * 5.0 + k * 0.37);
    let seed = vec2<f32>(life, k);
    let present = step(1.0 - s * 0.7, hash(seed, 41.0));
    let x = hash(seed, 43.0) * frame.size.x + sin(frame.time * 3.0 + k) * 4.0;
    let width = (0.7 + 1.3 * hash(seed, 47.0)) * frame.size.y / 1080.0;
    let cover = 1.0 - smoothstep(width * 0.5, width + 0.5, abs(px.x - x));
    let whole = step(0.3, hash(vec2<f32>(floor(px.y / frame.size.y * 9.0 + life), k), 53.0));
    return vec2<f32>(cover * whole * present, step(0.5, hash(seed, 59.0)));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let a = clamp(params.age, 0.0, 100.0) / 100.0;
    let s = clamp(params.scratches, 0.0, 100.0) / 100.0;
    let gr = clamp(params.grain, 0.0, 100.0) / 100.0;
    let f = floor(frame.time * 24.0);
    let weave = (vec2<f32>(hash(vec2<f32>(f, 1.0), 3.0), hash(vec2<f32>(f, 2.0), 3.0)) - vec2<f32>(0.5))
        * vec2<f32>(0.0015, 0.002) * a;
    let c = sample(uv + weave);
    var rgb = c.rgb;
    rgb = mix(rgb, vec3<f32>(luma(rgb)) * vec3<f32>(1.08, 0.97, 0.8), 0.4 * a);
    rgb = exposure(rgb, (hash(vec2<f32>(f, 3.0), 5.0) - 0.5) * 0.3 * a);
    let n = hash(floor(uv * frame.size), fract(frame.time * 7.31)) - 0.5;
    rgb = from_log(to_log(rgb) + vec3<f32>(n * gr * 0.07));
    let d = (uv - vec2<f32>(0.5)) * vec2<f32>(frame.size.x / frame.size.y, 1.0);
    rgb = rgb * (1.0 - a * 0.55 * smoothstep(0.3, 1.0, length(d)));
    let px = uv * frame.size;
    for (var k = 0; k < 3; k++) {
        let line = scratch(px, f32(k), s);
        let marked = select(rgb * 0.3, rgb + vec3<f32>(0.3), line.y > 0.5);
        rgb = mix(rgb, marked, line.x * 0.8);
    }
    let grid = vec2<f32>(32.0, 18.0);
    let cell = floor(uv * grid);
    let local = fract(uv * grid);
    let has = step(1.0 - 0.08 * s, hash(cell, f + 61.0));
    let centre = vec2<f32>(hash(cell, f + 67.0), hash(cell, f + 71.0));
    let cell_px = frame.size / grid;
    let r = (0.04 + 0.1 * hash(cell, f + 73.0));
    let dd = length((local - centre) * vec2<f32>(cell_px.x / cell_px.y, 1.0));
    let speck = (1.0 - smoothstep(r * 0.5, r, dd)) * has;
    rgb = mix(rgb, rgb * 0.12, speck * 0.85);
    return vec4<f32>(rgb, c.a);
}
