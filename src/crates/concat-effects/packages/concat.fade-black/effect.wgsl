// A dip to black: the outgoing picture fades down to black by the middle of
// the cut, and the incoming one fades up out of it. The fade runs in the
// display encoding, so it darkens evenly to the eye and matches the CPU's
// `fadeblack`, which mixes the encoded pictures.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let base = select(from_at(uv), to_at(uv), progress >= 0.5);
    let dark = 1.0 - abs(progress * 2.0 - 1.0);
    let rgb = from_display(to_display(base.rgb) * (1.0 - dark));
    return vec4<f32>(rgb, base.a);
}
