// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Light Leak: light spilling past a loose film gate. A broad band of
// glow sweeps across on a slant, white-hot at its core and amber to red at
// its fringes (whiter as warmth falls); it is added to the picture as
// light is, and where it has passed the incoming shot shows.

struct Params { warmth: f32 }

// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let warmth = clamp(params.warmth, 0.0, 100.0) / 100.0;
    let aspect = frame.size.x / frame.size.y;
    // Distance along the sweep, slanted, nought to one across the frame.
    let x = (uv.x * aspect + (1.0 - uv.y) * 0.45) / (aspect + 0.45);
    let centre = mix(-0.35, 1.35, p);
    let d = x - centre;
    let core = exp(-d * d / 0.003);
    let glow = exp(-d * d / 0.02);
    let swap = 1.0 - smoothstep(-0.12, 0.12, d);
    let base = mix(from_at(uv), to_at(uv), swap);
    let amber = mix(vec3<f32>(1.0, 0.92, 0.8), vec3<f32>(1.0, 0.45, 0.12), warmth);
    let fringe = mix(vec3<f32>(1.0, 0.85, 0.75), vec3<f32>(0.95, 0.18, 0.08), warmth);
    let flare = (amber * glow * 0.9 + vec3<f32>(1.0, 0.97, 0.92) * core * 1.2
        + fringe * exp(-(d + 0.16) * (d + 0.16) / 0.012) * 0.35) * bell(p);
    return vec4<f32>(base.rgb + flare, base.a);
}
