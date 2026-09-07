struct Params { age: f32, tint: f32 }

// A fade at both ends, less colour, the print's yellow-green, the corners.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let age = params.age / 100.0;
    var out = fade(c.rgb, age * 0.12) * (1.0 - age * 0.06);
    out = saturation(out, 1.0 - age * 0.35);
    out = contrast(out, 1.0 - age * 0.12);
    out = tint_midtones(out, vec3<f32>(0.06, 0.06, -0.1), params.tint / 100.0);
    out = vignette(out, uv, age * 0.4);
    return vec4<f32>(clamp01(out), c.a);
}
