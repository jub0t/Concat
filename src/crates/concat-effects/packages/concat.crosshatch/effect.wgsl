struct Params { amount: f32 }

// Three tiers of anti-aliased lines, each fading in over its own luma band
// instead of a hard boolean cutoff - so both the strokes and where they
// start are smooth, the way inked hatching reads rather than a jagged mask.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let l = luma(c.rgb);
    let p = uv * frame.size / 4.0;
    let d1 = abs(fract(p.x + p.y) - 0.5);
    let d2 = abs(fract(p.x - p.y) - 0.5);
    let d3 = abs(fract(p.x * 2.0 + p.y * 2.0) - 0.5);
    let line1 = 1.0 - smoothstep(0.05, 0.09, d1);
    let line2 = 1.0 - smoothstep(0.05, 0.09, d2);
    let line3 = 1.0 - smoothstep(0.05, 0.09, d3);
    let band1 = smoothstep(0.75, 0.65, l);
    let band2 = smoothstep(0.45, 0.35, l);
    let band3 = smoothstep(0.2, 0.1, l);
    var hatch = 1.0;
    hatch -= line1 * band1 * 0.65;
    hatch -= line2 * band2 * 0.4;
    hatch -= line3 * band3 * 0.35;
    hatch = clamp(hatch, 0.0, 1.0);
    let k = params.amount * 0.01;
    return vec4<f32>(clamp01(mix(c.rgb, vec3<f32>(hatch), k)), c.a);
}
