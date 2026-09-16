struct Params { depth: f32 }

// A relief kernel over the picture's own colour: the pixel above and to the
// left is taken away, the one below and to the right added, weighted by the
// depth, and the kernel sums to one so the picture stays under the relief.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let d = params.depth;
    let c = sample(uv);
    var rgb = c.rgb;
    rgb = rgb - 2.0 * d * sample(uv + vec2<f32>(-t.x, -t.y)).rgb;
    rgb = rgb - d * sample(uv + vec2<f32>(0.0, -t.y)).rgb;
    rgb = rgb - d * sample(uv + vec2<f32>(-t.x, 0.0)).rgb;
    rgb = rgb + d * sample(uv + vec2<f32>(t.x, 0.0)).rgb;
    rgb = rgb + d * sample(uv + vec2<f32>(0.0, t.y)).rgb;
    rgb = rgb + 2.0 * d * sample(uv + vec2<f32>(t.x, t.y)).rgb;
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
