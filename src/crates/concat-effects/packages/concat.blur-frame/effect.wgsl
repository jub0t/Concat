// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Blur Frame: the picture shrunk to its share of the frame and set in
// the middle, over itself enlarged to cover the frame and blurred. The
// blur is drawn at an eighth of the layer's size each way - a box down
// into it, then a Gaussian across and down - so it costs little however
// wide, and is read back up smoothly. The light is summed premultiplied,
// so a see-through edge adds none.

struct Params { size: f32, blur: f32, shade: f32 }

// The background is the picture enlarged by this much about the middle,
// so its blurred edge never shows.
const ENLARGE: f32 = 1.12;

fn small(uv: vec2<f32>) -> vec4<f32> {
    let at = (uv - vec2<f32>(0.5)) / ENLARGE + vec2<f32>(0.5);
    let t = texel() / ENLARGE;
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            sum += sample_premultiplied(at + (vec2<f32>(f32(x), f32(y)) - vec2<f32>(1.5)) * 2.0 * t);
        }
    }
    return sum / 16.0;
}

fn sigma() -> f32 {
    return mix(1.0, 6.0, clamp(params.blur, 0.0, 100.0) / 100.0);
}

fn weight(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

fn across(uv: vec2<f32>) -> vec4<f32> {
    let width = 1.0 / small_texel().x;
    let s = sigma();
    let centre = uv.x * width;
    let first = floor(centre - 3.0 * s - 0.5) + 0.5;
    let pairs = i32(ceil(3.0 * s)) + 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        let w = wa + wb;
        sum += small_at(vec2<f32>((a + wb / w) / width, uv.y)) * w;
        total += w;
    }
    return sum / total;
}

fn down(uv: vec2<f32>) -> vec4<f32> {
    let rows = 1.0 / across_texel().y;
    let s = sigma();
    let centre = uv.y * rows;
    let first = floor(centre - 3.0 * s - 0.5) + 0.5;
    let pairs = i32(ceil(3.0 * s)) + 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        let w = wa + wb;
        sum += across_at(vec2<f32>(uv.x, (a + wb / w) / rows)) * w;
        total += w;
    }
    return sum / total;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let share = clamp(params.size, 40.0, 95.0) / 100.0;
    let inner = (uv - vec2<f32>(0.5)) / share + vec2<f32>(0.5);
    let back = down_at(uv);
    let dark = 1.0 - clamp(params.shade, 0.0, 100.0) / 100.0 * 0.7;
    // How far outside the picture `uv` is, in pixels, for its shadow.
    let half = vec2<f32>(0.5) * share * frame.size;
    let off = max(abs(uv - vec2<f32>(0.5)) * frame.size - half, vec2<f32>(0.0));
    let shadow = 1.0 - 0.5 * exp(-length(off) / (0.012 * frame.size.y));
    var ground = vec4<f32>(back.rgb / max(back.a, 1e-6) * dark * shadow, back.a);
    // The picture's edge softened over a pixel.
    let edge_px = (vec2<f32>(0.5) - abs(inner - vec2<f32>(0.5))) * share * frame.size;
    let cover = clamp(min(edge_px.x, edge_px.y) + 0.5, 0.0, 1.0);
    let front = sample(clamp(inner, vec2<f32>(0.0), vec2<f32>(1.0)));
    return mix(ground, front, cover);
}
