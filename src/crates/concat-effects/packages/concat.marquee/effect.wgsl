struct Params { lights: f32, depth: f32 }

// Deep cool blacks, then the bright saturated things only: richer and blooming.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let l = params.lights / 100.0;
    let d = params.depth / 100.0;
    var out = film_curve(c.rgb, d * 0.8, 0.0);
    out = split_tone(out, vec3<f32>(0.0, 0.0, 0.1), vec3<f32>(0.0), d);
    let lit = highlights(c.rgb) * smoothstep(0.15, 0.5, chroma_of(c.rgb)) * (1.0 - skin_mask(c.rgb));
    out = mix(out, saturation(out, 1.6), lit * l);
    out = halation(uv, out, 0.7, 8.0, vec3<f32>(1.0), l * 0.5);
    return vec4<f32>(clamp01(out), c.a);
}
