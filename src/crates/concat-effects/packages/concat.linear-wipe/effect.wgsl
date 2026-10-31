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
    // The edge is led, not centred: it travels from -soft to 1 + soft,
    // so the whole of its band is off the picture at both ends - a
    // centred band straddled coord 0 at progress 0 and coord 1 at
    // progress 1, leaving the first and last positions half-blended.
    // The same lead the CPU fallback's edge() takes (see concat-render's
    // transitions.rs).
    let at = p * (1.0 + 2.0 * soft) - soft;
    let m = smoothstep(at - soft, at + soft, coord);
    return mix(to_at(uv), from_at(uv), m);
}
