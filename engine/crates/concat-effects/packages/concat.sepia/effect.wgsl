// The classic sepia matrix, the same numbers the FFmpeg chain uses.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let r = dot(c.rgb, vec3<f32>(0.393, 0.769, 0.189));
    let g = dot(c.rgb, vec3<f32>(0.349, 0.686, 0.168));
    let b = dot(c.rgb, vec3<f32>(0.272, 0.534, 0.131));
    return vec4<f32>(clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
