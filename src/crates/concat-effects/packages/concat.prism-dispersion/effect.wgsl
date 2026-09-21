struct Params { strength: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dir = uv - center;
    let disp = dir * (params.strength * 0.0006);
    let r = sample(uv + disp * 1.5).r;
    let g = sample(uv).g;
    let b = sample(uv - disp * 1.5).b;
    let a = sample(uv).a;
    return vec4<f32>(r, g, b, a);
}
