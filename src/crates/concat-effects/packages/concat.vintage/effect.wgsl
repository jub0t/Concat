struct Params { age: f32, tint: f32 }

// A matte at both ends, colour faded, greens turned to yellow, warm middle.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let age = params.age / 100.0;
    let t = params.tint / 100.0;
    var out = matte(c.rgb, age * 0.12, 1.0 - age * 0.06);
    out = saturation(out, 1.0 - age * 0.35);
    out = contrast(out, 1.0 - age * 0.1);
    out = hsl_band(out, 120.0, 70.0, -35.0 * t, 1.0 - t * 0.1, 1.0);
    out = tint_midtones(out, vec3<f32>(0.05, 0.03, -0.08), t);
    out = vignette(out, uv, age * 0.4);
    return vec4<f32>(clamp01(out), c.a);
}
