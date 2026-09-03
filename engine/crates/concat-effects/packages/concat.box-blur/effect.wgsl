struct Params { radius: f32 }

// A box of `radius` pixels either side, sampled on a 9×9 grid.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let step = max(params.radius / 4.0, 0.5);
    var sum = vec4<f32>(0.0);
    for (var y: i32 = -4; y <= 4; y++) {
        for (var x: i32 = -4; x <= 4; x++) {
            sum += sample(uv + vec2<f32>(f32(x), f32(y)) * step * t);
        }
    }
    return sum / 81.0;
}
