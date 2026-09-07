struct Params { water: f32, brightness: f32 }

// Blues turned toward cyan and lifted, cyans richer, everything a touch brighter.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let water = params.water / 100.0;
    var out = lift_gamma_gain(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0 + params.brightness / 100.0 * 0.2), vec3<f32>(1.0));
    out = hsl_band(out, 225.0, 60.0, -25.0 * water, 1.0 + water * 0.4, 1.0 + water * 0.12);
    out = hsl_band(out, 185.0, 50.0, 0.0, 1.0 + water * 0.5, 1.0 + water * 0.08);
    out = saturation(out, 1.0 + water * 0.06);
    return vec4<f32>(clamp01(out), c.a);
}
