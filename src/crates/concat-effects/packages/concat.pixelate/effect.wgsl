struct Params { size: f32 }

// Squares `size` pixels across, each the average of the light inside it -
// sixteen bilinear taps spread over the square, each four pixels - rather
// than whichever pixel happened to be at its middle.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let block = max(round(params.size), 1.0);
    let corner = floor(uv * frame.size / block) * block;
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let at = corner + (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) * block / 4.0;
            sum += sample_premultiplied(at / frame.size);
        }
    }
    let mean = sum / 16.0;
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
