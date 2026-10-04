struct Params { amount: f32, speed: f32, spread: f32, radius: f32 }

// Lengths below are in units of the layer's short side, so the band of
// embers and the corners keep their shape at 16:9, 9:16 and 1:1 alike.

// How near the frame's edge a point is: 1 on the edge, 0 from `reach` in.
fn edge_weight(q: vec2<f32>, dims: vec2<f32>, reach: f32) -> f32 {
    let d = min(min(q.x, dims.x - q.x), min(q.y, dims.y - q.y));
    return 1.0 - smoothstep(reach * 0.2, reach, d);
}

// One layer of embers: a grid of cells `cell` wide, one soft disc a cell,
// the whole field rising slowly. Each disc's place, size, sway and twinkle
// are hashed from its cell, and it is kept only where the edge is near, so
// it fades in and out as it drifts through the band. Returns the coverage
// and how hot (bright at the core) the ember there is.
fn embers(q: vec2<f32>, dims: vec2<f32>, cell: f32, seed: f32, t: f32, reach: f32) -> vec2<f32> {
    let rise = t * (0.035 + 0.02 * seed);
    let g = (q + vec2<f32>(0.0, rise)) / cell;
    let base = floor(g);
    var cover = 0.0;
    var heat = 0.0;
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            let id = base + vec2<f32>(f32(x), f32(y));
            let h1 = hash(id, seed + 1.0);
            let h2 = hash(id, seed + 2.0);
            let h3 = hash(id, seed + 3.0);
            let h4 = hash(id, seed + 4.0);
            let sway = vec2<f32>(sin(t * (0.6 + h2) + h1 * 6.2832), cos(t * (0.5 + h1) + h3 * 6.2832)) * 0.08;
            let centre = id + vec2<f32>(0.5) + (vec2<f32>(h1, h2) - vec2<f32>(0.5)) * 0.6 + sway;
            let pos = centre * cell - vec2<f32>(0.0, rise);
            let alive = clamp((edge_weight(pos, dims, reach) - h4) * 4.0, 0.0, 1.0);
            let r = 0.08 + 0.34 * h3 * h3;
            let d = length(g - centre);
            let disc = 1.0 - smoothstep(r * 0.45, r, d);
            let core = 1.0 - smoothstep(0.0, r * 0.5, d);
            let twinkle = 0.5 + 0.5 * sin(6.2832 * t * (0.25 + 0.6 * h2) + h4 * 6.2832);
            let c = disc * alive * (0.35 + 0.65 * twinkle);
            if (c > cover) {
                cover = c;
                heat = core * twinkle * (1.0 - h3);
            }
        }
    }
    return vec2<f32>(cover, heat);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let side = max(min(frame.size.x, frame.size.y), 1.0);
    let dims = frame.size / side;
    let q = uv * dims;
    let t = frame.time * (0.2 + params.speed / 100.0 * 2.4);
    let reach = 0.08 + params.spread / 100.0 * 0.4;

    let fine = embers(q, dims, 0.035, 0.0, t, reach);
    let mid = embers(q, dims, 0.07, 1.0, t, reach * 0.9);
    let wide = embers(q, dims, 0.13, 2.0, t, reach * 0.8);
    var e = fine;
    if (mid.x > e.x) { e = mid; }
    if (wide.x > e.x) { e = wide; }

    let ember = mix(vec3<f32>(0.85, 0.2, 0.04), vec3<f32>(1.0, 0.62, 0.28), clamp(e.y, 0.0, 1.0));
    let ea = clamp(e.x * params.amount / 100.0, 0.0, 1.0);

    let r = params.radius / 100.0 * 0.5;
    let p = abs(q - dims * 0.5) - (dims * 0.5 - vec2<f32>(r));
    let dist = length(max(p, vec2<f32>(0.0))) + min(max(p.x, p.y), 0.0) - r;
    let corner = clamp(0.5 - dist * side, 0.0, 1.0);

    let base_a = c.a * corner;
    let a = ea + base_a * (1.0 - ea);
    let rgb = (ember * ea + c.rgb * base_a * (1.0 - ea)) / max(a, 1e-5);
    return vec4<f32>(rgb, a);
}
