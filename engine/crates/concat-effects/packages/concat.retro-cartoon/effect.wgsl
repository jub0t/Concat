struct Params { levels: f32, ink: f32 }

// Posterized, then darkened where the picture has an edge.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let levels = max(round(params.levels), 2.0);
    let flat = floor(c.rgb * levels) / (levels - 1.0);
    let ink = edge_at(uv) * params.ink / 100.0;
    return vec4<f32>(clamp01(flat * (1.0 - ink)), c.a);
}
