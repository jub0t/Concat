struct Params { level: f32 }

// Brighter than the level is white, the rest black.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let on = luma(c.rgb) > params.level / 100.0;
    return vec4<f32>(vec3<f32>(select(0.0, 1.0, on)), c.a);
}
