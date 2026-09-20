struct Params { amount: f32, radius: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let tint = vec3<f32>(1.0, 0.35, 0.1);
    let glow = halation(uv, c.rgb, 0.6, params.radius, tint, params.amount * 0.01 * 1.5);
    return vec4<f32>(glow, c.a);
}
