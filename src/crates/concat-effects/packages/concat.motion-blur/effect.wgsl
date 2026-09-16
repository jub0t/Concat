struct Params { length: f32 }

// A horizontal streak of `length` pixels: taps along a line, Gaussian
// weighted so the ends fade.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let sigma = max(params.length, 0.5);
    let step = max(sigma / 6.0, 1.0);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var i: i32 = -12; i <= 12; i++) {
        let d = f32(i) * step;
        let w = exp(-(d * d) / (2.0 * sigma * sigma));
        sum += sample(uv + vec2<f32>(d, 0.0) * t) * w;
        weight += w;
    }
    return sum / weight;
}
