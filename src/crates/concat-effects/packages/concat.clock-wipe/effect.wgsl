fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let c = uv - vec2<f32>(0.5);
    var a = atan2(c.x, -c.y);
    if (a < 0.0) {
        a = a + 6.28318530718;
    }
    if (a < p * 6.28318530718) {
        return to_at(uv);
    }
    return from_at(uv);
}
