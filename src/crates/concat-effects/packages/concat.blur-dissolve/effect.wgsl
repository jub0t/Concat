// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Blur Dissolve: each shot is defocused by a disc whose size swells to the
// middle of the cut and falls away after it; the mix between them is held
// to the blurriest stretch, so the change itself is never seen sharp.

struct Params { strength: f32 }

// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}
// Two colours mixed by `w`, premultiplied, in light: halfway from white
// to black is half the light, as a dissolve between two exposures is.
fn blend(a: vec4<f32>, b: vec4<f32>, w: f32) -> vec4<f32> {
    let alpha = mix(a.a, b.a, w);
    let rgb = mix(a.rgb * a.a, b.rgb * b.a, w);
    return vec4<f32>(rgb / max(alpha, 1e-6), alpha);
}

// Taps on a sunflower spiral: an even disc from few samples.
const TAPS: i32 = 24;
const GOLDEN: f32 = 2.39996323;

// A picture defocused by a disc of `radius` pixels. `side` picks which:
// nought the outgoing shot, one the incoming.
fn defocus(uv: vec2<f32>, radius: f32, side: i32) -> vec4<f32> {
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let r = sqrt((f32(k) + 0.5) / f32(TAPS)) * radius;
        let a = f32(k) * GOLDEN;
        let at = uv + vec2<f32>(cos(a), sin(a)) * r / frame.size;
        var c = from_at(at);
        if (side == 1) {
            c = to_at(at);
        }
        gathered += vec4<f32>(c.rgb * c.a, c.a);
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    // Up to a twentieth of the frame's height across, at full strength.
    let radius = clamp(params.strength / 100.0, 0.0, 1.0) * 0.05 * frame.size.y * bell(p);
    let w = smoothstep(0.3, 0.7, p);
    return blend(defocus(uv, radius, 0), defocus(uv, radius, 1), w);
}
