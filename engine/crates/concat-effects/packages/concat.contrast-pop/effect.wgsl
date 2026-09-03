struct Params { contrast: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let out = (c.rgb - vec3<f32>(0.5)) * params.contrast + vec3<f32>(0.5);
    return vec4<f32>(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
