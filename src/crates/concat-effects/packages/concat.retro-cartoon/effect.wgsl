struct Params { levels: f32, ink: f32 }

// Posterized, then darkened where the picture has an edge.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let levels = max(round(params.levels), 2.0);
    // White is the top band; light past it is left past it, not stretched.
    let bands = min(floor(c.rgb * levels), vec3<f32>(levels - 1.0)) / (levels - 1.0);
    let flat = select(bands, c.rgb, c.rgb > vec3<f32>(1.0));
    let ink = edge_at(uv) * params.ink / 100.0;
    return vec4<f32>(flat * (1.0 - ink), c.a);
}
