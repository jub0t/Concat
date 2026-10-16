// A dip to white: the outgoing picture fades up to white by the middle of
// the cut, and the incoming one fades down out of it, in the display
// encoding as the dip to black is. No `xfade` names it: FFmpeg's
// `fadewhite` is drawn as the film-burn flash, so a GPU-less path keeps
// the plain dissolve instead.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let base = select(from_at(uv), to_at(uv), progress >= 0.5);
    let light = 1.0 - abs(progress * 2.0 - 1.0);
    let rgb = from_display(mix(to_display(base.rgb), vec3<f32>(1.0), light));
    return vec4<f32>(rgb, base.a);
}
