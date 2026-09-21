struct Params { strength: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dir = uv - center;
    let k = sin(progress * 3.14159);
    let zoom = 1.0 + k * 0.5;
    let blur = k * (params.strength * 0.0006);
    var sum = vec4<f32>(0.0);
    for (var i: i32 = 0; i < 5; i++) {
        let scale = 1.0 + f32(i) * blur;
        let sample_uv = center + (dir / zoom) * scale;
        sum += mix(from_at(sample_uv), to_at(sample_uv), progress);
    }
    return sum / 5.0;
}
