struct Params { haze: f32, blush: f32 }

// A fade, less contrast, the blush in the brights, and a bloom screened over.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let haze = params.haze / 100.0;
    var out = fade(c.rgb, haze * 0.1);
    out = contrast(out, 1.0 - haze * 0.2);
    out = saturation(out, 1.0 - haze * 0.15);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.1, -0.04, 0.08), params.blush / 100.0);
    let bloom = soften(uv, 10.0);
    let screen = vec3<f32>(1.0) - (vec3<f32>(1.0) - out) * (vec3<f32>(1.0) - bloom);
    out = mix(out, screen, haze * 0.35);
    return vec4<f32>(clamp01(out), c.a);
}
