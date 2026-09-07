struct Params { fade: f32, warmth: f32 }

// A cream matte, warm light, cyan in the dark, soft corners.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = params.fade / 100.0;
    let w = params.warmth / 100.0;
    var out = matte(c.rgb, f * 0.1, 1.0 - f * 0.05);
    out = contrast(out, 1.0 - f * 0.1);
    out = saturation(out, 1.0 - f * 0.15);
    out = split_tone(out, vec3<f32>(-0.06, 0.05, 0.08) * f, vec3<f32>(0.1, 0.05, -0.06) * w, 1.0);
    out = mix(out, soften(uv, 3.0), smoothstep(0.5, 1.0, distance(uv, vec2<f32>(0.5)) * 1.4142) * f * 0.6);
    out = vignette(out, uv, f * 0.3);
    return vec4<f32>(clamp01(out), c.a);
}
