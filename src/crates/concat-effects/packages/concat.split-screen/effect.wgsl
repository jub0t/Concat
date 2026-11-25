// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Split Screen: the frame cut into a grid of tiles, each the whole
// picture where its shape is the frame's and the picture's middle cropped
// to fit where it is not, so nothing is ever squashed. A gap of the gap's
// colour sits between the tiles and round them, half as wide at the edge.

struct Params { tiles: f32, gap: f32, colour: vec4<f32> }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let pick = i32(round(clamp(params.tiles, 0.0, 5.0)));
    var grid = vec2<f32>(2.0, 2.0);
    if (pick == 0) {
        grid = vec2<f32>(2.0, 1.0);
    } else if (pick == 1) {
        grid = vec2<f32>(1.0, 2.0);
    } else if (pick == 2) {
        grid = vec2<f32>(3.0, 1.0);
    } else if (pick == 3) {
        grid = vec2<f32>(1.0, 3.0);
    } else if (pick == 5) {
        grid = vec2<f32>(3.0, 3.0);
    }
    let cell = min(floor(uv * grid), grid - vec2<f32>(1.0));
    let local = uv * grid - cell;
    let tile = frame.size / grid;
    let shape = tile.x / tile.y;
    let whole = frame.size.x / frame.size.y;
    var pu = local - vec2<f32>(0.5);
    if (shape < whole) {
        pu.x = pu.x * shape / whole;
    } else {
        pu.y = pu.y * whole / shape;
    }
    let picture = sample(pu + vec2<f32>(0.5));
    let gap = clamp(params.gap, 0.0, 100.0) / 100.0 * 0.04 * frame.size.y;
    let edge = min(local, vec2<f32>(1.0) - local) * tile;
    let near = min(edge.x, edge.y);
    let inside = select(smoothstep(gap * 0.5 - 0.5, gap * 0.5 + 0.5, near), 1.0, gap < 0.5);
    let fill = vec4<f32>(from_display(clamp(params.colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
    return mix(fill, picture, inside);
}
