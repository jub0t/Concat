// A hand sweeps clockwise from twelve. The angle is taken in pixels, so the
// hand turns at an even pace on a wide frame, and both of its edges - the
// sweeping one and the one it set out from - are a pixel soft.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    if (p >= 1.0) {
        return to_at(uv);
    }
    let c = (uv - vec2<f32>(0.5)) * frame.size;
    var a = atan2(c.x, -c.y);
    if (a < 0.0) {
        a = a + 6.28318530718;
    }
    // Arc lengths in pixels from the pixel to each edge.
    let r = length(c);
    let swept = clamp((p * 6.28318530718 - a) * r + 0.5, 0.0, 1.0);
    let started = clamp(a * r + 0.5, 0.0, 1.0);
    return mix(from_at(uv), to_at(uv), swept * started);
}
