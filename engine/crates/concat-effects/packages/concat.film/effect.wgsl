struct Params { grain: f32, warmth: f32 }

// Toe and shoulder, a warm middle, and the grain on top.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    var out = s_curve(fade(c.rgb, 0.03), 0.35) * 0.985;
    out = tint_midtones(out, vec3<f32>(0.06, 0.0, -0.06), params.warmth / 100.0);
    out = out + grain_at(uv, params.grain / 100.0 * 0.12);
    return vec4<f32>(clamp01(out), c.a);
}
