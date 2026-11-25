// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// A point folded back into the picture, as a mirror at each edge would.
fn fold(uv: vec2<f32>) -> vec2<f32> {
    let m = abs(uv - 2.0 * floor(uv * 0.5 + vec2<f32>(0.5)));
    return clamp(m, vec2<f32>(0.0), vec2<f32>(1.0));
}

// Kaleidoscope: about the centre, every direction folded into one wedge
// of the circle and reflected at its middle, so the slices meet edge to
// edge as a real kaleidoscope's mirrors make them. The picture is read
// from the wedge, turned over time and zoomed, and folded back at its
// own edges so nothing outside it ever shows.

struct Params { pieces: f32, spin: f32, zoom: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let n = clamp(round(params.pieces), 3.0, 16.0);
    let aspect = frame.size.x / frame.size.y;
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let r = length(p);
    let wedge = 6.28318531 / n;
    var turn = atan2(p.y, p.x) + frame.time * clamp(params.spin, 0.0, 100.0) / 100.0 * 0.8;
    turn = turn - wedge * floor(turn / wedge);
    turn = abs(turn - wedge * 0.5);
    let z = mix(1.0, 0.35, clamp(params.zoom, 0.0, 100.0) / 100.0);
    let q = vec2<f32>(cos(turn), sin(turn)) * r * z;
    return sample(fold(q / vec2<f32>(aspect, 1.0) + vec2<f32>(0.5, 0.5)));
}
