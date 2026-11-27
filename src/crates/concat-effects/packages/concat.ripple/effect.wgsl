// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Ripple: a ring wave out from the centre of the frame that bends both
// shots as it passes, strongest in the middle of the cut. The next shot
// spreads out from the centre a little behind the leading ring, its edge
// softened by the wave itself.

struct Params { strength: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let s = clamp(params.strength, 0.0, 100.0) / 100.0;
    let aspect = frame.size.x / frame.size.y;
    let d = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let r = length(d);
    let reach = 0.5 * length(vec2<f32>(aspect, 1.0));
    // The leading ring, from the centre to past the corners.
    let front = p * (reach + 0.3);
    let behind_front = max(front - r, 0.0);
    // Rings behind the front, fading out behind it and over the cut.
    let wave = sin(behind_front * 42.0) * exp(-behind_front * 6.0)
        * step(r, front) * sin(3.14159265 * p);
    let way = d / max(r, 1e-4);
    let push = way * wave * 0.03 * s / vec2<f32>(aspect, 1.0);
    let at = uv + push;
    let w = smoothstep(0.0, 0.18, front - 0.15 - r);
    let c = mix(from_at(at), to_at(at), w);
    // The crests catch a little light.
    return vec4<f32>(c.rgb * (1.0 + 0.25 * s * wave), c.a);
}
