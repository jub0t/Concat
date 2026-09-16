struct Params { threshold: f32 }

// Each channel above the threshold is flipped; below it is left alone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let t = vec3<f32>(params.threshold / 100.0);
    let flipped = select(c.rgb, vec3<f32>(1.0) - c.rgb, c.rgb > t);
    return vec4<f32>(flipped, c.a);
}
