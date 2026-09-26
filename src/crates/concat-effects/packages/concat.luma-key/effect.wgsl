struct Params { threshold: f32, softness: f32, invert: f32 }

// A key on the display level's luminance: a pixel darker than `threshold`
// is taken out, fading back in over `softness` below it, so at 0 the whole
// picture stays; inverted, a pixel lighter than it, fading over `softness`
// above it.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let threshold = params.threshold / 100.0;
    let softness = max(params.softness / 100.0, 0.001);
    let level = luma(c.rgb);
    var keep = smoothstep(threshold - softness, threshold, level);
    if (params.invert > 0.5) {
        keep = 1.0 - smoothstep(threshold, threshold + softness, level);
    }
    return vec4<f32>(c.rgb, c.a * keep);
}
