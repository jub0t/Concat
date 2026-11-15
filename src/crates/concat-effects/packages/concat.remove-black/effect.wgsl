struct Params { threshold: f32, strength: f32 }

// Black taken out by unmultiplying: a picture shot on black is light over
// nothing, so its brightest channel is how much of it there is, and the
// picture is that much of its colour drawn at full brightness. Worked in
// light, so a glow keeps its colour and covers what is beneath by as much
// light as it has - the sum light makes with light, near enough, not the
// darkening a key cut on the display level would give - and a dim edge
// fades out rather than turning into a dark rim. The black level is taken
// off on the display level, where a lifted black or a stream's noise is a
// few steps of the code, and what is left is stretched back over the
// range.

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let most = max(max(c.r, c.g), c.b);
    if (most <= 0.0) {
        return vec4<f32>(0.0);
    }
    let black = clamp(params.threshold / 100.0, 0.0, 0.99);
    let level = pow(most, 1.0 / 2.4);
    let lifted = max(level - black, 0.0) / (1.0 - black);
    let light = pow(lifted, 2.4);
    let alpha = clamp(light * params.strength / 100.0, 0.0, 1.0);
    // The colour with its black taken off; a highlight past white is
    // already whole and is left as it is.
    let kept = c.rgb * min(light / most, 1.0);
    let front = kept / max(alpha, 1e-4);
    return vec4<f32>(min(front, vec3<f32>(max(max(max(kept.r, kept.g), kept.b), 1.0))), c.a * alpha);
}
