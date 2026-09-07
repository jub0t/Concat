struct Params { colour: f32, punch: f32 }

// An S-curve, vibrance that leaves skin be, and the picture minus its blur.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let punch = params.punch / 100.0;
    let keep = 1.0 - skin_mask(c.rgb) * 0.6;
    var out = s_curve(c.rgb, punch * 0.7);
    out = mix(out, vibrance(out, params.colour / 100.0 * 1.4), keep);
    out = out + (c.rgb - soften(uv, 2.0)) * punch * 0.7;
    return vec4<f32>(clamp01(out), c.a);
}
