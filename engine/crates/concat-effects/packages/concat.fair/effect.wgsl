struct Params { brighten: f32, even: f32 }

// The skin band alone: lifted, its reds eased, a touch less saturated.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let b = params.brighten / 100.0;
    let e = params.even / 100.0;
    var out = hsl_band(c.rgb, 20.0, 45.0, 4.0 * e, 1.0 - e * 0.15, 1.0 + b * 0.18);
    out = lift_gamma_gain(out, vec3<f32>(0.0), vec3<f32>(1.0 + b * 0.08), vec3<f32>(1.0));
    return vec4<f32>(clamp01(out), c.a);
}
