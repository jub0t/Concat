struct Params { amount: f32, x: f32, y: f32 }

// Each pixel streaked along the line to the centre, over `amount` of its
// distance from it, in light: the picture drawn at every scale from its
// own down to `amount` smaller about the centre, at a steady rate of zoom,
// and averaged - nothing at the centre, the most at the edge. Three passes
// of sixteen scales, each averaging the scales between two of the next
// one's, so the four thousand and ninety-six of the whole leave no gap
// along even the longest streak of an 8K frame.

const TAPS: i32 = 16;

fn centre() -> vec2<f32> {
    return vec2<f32>(params.x, params.y) / 100.0;
}

/// The whole zoom, as a distance in log scale: nothing at 0, and never
/// quite as far as the centre.
fn span() -> f32 {
    return -log(1.0 - clamp(params.amount / 100.0, 0.0, 0.99));
}

/// `uv` drawn `by` of log scale smaller about the centre.
fn scaled(uv: vec2<f32>, by: f32) -> vec2<f32> {
    return centre() + (uv - centre()) * exp(-by);
}

fn fine(uv: vec2<f32>) -> vec4<f32> {
    let step = span() / 4096.0;
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < TAPS; i++) {
        sum += sample_premultiplied(scaled(uv, step * f32(i)));
    }
    return sum / f32(TAPS);
}

fn middle(uv: vec2<f32>) -> vec4<f32> {
    let step = span() / 256.0;
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < TAPS; i++) {
        sum += fine_at(scaled(uv, step * f32(i)));
    }
    return sum / f32(TAPS);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let step = span() / 16.0;
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < TAPS; i++) {
        sum += middle_at(scaled(uv, step * f32(i)));
    }
    let blurred = sum / f32(TAPS);
    return vec4<f32>(blurred.rgb / max(blurred.a, 1e-6), blurred.a);
}
