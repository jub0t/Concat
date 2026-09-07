struct Params { chill: f32, contrast: f32 }

// The three-way version of Cool: blue where it is dark, not everywhere.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let chill = params.chill / 100.0;
    var out = split_tone(c.rgb, vec3<f32>(-0.08, 0.0, 0.25), vec3<f32>(0.0), chill);
    out = tint_midtones(out, vec3<f32>(0.0, 0.0, 0.12), chill);
    out = contrast(out, 1.0 + params.contrast / 100.0 * 0.4);
    return vec4<f32>(clamp01(out), c.a);
}
