// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// RGB Split: the three channels part sideways as the cut nears, red one
// way and blue the other, and each changes to the incoming shot a beat
// apart - red first, then green, then blue - so the swap flashes through
// colour before the channels snap back into register.

struct Params { spread: f32 }

// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}

// One channel of the cut: offset by `shift` pixels, switching at `at`.
fn channel(uv: vec2<f32>, p: f32, shift: f32, at: f32) -> vec2<f32> {
    let moved = uv + vec2<f32>(shift / frame.size.x, 0.0);
    let w = smoothstep(at - 0.08, at + 0.08, p);
    let a = from_at(moved);
    let b = to_at(moved);
    return vec2<f32>(mix(a.r, b.r, w), mix(a.a, b.a, w));
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    // Up to a twelfth of the height apart, at full spread.
    let shift = clamp(params.spread, 0.0, 100.0) / 100.0 * frame.size.y / 12.0 * bell(p);
    let wr = smoothstep(0.34, 0.5, p);
    let wg = smoothstep(0.42, 0.58, p);
    let wb = smoothstep(0.5, 0.66, p);
    let red = mix(from_at(uv + vec2<f32>(shift / frame.size.x, 0.0)), to_at(uv + vec2<f32>(shift / frame.size.x, 0.0)), wr);
    let green = mix(from_at(uv), to_at(uv), wg);
    let blue = mix(from_at(uv - vec2<f32>(shift / frame.size.x, 0.0)), to_at(uv - vec2<f32>(shift / frame.size.x, 0.0)), wb);
    return vec4<f32>(red.r, green.g, blue.b, max(max(red.a, green.a), blue.a));
}
