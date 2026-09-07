struct Params { glow: f32, warmth: f32 }

// White balance toward candlelight, then the split tone the hour has.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let glow = params.glow / 100.0;
    var out = white_balance(c.rgb, params.warmth);
    out = split_tone(out, vec3<f32>(0.06, 0.0, 0.04), vec3<f32>(0.15, 0.05, 0.0), glow);
    out = fade(out, glow * 0.04);
    return vec4<f32>(clamp01(out), c.a);
}
