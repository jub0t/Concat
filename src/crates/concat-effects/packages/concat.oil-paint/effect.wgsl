struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let s = soften(uv, 2.0 + params.amount * 0.04);
    let posterized = floor(s * 6.0) / 6.0;
    return vec4<f32>(clamp01(mix(c.rgb, posterized, params.amount * 0.01)), c.a);
}
