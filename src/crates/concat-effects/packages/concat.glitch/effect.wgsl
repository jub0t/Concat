// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Glitch: the signal re-dealt a number of times a second. Some deals are
// calm; in the rest a grid of blocks of its own size is laid over the
// frame and some blocks are thrown sideways, a few with their colours
// turned round, thin lines tear, and red and blue slip off green - more
// in a thrown block. Scan lines sit over all of it.

struct Params { amount: f32, speed: f32, split: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let n = floor(frame.time * clamp(params.speed, 1.0, 30.0));
    let wild = step(0.6 - 0.5 * a, hash(vec2<f32>(n, 3.0), 7.0));
    let g = a * mix(0.4, 1.0, hash(vec2<f32>(n, 5.0), 7.0)) * wild;
    let grid = vec2<f32>(mix(3.0, 12.0, hash(vec2<f32>(n, 11.0), 7.0)), mix(6.0, 26.0, hash(vec2<f32>(n, 13.0), 7.0)));
    let cell = floor(uv * grid);
    let hit = step(1.0 - 0.4 * g, hash(cell + vec2<f32>(n * 7.0, n * 3.0), 17.0));
    var at = uv;
    at.x = at.x + (hash(cell, n + 19.0) - 0.5) * 0.25 * g * hit;
    let row = floor(uv.y * frame.size.y / 2.0);
    let tear = step(0.985 - 0.05 * g, hash(vec2<f32>(row, n), 23.0));
    at.x = fract(at.x + (hash(vec2<f32>(row, n), 29.0) - 0.5) * 0.1 * g * tear);
    let reach = (0.002 + 0.012 * g * (1.0 + 2.0 * hit)) * clamp(params.split, 0.0, 100.0) / 100.0;
    let shift = vec2<f32>(reach, 0.0);
    let c = sample(at);
    var rgb = vec3<f32>(sample(at + shift).r, c.g, sample(at - shift).b);
    let swap = hit * step(0.75, hash(cell, n + 31.0));
    rgb = mix(rgb, rgb.gbr, swap);
    let line = 1.0 - 0.2 * a * step(0.5, fract(uv.y * frame.size.y / 3.0));
    return vec4<f32>(rgb * line, c.a);
}
