// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Iris: a circle that is round on any frame, growing from nothing to
// past the corners, its rim softened in pixels so a hard iris is still
// clean.

struct Params { softness: f32 }

// Smoothstep's curve: leaves and lands with no jolt.
fn ease(p: f32) -> f32 {
    let q = clamp(p, 0.0, 1.0);
    return q * q * (3.0 - 2.0 * q);
}
// A point in pixels about the centre of the frame, and back.
fn centred(uv: vec2<f32>) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) * frame.size;
}

fn uncentred(d: vec2<f32>) -> vec2<f32> {
    return d / frame.size + vec2<f32>(0.5);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let corner = 0.5 * length(frame.size);
    let half = max(clamp(params.softness, 0.0, 100.0) / 100.0 * frame.size.y / 8.0, 0.75);
    let radius = mix(-half, corner + half, ease(p));
    let w = 1.0 - smoothstep(radius - half, radius + half, length(centred(uv)));
    return mix(from_at(uv), to_at(uv), w);
}
