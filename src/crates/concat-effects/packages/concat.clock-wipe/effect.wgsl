// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Clock Wipe: a hand from the centre that starts at twelve and turns
// clockwise once round. Its edge is softened by its distance in pixels,
// so it is as crisp at the rim as near the middle.

// A point in pixels about the centre of the frame, and back.
fn centred(uv: vec2<f32>) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) * frame.size;
}

fn uncentred(d: vec2<f32>) -> vec2<f32> {
    return d / frame.size + vec2<f32>(0.5);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let d = centred(uv);
    // Angle clockwise from twelve, nought to one; y runs down the frame.
    let turned = fract(atan2(d.x, -d.y) / 6.28318531 + 1.0);
    let r = length(d);
    // The swept angle's distance from the hand, as an arc in pixels.
    let behind = (p - turned) * 6.28318531 * r;
    var w = smoothstep(-0.75, 0.75, behind);
    // Past the start line the hand's back edge is at twelve: keep the
    // seam there hard only until the hand has gone round.
    w = select(w, 1.0, p >= 1.0);
    return mix(from_at(uv), to_at(uv), w);
}
