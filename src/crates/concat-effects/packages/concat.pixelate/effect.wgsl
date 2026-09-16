struct Params { size: f32 }

// Every pixel takes the colour at the centre of its `size`-pixel block.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let block = max(round(params.size), 1.0);
    let p = floor(uv * frame.size / block) * block + vec2<f32>(block * 0.5);
    return sample(p / frame.size);
}
