// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// VHS Tracking: a tape's playback faults, on lines of a tape's count
// whatever the frame's size. Each line wanders sideways a little and the
// frame sways; a band of badly tracked lines rolls slowly down the frame,
// thrown sideways and full of snow; the last lines fray where the heads
// switch; colour smears to the right of what it belongs to.

struct Params { wear: f32, tracking: f32, noise: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let w = clamp(params.wear, 0.0, 100.0) / 100.0;
    let tr = clamp(params.tracking, 0.0, 100.0) / 100.0;
    let nz = clamp(params.noise, 0.0, 100.0) / 100.0;
    let t = frame.time;
    let field = floor(t * 30.0);
    let line = floor(uv.y * 480.0);
    var dx = sin(uv.y * 18.0 + t * 2.3) * 0.0012 * w
        + (hash(vec2<f32>(line, field), 3.0) - 0.5) * 0.0018 * w;
    // The rolling band, its distance from `uv` wrapped round the frame.
    let gap = fract(uv.y - fract(t * 0.09) + 0.5) - 0.5;
    let band = exp(-(gap / 0.035) * (gap / 0.035)) * tr;
    dx = dx + (hash(vec2<f32>(line, field), 7.0) - 0.3) * 0.05 * band;
    let fray = smoothstep(0.955, 0.975, uv.y) * w;
    dx = dx + fray * (0.025 + 0.015 * sin(uv.y * 300.0 + t * 40.0));
    let at = vec2<f32>(uv.x + dx, uv.y);
    let c = sample(at);
    let smear = vec2<f32>(0.001 + 0.004 * w, 0.0);
    let c1 = sample(at - smear).rgb;
    let c2 = sample(at - smear * 2.0).rgb;
    let y = luma(c.rgb);
    let chroma = (c.rgb - vec3<f32>(y)) * 0.4 + (c1 - vec3<f32>(luma(c1))) * 0.35
        + (c2 - vec3<f32>(luma(c2))) * 0.25;
    var rgb = vec3<f32>(y) + chroma * (1.0 + 0.3 * w);
    let snow = hash(floor(uv * frame.size / vec2<f32>(2.0, 1.0)), fract(t * 7.31)) - 0.5;
    rgb = rgb + vec3<f32>(snow * (0.05 * nz + 0.5 * band * nz + 0.4 * fray * nz) * (0.25 + min(y, 1.0)));
    let fleck = step(0.996 - 0.04 * band, hash(vec2<f32>(floor(uv.x * frame.size.x / 24.0), line + field * 512.0), 11.0));
    rgb = rgb + vec3<f32>(fleck * 0.7 * nz * (band + 0.05));
    return vec4<f32>(rgb, c.a);
}
