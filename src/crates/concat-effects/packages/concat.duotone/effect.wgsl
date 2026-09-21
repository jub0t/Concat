struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = luma(c.rgb);
    let shadow = vec3<f32>(0.05, 0.05, 0.25);
    let highlight = vec3<f32>(1.0, 0.75, 0.25);
    let toned = mix(shadow, highlight, g);
    return vec4<f32>(clamp01(mix(c.rgb, toned, params.amount * 0.01)), c.a);
}
