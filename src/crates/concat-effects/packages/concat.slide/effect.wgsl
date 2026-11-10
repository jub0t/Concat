// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Slide: the next shot is a card dealt over the last one. It arrives fast
// and slows to a stop, and the shot underneath darkens a little in the
// shadow along its leading edge.

struct Params { direction: f32 }

// Fast out of the gate and gently to rest: a cubic ease-out.
fn arrive(p: f32) -> f32 {
    let q = 1.0 - clamp(p, 0.0, 1.0);
    return 1.0 - q * q * q;
}

// The way the card travels, one frame long.
fn heading(choice: f32) -> vec2<f32> {
    let pick = i32(round(clamp(choice, 0.0, 3.0)));
    var way = vec2<f32>(-1.0, 0.0);
    if (pick == 1) {
        way = vec2<f32>(1.0, 0.0);
    } else if (pick == 2) {
        way = vec2<f32>(0.0, -1.0);
    } else if (pick == 3) {
        way = vec2<f32>(0.0, 1.0);
    }
    return way;
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let t = arrive(progress);
    let way = heading(params.direction);
    let card = uv + way * (1.0 - t);
    let inside = all(card >= vec2<f32>(0.0)) && all(card <= vec2<f32>(1.0));
    if (inside) {
        return to_at(card);
    }
    // How far ahead of the card's edge this point is, in pixels.
    let gap = dot(card - clamp(card, vec2<f32>(0.0), vec2<f32>(1.0)), way) * dot(abs(way), frame.size);
    let shade = 1.0 - 0.35 * exp(-gap / (0.04 * frame.size.y)) * min(t * 4.0, 1.0);
    let under = from_at(uv);
    return vec4<f32>(under.rgb * shade, under.a);
}
