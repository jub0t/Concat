struct Params { direction: f32, softness: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    var coord: f32;
    if (params.direction < 0.5) {
        coord = uv.x;
    } else if (params.direction < 1.5) {
        coord = 1.0 - uv.x;
    } else if (params.direction < 2.5) {
        coord = 1.0 - uv.y;
    } else {
        coord = uv.y;
    }
    let soft = max(params.softness * 0.003, 0.001);
    let m = smoothstep(p - soft, p + soft, coord);
    return mix(to_at(uv), from_at(uv), m);
}
