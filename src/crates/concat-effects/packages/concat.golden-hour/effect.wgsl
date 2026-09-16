struct Params { glow: f32, warmth: f32 }

// Balance toward candlelight without darkening, then the hour's split tone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let glow = params.glow / 100.0;
    var out = white_balance(c.rgb, params.warmth);
    out = split_tone(out, vec3<f32>(0.05, 0.0, 0.04), vec3<f32>(0.14, 0.05, 0.0), glow);
    out = matte(out, glow * 0.04, 1.0);
    return vec4<f32>(clamp01(out), c.a);
}
