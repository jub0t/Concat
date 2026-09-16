struct Params { similarity: f32, softness: f32 }

// Keys by chroma distance in YCbCr, the way the FFmpeg chain does, so the
// same similarity and softness mean the same edge on either path.
fn chroma(rgb: vec3<f32>) -> vec2<f32> {
    let cb = -0.1146 * rgb.r - 0.3854 * rgb.g + 0.5 * rgb.b;
    let cr = 0.5 * rgb.r - 0.4542 * rgb.g - 0.0458 * rgb.b;
    return vec2<f32>(cb, cr);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let key = vec3<f32>(0.0, 1.0, 0.0);
    let d = distance(chroma(c.rgb), chroma(key)) / 0.7071;
    let similarity = params.similarity / 100.0;
    let softness = params.softness / 100.0;
    let keep = smoothstep(similarity, similarity + max(softness, 0.001), d);
    return vec4<f32>(c.rgb, c.a * keep);
}
