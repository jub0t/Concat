struct Params { amount: f32 }

// An unsharp mask on the display level: the picture's difference from a
// small Gaussian blur of it, added back `amount` times over. An edge is
// judged by eye, so it is sharpened where the eye sees it; a level past
// white is carried, never clipped.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let c = sample(uv);
    var blur = vec3<f32>(0.0);
    var total = 0.0;
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            let w = exp(-f32(x * x + y * y) / 2.0);
            blur += sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb * w;
            total += w;
        }
    }
    return vec4<f32>(c.rgb + (c.rgb - blur / total) * params.amount, c.a);
}
