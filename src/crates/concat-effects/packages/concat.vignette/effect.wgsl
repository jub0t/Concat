struct Params { strength: f32 }

// The light falling off towards the corners, as a lens's does: a multiply
// in light, shaped as the vignette always was on SDR (its fall taken to the
// 2.4th power, the display encoding's), so a highlight in a corner darkens
// by the same share as the rest.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let d = length((uv - vec2<f32>(0.5)) * vec2<f32>(2.0));
    let s = params.strength / 100.0;
    let fall = smoothstep(1.4 - s * 0.9, 1.4 + 0.2 - s * 0.3, d);
    return vec4<f32>(c.rgb * pow(1.0 - fall * (0.4 + 0.6 * s), 2.4), c.a);
}
