struct Params { amount: f32 }

// A wide soft blur, screened over the picture by the amount.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let c = sample(uv);
    let step = 6.0;
    var blur = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y: i32 = -3; y <= 3; y++) {
        for (var x: i32 = -3; x <= 3; x++) {
            let d = vec2<f32>(f32(x), f32(y));
            let w = exp(-dot(d, d) / 8.0);
            blur += sample(uv + d * step * t).rgb * w;
            weight += w;
        }
    }
    blur /= weight;
    let screen = vec3<f32>(1.0) - (vec3<f32>(1.0) - c.rgb) * (vec3<f32>(1.0) - blur);
    return vec4<f32>(mix(c.rgb, screen, params.amount / 100.0), c.a);
}
