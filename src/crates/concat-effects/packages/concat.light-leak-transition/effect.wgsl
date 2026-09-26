struct Params { warmth: f32 }

// Light added to `rgb` in the display encoding, up to white and no
// further: a flash burns a picture white, and a highlight already brighter
// than that is left as bright as it was.
fn burn_to_white(rgb: vec3<f32>, flash: vec3<f32>) -> vec3<f32> {
    let lit = to_display(rgb);
    return from_display(min(lit + flash, max(lit, vec3<f32>(1.0))));
}

// A bright near-white core inside a wider warm halo, rather than one flat
// tint - the hot centre a real light leak burns and the softer colour
// bleed around it. The pictures dissolve in light; the leak is laid on in
// the display encoding it was drawn in, burning to white.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let base = mix(from_at(uv), to_at(uv), p);
    let sweep = uv.x + uv.y * 0.3 - (p * 2.2 - 0.6);
    let core = exp(-sweep * sweep * 22.0) * 1.4;
    let halo = exp(-sweep * sweep * 6.0) * 0.6;
    let k = params.warmth * 0.01;
    let hot = vec3<f32>(1.0, 0.95, 0.85);
    let warm = vec3<f32>(1.0, 0.8, 0.5);
    let leak = hot * core * k * 1.3 + warm * halo * k;
    return vec4<f32>(burn_to_white(base.rgb, leak), base.a);
}
