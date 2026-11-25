// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Flicker: the light of the whole picture stepped at random, a few
// times a second. Most steps dim it a little, now and then one drops it
// hard or kicks it bright, which is how an unsteady lamp looks. The
// steps are eased into each other over a fifth of a step so they do not
// tear between frames.

struct Params { amount: f32, speed: f32 }

// The light, in stops, of step `n`.
fn level(n: f32) -> f32 {
    let r = hash(vec2<f32>(n, 17.0), 3.0);
    let dip = select(0.0, -1.6, hash(vec2<f32>(n, 29.0), 3.0) > 0.82);
    let kick = select(0.0, 0.7, hash(vec2<f32>(n, 41.0), 3.0) > 0.9);
    return (r - 0.65) * 0.9 + dip + kick;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let t = max(frame.clip_time, 0.0) * clamp(params.speed, 2.0, 30.0);
    let n = floor(t);
    let blend = smoothstep(0.8, 1.0, t - n);
    let stops = mix(level(n), level(n + 1.0), blend) * a;
    return vec4<f32>(exposure(c.rgb, stops), c.a);
}
