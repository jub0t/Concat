struct Params { lift: f32, clarity: f32 }

// The darks are softened before they are lifted, so grain stays down.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let lift = params.lift / 100.0;
    let soft = soften(uv, 3.0);
    let base = mix(c.rgb, soft, shadows(c.rgb) * params.clarity / 100.0);
    let out = lift_gamma_gain(base, vec3<f32>(lift * 0.05), vec3<f32>(1.0 + lift * 0.45), vec3<f32>(1.0));
    return vec4<f32>(clamp01(out), c.a);
}
