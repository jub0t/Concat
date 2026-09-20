struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = uv - vec2<f32>(0.5);
    let d = length(c);
    let a = params.amount * 0.01 * 6.0 * (1.0 - smoothstep(0.0, 0.7, d));
    let s = sin(a);
    let co = cos(a);
    let r = vec2<f32>(c.x * co - c.y * s, c.x * s + c.y * co);
    return sample(r + vec2<f32>(0.5));
}
