struct Params { sky: f32, greens: f32 }

// Two bands, each turned and weighted on its own; skin is in neither.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let sky = params.sky / 100.0;
    let greens = params.greens / 100.0;
    // Sky: toward azure, richer, a shade darker so it reads deep not bright.
    var out = hsl_band(c.rgb, 210.0, 70.0, -6.0 * sky, 1.0 + sky * 0.55, 1.0 - sky * 0.08);
    // Foliage: toward teal, quieter.
    out = hsl_band(out, 105.0, 60.0, 12.0 * greens, 1.0 - greens * 0.2, 1.0);
    return vec4<f32>(clamp01(out), c.a);
}
