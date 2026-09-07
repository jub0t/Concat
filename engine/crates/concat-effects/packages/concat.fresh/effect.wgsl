struct Params { pop: f32, brightness: f32 }

// Midtones up, vibrance rather than saturation so skin does not go orange,
// and a mint cast only in the whites.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pop = params.pop / 100.0;
    var out = lift_gamma_gain(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0 + params.brightness / 100.0 * 0.25), vec3<f32>(1.0));
    out = vibrance(out, pop * 0.9);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.0, 0.05, 0.03), pop);
    return vec4<f32>(clamp01(out), c.a);
}
