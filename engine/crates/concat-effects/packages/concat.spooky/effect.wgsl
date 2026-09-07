struct Params { dread: f32 }

// Less colour, more contrast, darker, sickly green, and the walls close in.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let dread = params.dread / 100.0;
    var out = c.rgb - vec3<f32>(dread * 0.08);
    out = saturation(out, 1.0 - dread * 0.6);
    out = contrast(out, 1.0 + dread * 0.4);
    out = tint_midtones(out, vec3<f32>(-0.08, 0.1, 0.06), dread);
    out = vignette(out, uv, dread * 0.85);
    return vec4<f32>(clamp01(out), c.a);
}
