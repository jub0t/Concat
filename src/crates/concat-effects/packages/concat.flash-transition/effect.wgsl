// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Flash: a burst of light centred on the cut. The pictures are pushed up
// in stops as it builds, so their highlights go first, and at its peak
// the frame is the flash's colour; the shots change hands under the
// glare. The burst is short - most of the cut is plain picture - so it
// reads as a flash, not a fade through white.

struct Params { colour: f32, strength: f32 }

// The flash's colour in light, by the knob's choice.
fn tint_of(choice: f32) -> vec3<f32> {
    let pick = i32(round(clamp(choice, 0.0, 2.0)));
    if (pick == 1) {
        return vec3<f32>(1.0, 0.72, 0.42);
    } else if (pick == 2) {
        return vec3<f32>(0.62, 0.8, 1.0);
    }
    return vec3<f32>(1.0);
}

// How lit the cut is at `p`: nought at both ends, one in the middle,
// climbing and falling steeply.
fn burst(p: f32) -> f32 {
    let d = abs(p - 0.5) * 2.0;
    let k = 5.0;
    return max(0.0, (exp(-k * d) - exp(-k)) / (1.0 - exp(-k)));
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let s = clamp(params.strength, 20.0, 100.0) / 100.0;
    let e = burst(p) * s;
    let base = mix(from_at(uv), to_at(uv), smoothstep(0.44, 0.56, p));
    let tint = tint_of(params.colour);
    let lit = exposure(base.rgb, 4.0 * e) * mix(vec3<f32>(1.0), tint, e);
    return vec4<f32>(mix(lit, tint, smoothstep(0.35, 1.0, e)), base.a);
}
