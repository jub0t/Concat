struct Params { glow: f32, softness: f32 }

// Skin turned a few degrees toward peach and lifted, softened only where
// it is skin, then a light matte and a warm highlight.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let glow = params.glow / 100.0;
    let skin = skin_mask(c.rgb);
    var out = mix(c.rgb, soften(uv, 4.0), skin * params.softness / 100.0 * 0.7);
    out = hsl_band(out, 20.0, 45.0, 6.0 * glow, 1.0 + glow * 0.1, 1.0 + glow * 0.1);
    out = matte(out, glow * 0.06, 1.0);
    out = contrast(out, 1.0 - glow * 0.1);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.05, 0.02, 0.0), glow);
    return vec4<f32>(clamp01(out), c.a);
}
