struct Params { grain: f32, warmth: f32 }

// Toe and shoulder, a warm middle, halation on the lights, grain on top.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let w = params.warmth / 100.0;
    var out = matte(film_curve(c.rgb, 0.6, 0.6), 0.03, 0.97);
    out = tint_midtones(out, vec3<f32>(0.06, 0.0, -0.06), w);
    out = halation(uv, out, 0.7, 8.0, vec3<f32>(1.0, 0.6, 0.3), 0.15 + w * 0.1);
    out = out + grain_at(uv, params.grain / 100.0 * 0.12);
    return vec4<f32>(clamp01(out), c.a);
}
