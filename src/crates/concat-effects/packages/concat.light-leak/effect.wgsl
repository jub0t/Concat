// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Light Leak: two soft pools of coloured light that wander along the top
// and right edges of the frame, their shapes broken up by slow smooth
// noise, added to the picture as light. Every moment of it is worked out
// from the time alone.

struct Params { amount: f32, speed: f32, colour: f32 }

// Smooth noise in 0..1 on a grid of whole numbers.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    let a = hash(i, 9.0);
    let b = hash(i + vec2<f32>(1.0, 0.0), 9.0);
    let c = hash(i + vec2<f32>(0.0, 1.0), 9.0);
    let d = hash(i + vec2<f32>(1.0, 1.0), 9.0);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pick = i32(round(clamp(params.colour, 0.0, 3.0)));
    var hot = vec3<f32>(1.0, 0.42, 0.1);
    var soft = vec3<f32>(1.0, 0.72, 0.32);
    if (pick == 1) {
        hot = vec3<f32>(1.0, 0.25, 0.3);
        soft = vec3<f32>(1.0, 0.58, 0.5);
    } else if (pick == 2) {
        hot = vec3<f32>(1.0, 0.7, 0.2);
        soft = vec3<f32>(1.0, 0.88, 0.55);
    } else if (pick == 3) {
        hot = vec3<f32>(0.15, 0.75, 0.85);
        soft = vec3<f32>(0.5, 0.6, 1.0);
    }
    let t = frame.time * mix(0.05, 0.7, clamp(params.speed, 0.0, 100.0) / 100.0);
    let aspect = frame.size.x / frame.size.y;
    let p = uv * vec2<f32>(aspect, 1.0);
    let top = vec2<f32>(aspect * (0.15 + 0.7 * (0.5 + 0.5 * sin(t * 0.9))), -0.05 + 0.2 * (0.5 + 0.5 * sin(t * 0.6 + 1.3)));
    let side = vec2<f32>(aspect * (1.02 - 0.12 * (0.5 + 0.5 * sin(t * 0.7 + 2.0))), 0.25 + 0.6 * (0.5 + 0.5 * sin(t * 0.5 + 0.4)));
    let n = noise(p * 2.5 + vec2<f32>(t * 0.7, -t * 0.4));
    let m = noise(p * 1.3 - vec2<f32>(t * 0.3, t * 0.5) + vec2<f32>(7.0));
    let d1 = p - top;
    let d2 = p - side;
    let pool1 = exp(-dot(d1, d1) / (0.16 * (0.6 + 0.8 * n)));
    let pool2 = exp(-dot(d2, d2) / (0.1 * (0.6 + 0.8 * m)));
    let leak = hot * pool1 * (0.5 + n) + soft * pool2 * (0.4 + m);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    return vec4<f32>(c.rgb + leak * a * 1.4, c.a);
}
