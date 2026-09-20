struct Params { direction: f32 }

fn zoom_uv(uv: vec2<f32>, amount: f32) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) / max(amount, 0.0001) + vec2<f32>(0.5);
}

// The outgoing picture whips away while the incoming one whips into place;
// "Out" reverses which side does the whipping.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    var out_scale: f32;
    var in_scale: f32;
    if (params.direction < 0.5) {
        out_scale = 1.0 + p * 1.6;
        in_scale = 1.6 - p * 1.6;
    } else {
        out_scale = 1.0 - p * 0.6;
        in_scale = 0.4 + p * 0.6;
    }
    let a = from_at(zoom_uv(uv, out_scale));
    let b = to_at(zoom_uv(uv, in_scale));
    return mix(a, b, p);
}
