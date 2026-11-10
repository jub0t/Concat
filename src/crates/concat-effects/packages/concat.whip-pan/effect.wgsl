// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Whip Pan: the camera flicks from one shot to the next. The shots sit
// side by side and the frame is dragged across, the drag packed into a
// shorter stretch of the cut the higher the speed, and smeared along its
// way in proportion to how fast it moves - a long streak at the flick,
// sharp at the ends.

struct Params { speed: f32 }

const TAPS: i32 = 24;

// The pan's position, nought to one, at `p`: still, a flick, still.
fn pan(p: f32, window: f32) -> f32 {
    let q = clamp((p - 0.5) / window + 0.5, 0.0, 1.0);
    return q * q * (3.0 - 2.0 * q);
}

// The strip of two shots, the outgoing on the left, at pan `t`.
fn strip(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let x = uv.x + t;
    var c = from_at(vec2<f32>(x, uv.y));
    if (x > 1.0) {
        c = to_at(vec2<f32>(x - 1.0, uv.y));
    }
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let speed = clamp(params.speed, 20.0, 100.0) / 100.0;
    // The flick takes from nine tenths of the cut at 20% to a quarter at full.
    let window = mix(1.06, 0.25, speed);
    let t = pan(p, window);
    // How far the pan moves in a frame of a one-second cut, here.
    let q = clamp((p - 0.5) / window + 0.5, 0.0, 1.0);
    let rate = 6.0 * q * (1.0 - q) / window;
    let smear = min(rate / 30.0 * 1.6, 0.6);
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let off = (f32(k) / f32(TAPS - 1) - 0.5) * smear;
        gathered += strip(uv + vec2<f32>(off, 0.0), t);
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
