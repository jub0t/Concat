// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Dissolve: the plainest cut there is - the two shots mixed in light,
// evenly over the cut, as two exposures on one frame would be.

// Two colours mixed by `w`, premultiplied, in light: halfway from white
// to black is half the light, as a dissolve between two exposures is.
fn blend(a: vec4<f32>, b: vec4<f32>, w: f32) -> vec4<f32> {
    let alpha = mix(a.a, b.a, w);
    let rgb = mix(a.rgb * a.a, b.rgb * b.a, w);
    return vec4<f32>(rgb / max(alpha, 1e-6), alpha);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    return blend(from_at(uv), to_at(uv), clamp(progress, 0.0, 1.0));
}
