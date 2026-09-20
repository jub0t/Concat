struct Params { shift: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel() * params.shift;
    let r = sample(uv + vec2<f32>(t.x, 0.0)).r;
    let g = sample(uv).g;
    let b = sample(uv - vec2<f32>(t.x, 0.0)).b;
    return vec4<f32>(r, g, b, sample(uv).a);
}
