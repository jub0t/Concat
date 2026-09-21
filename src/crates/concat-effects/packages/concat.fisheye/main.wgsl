struct Params { strength: f32 }

// Barrel distortion about the centre, stronger towards the edge.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let k = params.strength / 100.0 * 0.6;
    let aspect = frame.size.x / frame.size.y;
    var p = (uv - vec2<f32>(0.5)) * vec2<f32>(2.0 * aspect, 2.0);
    let r2 = dot(p, p);
    p = p * (1.0 - k * r2) / (1.0 - k * (aspect * aspect + 1.0) * 0.5);
    let q = p / vec2<f32>(2.0 * aspect, 2.0) + vec2<f32>(0.5);
    if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return sample(q);
}
