// SPDX-License-Identifier: CC0-1.0
// Diagonal Wipe: an edge running from the top-left corner to the bottom-right.

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let edge = (uv.x + uv.y) * 0.5;
    let w = smoothstep(edge - 0.02, edge + 0.02, p * 1.04 - 0.02);
    return mix(from_at(uv), to_at(uv), w);
}
