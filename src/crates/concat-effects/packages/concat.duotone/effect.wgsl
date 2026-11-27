// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Duotone: each pixel's brightness, as the eye judges it, picks a colour
// on the line from the shadow colour to the highlight colour, with a
// gentle S so the two colours hold their ground. Worked in the display
// encoding, where the swatches were picked; a level past white keeps the
// highlight colour lifted by as much.

struct Params { shadows: vec4<f32>, highlights: vec4<f32>, amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let y = luma(c.rgb);
    let inside = clamp(y, 0.0, 1.0);
    let shaped = mix(inside, inside * inside * (3.0 - 2.0 * inside), 0.5);
    let low = clamp(params.shadows.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let high = clamp(params.highlights.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let toned = mix(low, high, shaped) + vec3<f32>(max(y - 1.0, 0.0));
    return vec4<f32>(mix(c.rgb, toned, clamp(params.amount, 0.0, 100.0) / 100.0), c.a);
}
