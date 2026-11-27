// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Mosaic: the picture coarsens into square tiles, largest at the middle
// of the cut, and each tile turns from the last shot to the next at its
// own moment around that middle, so the change scatters across the frame
// instead of fading. Then the tiles shrink back into a sharp picture.

struct Params { size: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let bell = sin(3.14159265 * p);
    // The widest tile is the knob's share of the frame's height.
    let widest = clamp(params.size, 2.0, 20.0) / 100.0 * frame.size.y;
    let cell = max(1.0, round(mix(1.0, widest, bell * bell)));
    let px = uv * frame.size;
    let tile = floor(px / cell);
    let centre = (tile + vec2<f32>(0.5)) * cell / frame.size;
    let at = select(uv, clamp(centre, vec2<f32>(0.0), vec2<f32>(1.0)), cell > 1.5);
    // Each tile's own moment, spread over the middle half of the cut.
    // Dealt on the widest tiles' grid, so it holds as the tiles grow.
    let moment = 0.25 + 0.5 * hash(floor(px / max(widest, 1.0)), 23.0);
    let turned = smoothstep(moment - 0.04, moment + 0.04, p);
    let shown = mix(from_at(at), to_at(at), turned);
    return shown;
}
