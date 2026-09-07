struct Params { tone: f32, contrast: f32 }

// Nearly grey, bronze through the middle and a little into the light, the curve back.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let tone = params.tone / 100.0;
    var out = saturation(c.rgb, 1.0 - tone * 0.9);
    out = tint_midtones(out, vec3<f32>(0.12, 0.04, -0.12), tone);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.04, 0.01, -0.04), tone);
    out = s_curve(out, params.contrast / 100.0 * 0.6);
    return vec4<f32>(clamp01(out), c.a);
}
