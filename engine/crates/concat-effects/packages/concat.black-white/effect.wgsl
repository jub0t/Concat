// Every channel takes the luminance.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(vec3<f32>(luma(c.rgb)), c.a);
}
