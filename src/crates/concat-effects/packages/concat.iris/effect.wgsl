struct Params { softness: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    // A circle in pixels, measured in half-diagonals so it opens all the
    // way to the corners of any frame.
    let d = length((uv - vec2<f32>(0.5)) * frame.size) / (0.5 * length(frame.size));
    let soft = max(params.softness * 0.01, 0.02);
    // The edge is led, not centred: it travels from -soft to
    // 1.15 + soft, so the whole of its band is off the picture at both
    // ends - at progress 0 a centred band left a half-blended blob on the
    // centre, at progress 1 the corners up to 39 % outgoing at full
    // softness. The same lead the CPU fallback's edge() takes (see
    // concat-render's transitions.rs), over the same 1.15 overshoot.
    let radius = p * (1.15 + 2.0 * soft) - soft;
    let m = smoothstep(radius - soft, radius + soft, d);
    return mix(to_at(uv), from_at(uv), m);
}
