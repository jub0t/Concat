struct Params { matte: f32, cool: f32 }

// A high matte with a lifted middle, colour pulled back, cool whites.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let m = params.matte / 100.0;
    var out = lift_gamma_gain(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0 + m * 0.25), vec3<f32>(1.0));
    out = matte(out, m * 0.14, 1.0 - m * 0.05);
    out = saturation(out, 1.0 - m * 0.25);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(-0.03, 0.0, 0.06), params.cool / 100.0);
    return vec4<f32>(clamp01(out), c.a);
}
