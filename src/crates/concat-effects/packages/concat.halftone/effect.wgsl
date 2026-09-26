struct Params { size: f32 }

// A grid of black dots on white, each as large as its cell is dark on the
// display level - the picture as a newspaper prints it - with edges a
// pixel soft so the dots stay round at any size. The paper is as bright as
// the picture where the picture is brighter than white, so a highlight
// stays one.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let cell = max(params.size, 2.0);
    let at = uv * frame.size;
    let middle = (floor(at / cell) + vec2<f32>(0.5)) * cell;
    let seen = luma(sample(middle / frame.size).rgb);
    let level = clamp(seen, 0.0, 1.0);
    // A dot's area goes as the darkness, so its radius as the square root.
    let radius = sqrt(1.0 - level) * cell * 0.7071;
    let ink = 1.0 - smoothstep(radius - 0.75, radius + 0.75, distance(at, middle));
    return vec4<f32>(vec3<f32>((1.0 - ink) * max(seen, 1.0)), c.a);
}
