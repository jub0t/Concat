struct Params { bleed: f32, lines: f32 }

// Red and blue sampled off to either side, a smear downward, then the tape's marks.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let bleed = params.bleed / 100.0;
    let lines = params.lines / 100.0;
    let t = texel();
    let off = vec2<f32>(t.x * bleed * 3.0, 0.0);
    let c = sample(uv);
    var out = vec3<f32>(sample(uv - off).r, c.g, sample(uv + off).b);
    let smear = (sample(uv - vec2<f32>(0.0, t.y)).rgb + sample(uv - vec2<f32>(0.0, 2.0 * t.y)).rgb) * 0.5;
    out = mix(out, smear, bleed * 0.25);
    out = mix(out, soften(uv, 2.0), bleed * 0.3);
    out = saturation(out, 1.3);
    let row = fract(uv.y * frame.size.y / 3.0);
    out = out * (1.0 - lines * 0.25 * step(0.5, row));
    out = out + grain_at(uv, lines * 0.08);
    return vec4<f32>(clamp01(out), c.a);
}
