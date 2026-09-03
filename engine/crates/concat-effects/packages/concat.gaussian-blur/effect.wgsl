struct Params { radius: f32 }

// A Gaussian of sigma `radius` pixels, sampled on a 9×9 grid whose spacing
// grows with the sigma; bilinear filtering fills the gaps at large radii.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let sigma = max(params.radius, 0.5);
    let step = max(sigma / 2.5, 1.0);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y: i32 = -4; y <= 4; y++) {
        for (var x: i32 = -4; x <= 4; x++) {
            let d = vec2<f32>(f32(x), f32(y)) * step;
            let w = exp(-dot(d, d) / (2.0 * sigma * sigma));
            sum += sample(uv + d * t) * w;
            weight += w;
        }
    }
    return sum / weight;
}
