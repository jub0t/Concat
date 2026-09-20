struct Params { tiles: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = params.tiles;
    let scaled = uv * t;
    var p = fract(scaled);
    let ix = i32(floor(scaled.x));
    let iy = i32(floor(scaled.y));
    if ((ix & 1) == 1) { p.x = 1.0 - p.x; }
    if ((iy & 1) == 1) { p.y = 1.0 - p.y; }
    return sample(p);
}
