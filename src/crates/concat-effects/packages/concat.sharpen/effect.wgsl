struct Params { amount: f32 }

// Unsharp mask: the picture plus its difference from a small blur.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let c = sample(uv);
    var blur = vec3<f32>(0.0);
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            blur += sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
        }
    }
    blur /= 25.0;
    let sharp = c.rgb + (c.rgb - blur) * params.amount;
    return vec4<f32>(clamp(sharp, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
