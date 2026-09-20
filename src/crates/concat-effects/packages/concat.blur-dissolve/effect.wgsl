struct Params { strength: f32 }

fn soft_from(uv: vec2<f32>, radius: f32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    let t = texel() * radius;
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            sum += from_at(uv + vec2<f32>(f32(x), f32(y)) * t);
        }
    }
    return sum / 9.0;
}

fn soft_to(uv: vec2<f32>, radius: f32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    let t = texel() * radius;
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            sum += to_at(uv + vec2<f32>(f32(x), f32(y)) * t);
        }
    }
    return sum / 9.0;
}

// Blur peaks at the middle of the cut and resolves to sharp at both ends.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let k = 1.0 - abs(progress * 2.0 - 1.0);
    let radius = k * params.strength * 0.3;
    return mix(soft_from(uv, radius), soft_to(uv, radius), progress);
}
