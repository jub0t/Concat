// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Luma Wipe: the outgoing picture's own brightness is the map. A level
// that the moving threshold has passed shows the incoming shot, so the
// cut runs through the frame along the shapes in it - skies and lamps
// first, or the shadows first - with an edge as soft as asked.

struct Params { order: f32, softness: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let out_c = from_at(uv);
    let to = to_at(uv);
    // Brightness as the eye judges it, a touch of the incoming picture's
    // mixed in so a flat region does not turn over all at once.
    let seen = clamp(0.85 * to_display(vec3<f32>(luma(out_c.rgb))).x
        + 0.15 * to_display(vec3<f32>(luma(to.rgb))).x, 0.0, 1.0);
    let bright_first = round(clamp(params.order, 0.0, 1.0)) < 0.5;
    let v = select(seen, 1.0 - seen, bright_first);
    let s = max(clamp(params.softness, 0.0, 100.0) / 100.0 * 0.35, 0.01);
    let edge = p * (1.0 + 2.0 * s) - s;
    let w = 1.0 - smoothstep(edge - s, edge + s, v);
    return mix(out_c, to, w);
}
