// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Strobe: a flash at a steady speed from the start of the clip. Each
// flash is on for a third of its beat and goes out with a quick fall, so
// it reads as a light snapping on and off rather than a pulse. A black
// colour makes it a blackout strobe instead.

struct Params { speed: f32, amount: f32, colour: vec4<f32> }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let phase = fract(max(frame.clip_time, 0.0) * clamp(params.speed, 1.0, 20.0));
    let on = (1.0 - smoothstep(0.28, 0.4, phase)) * a;
    let tint = from_display(clamp(params.colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(mix(c.rgb, tint, on), c.a);
}
