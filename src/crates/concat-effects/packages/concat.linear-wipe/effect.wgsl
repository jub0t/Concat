// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Linear Wipe: a line sweeps across, the next shot behind it. The edge's
// softness is measured in the frame's height, so it is as wide on a tall
// frame as on a wide one, and never narrower than a pixel, so a hard wipe
// is still smooth.

struct Params { direction: f32, softness: f32 }

// How far `uv` lies along the wipe's travel, in pixels, and the travel's
// whole length.
fn along(uv: vec2<f32>, choice: i32) -> vec2<f32> {
    var at = uv.x;
    var length = frame.size.x;
    if (choice == 1) {
        at = 1.0 - uv.x;
    } else if (choice == 2) {
        at = uv.y;
        length = frame.size.y;
    } else if (choice == 3) {
        at = 1.0 - uv.y;
        length = frame.size.y;
    }
    return vec2<f32>(at * length, length);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let choice = i32(round(clamp(params.direction, 0.0, 3.0)));
    let pos = along(uv, choice);
    // Half a soft edge, in pixels: a third of the height at full softness.
    let half = max(clamp(params.softness, 0.0, 100.0) / 100.0 * frame.size.y / 6.0, 0.75);
    // The line starts wholly before the frame and ends wholly past it.
    let line = mix(-half, pos.y + half, p);
    let w = 1.0 - smoothstep(line - half, line + half, pos.x);
    return mix(from_at(uv), to_at(uv), w);
}
