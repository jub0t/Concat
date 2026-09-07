struct Params { depth: f32, grain: f32 }

// A hard toe, colour down, blue in the dark and the middle, the lights left.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let d = params.depth / 100.0;
    var out = film_curve(c.rgb, d, 0.0);
    out = saturation(out, 1.0 - d * 0.35);
    out = split_tone(out, vec3<f32>(-0.05, 0.0, 0.12), vec3<f32>(0.0), d);
    out = tint_midtones(out, vec3<f32>(0.0, 0.0, 0.08), d);
    out = out + grain_at(uv, params.grain / 100.0 * 0.1);
    return vec4<f32>(clamp01(out), c.a);
}
