// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Lens Flare: light added over the picture as a lens would add it, in
// frame heights so it is round on any frame. A hot core and a wide soft
// halo at the source, a thin streak across it, a faint ring with a
// split of colour in it, and ghosts - soft discs of the source's light -
// strung along the line from the source through the centre and out the
// other side. Nothing is sampled but the picture under it.

struct Params { amount: f32, x: f32, y: f32, warmth: f32 }

// A soft disc of `radius` about `centre`, its rim a little brighter.
fn ghost(p: vec2<f32>, centre: vec2<f32>, radius: f32) -> f32 {
    let r = length(p - centre) / radius;
    return (1.0 - smoothstep(0.75, 1.0, r)) * (0.6 + 0.4 * smoothstep(0.3, 0.95, r));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let aspect = frame.size.x / frame.size.y;
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let src = (vec2<f32>(params.x, params.y) / 100.0 - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let d = p - src;
    let r = length(d);
    let warm = clamp(params.warmth, 0.0, 100.0) / 100.0;
    let tint = mix(vec3<f32>(0.72, 0.86, 1.0), vec3<f32>(1.0, 0.76, 0.48), warm);
    var light = tint * (exp(-r * r / 0.0003) * 2.5 + 0.3 / (1.0 + (r / 0.05) * (r / 0.05)));
    light = light + tint * exp(-abs(d.y) / 0.004) * exp(-abs(d.x) / 0.4) * 0.7;
    let ring = exp(-((r - 0.24) / 0.012) * ((r - 0.24) / 0.012)) * 0.05;
    let split = vec3<f32>(sin(r * 80.0), sin(r * 80.0 + 2.1), sin(r * 80.0 + 4.2)) * 0.3 + vec3<f32>(0.9);
    light = light + split * ring;
    // The ghosts, from the source through the centre and past it.
    let towards = -src;
    let soft = mix(vec3<f32>(0.5, 0.8, 1.0), vec3<f32>(1.0, 0.6, 0.3), warm);
    light = light + soft * 0.05 * ghost(p, src + towards * 0.55, 0.035);
    light = light + tint * 0.035 * ghost(p, src + towards * 0.95, 0.07);
    light = light + vec3<f32>(0.6, 1.0, 0.7) * 0.05 * ghost(p, src + towards * 1.35, 0.02);
    light = light + soft * 0.03 * ghost(p, src + towards * 1.7, 0.12);
    light = light + vec3<f32>(1.0, 0.5, 0.7) * 0.04 * ghost(p, src + towards * 2.2, 0.05);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    return vec4<f32>(c.rgb + light * a * 1.3, c.a);
}
