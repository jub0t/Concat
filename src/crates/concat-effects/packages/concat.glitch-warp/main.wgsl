struct Params { amount: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let k = sin(progress * 3.14159);
    let slice = floor(uv.y * 20.0);
    let noise = hash(vec2<f32>(slice, floor(progress * 15.0)), 1.0);
    let disp = (noise - 0.5) * k * (params.amount * 0.001);
    let uv_r = uv + vec2<f32>(disp * 1.5, 0.0);
    let uv_g = uv + vec2<f32>(disp, 0.0);
    let uv_b = uv - vec2<f32>(disp, 0.0);
    let r = mix(from_at(uv_r).r, to_at(uv_r).r, progress);
    let g = mix(from_at(uv_g).g, to_at(uv_g).g, progress);
    let b = mix(from_at(uv_b).b, to_at(uv_b).b, progress);
    let a = mix(from_at(uv).a, to_at(uv).a, progress);
    return vec4<f32>(r, g, b, a);
}
