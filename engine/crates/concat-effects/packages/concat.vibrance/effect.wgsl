struct Params { amount: f32 }

// Saturates the dull colours more than the vivid ones.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let l = luma(c.rgb);
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    let sat = mx - mn;
    let boost = params.amount * (1.0 - sat);
    return vec4<f32>(clamp(mix(vec3<f32>(l), c.rgb, 1.0 + boost), vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
