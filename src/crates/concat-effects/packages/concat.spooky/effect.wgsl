struct Params { dread: f32 }

// Less colour, a toe, sickly green through the middle, and the walls close in.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let dread = params.dread / 100.0;
    var out = saturation(c.rgb, 1.0 - dread * 0.6);
    out = film_curve(out, dread, 0.0);
    out = contrast(out, 1.0 + dread * 0.2);
    out = tint_midtones(out, vec3<f32>(-0.08, 0.1, 0.06), dread);
    out = vignette(out, uv, dread * 0.85);
    return vec4<f32>(clamp01(out), c.a);
}
