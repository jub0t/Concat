struct Params { speed: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let blur_amt = sin(progress * 3.14159) * (params.speed * 0.0008);
    var col = vec4<f32>(0.0);
    for (var i: i32 = -2; i <= 2; i++) {
        let offset = vec2<f32>(f32(i) * blur_amt, 0.0);
        let from_col = from_at(uv + vec2<f32>(p, 0.0) + offset);
        let to_col = to_at(uv - vec2<f32>(1.0 - p, 0.0) + offset);
        col += mix(from_col, to_col, step(1.0 - p, uv.x));
    }
    return col / 5.0;
}
