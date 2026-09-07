struct Params { sky: f32, greens: f32 }

// Two hue bands, each treated on its own and the rest untouched.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let blue = hue_mask(c.rgb, 215.0, 70.0) * params.sky / 100.0;
    let green = hue_mask(c.rgb, 110.0, 70.0) * params.greens / 100.0;
    var out = mix(c.rgb, saturation(c.rgb, 1.6) * 0.9, blue);
    out = mix(out, saturation(out, 0.85) + vec3<f32>(-0.03, 0.0, 0.05), green);
    return vec4<f32>(clamp01(out), c.a);
}
