// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Zoom Pulse: on each beat from the start of the clip the picture jumps
// in by up to a fifth and eases back out over the beat. The camera's
// shutter is open over a short span of that move, so the punch streaks
// outward from the middle while it is fast and the picture is sharp when
// it is still.

struct Params { bpm: f32, amount: f32, blur: f32 }

fn zoom_at(t: f32, beat: f32, a: f32) -> f32 {
    let since = max(t, 0.0) - floor(max(t, 0.0) / beat) * beat;
    return 1.0 + 0.2 * a * exp(-5.0 * since / beat);
}

const TAPS: i32 = 9;

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let beat = 60.0 / clamp(params.bpm, 40.0, 240.0);
    let open = beat * 0.25 * clamp(params.blur, 0.0, 100.0) / 100.0;
    let t = max(frame.clip_time, 0.0);
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let when = t - open * f32(k) / f32(TAPS - 1);
        let z = zoom_at(max(when, 0.0), beat, a);
        sum += sample_premultiplied((uv - vec2<f32>(0.5)) / z + vec2<f32>(0.5));
    }
    let mean = sum / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
