struct Params { heat: f32 }

// Light added to `rgb` in the display encoding, up to white and no
// further: a flash burns a picture white, and a highlight already brighter
// than that is left as bright as it was.
fn burn_to_white(rgb: vec3<f32>, flash: vec3<f32>) -> vec3<f32> {
    let lit = to_display(rgb);
    return from_display(min(lit + flash, max(lit, vec3<f32>(1.0))));
}

// A warm flash triangular in time, peaking at the middle of the cut: the
// pictures dissolve in light, and the flash is laid on in the display
// encoding it was drawn in, burning to white.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let base = mix(from_at(uv), to_at(uv), progress);
    let burn = smoothstep(0.0, 0.5, progress) * (1.0 - smoothstep(0.5, 1.0, progress)) * 4.0;
    let warm = vec3<f32>(1.0, 0.85, 0.6) * burn * (params.heat * 0.01);
    return vec4<f32>(burn_to_white(base.rgb, warm), base.a);
}
