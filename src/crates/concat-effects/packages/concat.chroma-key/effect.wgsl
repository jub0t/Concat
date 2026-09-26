struct Params { color: vec4<f32>, similarity: f32, softness: f32, spill: f32 }

// A key on the display level, where the key colour was picked: a pixel
// whose chroma - BT.709's Cb and Cr - lies within `similarity` of the key
// colour's is taken out, with `softness` more to fade over; and what stays
// has the key's hue taken out of it by `spill`, as far as it leans towards
// it, so a green fringe on hair or a green cast on a face is cleaned off.

fn chroma(rgb: vec3<f32>) -> vec2<f32> {
    let cb = -0.1146 * rgb.r - 0.3854 * rgb.g + 0.5 * rgb.b;
    let cr = 0.5 * rgb.r - 0.4542 * rgb.g - 0.0458 * rgb.b;
    return vec2<f32>(cb, cr);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let here = chroma(c.rgb);
    let key = chroma(params.color.rgb);
    let similarity = params.similarity / 100.0;
    let softness = max(params.softness / 100.0, 0.001);
    let keep = smoothstep(similarity, similarity + softness, distance(here, key) / 0.7071);
    let reach = length(key);
    var cleaned = here;
    if (reach > 1e-4) {
        let towards = key / reach;
        cleaned = here - towards * max(dot(here, towards), 0.0) * (params.spill / 100.0);
    }
    let y = luma(c.rgb);
    let rgb = vec3<f32>(
        y + 1.5748 * cleaned.y,
        y - 0.1873 * cleaned.x - 0.4681 * cleaned.y,
        y + 1.8556 * cleaned.x,
    );
    return vec4<f32>(rgb, c.a * keep);
}
