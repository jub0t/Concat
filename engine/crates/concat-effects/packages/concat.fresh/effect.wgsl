struct Params { pop: f32, brightness: f32 }

// Brighter, richer, a mint cast in the highlights.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pop = params.pop / 100.0;
    var out = c.rgb + vec3<f32>(params.brightness / 100.0 * 0.12);
    out = saturation(out, 1.0 + pop * 0.5);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.0, 0.08, 0.04), pop);
    return vec4<f32>(clamp01(out), c.a);
}
