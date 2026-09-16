struct Params { flash: f32, noise: f32 }

// The brights pushed to clipping, colour up, cyan in the whites, a hard unsharp, noise.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = params.flash / 100.0;
    var out = c.rgb + highlights(c.rgb) * f * 0.35;
    out = saturation(out, 1.0 + f * 0.3);
    out = contrast(out, 1.0 + f * 0.15);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(-0.02, 0.04, 0.06), f);
    out = out + (c.rgb - soften(uv, 2.0)) * (0.8 + f * 0.8);
    out = out + grain_at(uv, params.noise / 100.0 * 0.14);
    return vec4<f32>(clamp01(out), c.a);
}
