struct Params { amount: f32, speed: f32, background: f32 }

// Lengths are in units of the layer's short side, so the marks keep their
// shape at 16:9, 9:16 and 1:1 alike. A glyph's own space is one unit tall,
// centred on its middle, y up.

const TAU: f32 = 6.2831853;

fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Distance to the arc of the circle at `c`, radius `r`, from angle `a0`
// anticlockwise to `a1`.
fn arc(p: vec2<f32>, c: vec2<f32>, r: f32, a0: f32, a1: f32) -> f32 {
    let q = p - c;
    let a = atan2(q.y, q.x);
    let along = a - a0 - TAU * floor((a - a0) / TAU);
    if (along <= a1 - a0) {
        return abs(length(q) - r);
    }
    let e0 = c + r * vec2<f32>(cos(a0), sin(a0));
    let e1 = c + r * vec2<f32>(cos(a1), sin(a1));
    return min(length(p - e0), length(p - e1));
}

// Distance to the centre line of a question mark: the hook, the stem
// under it and the dot.
fn question(p: vec2<f32>) -> f32 {
    let c = vec2<f32>(0.0, 0.17);
    let r = 0.2;
    let a0 = -0.9;
    let hook = arc(p, c, r, a0, 3.35);
    let bend = c + r * vec2<f32>(cos(a0), sin(a0));
    let neck = vec2<f32>(0.0, -0.07);
    let stem = min(segment(p, bend, neck), segment(p, neck, vec2<f32>(0.0, -0.17)));
    let dot_d = max(length(p - vec2<f32>(0.0, -0.37)) - 0.02, 0.0);
    return min(min(hook, stem), dot_d);
}

// One depth of falling marks: a grid of cells `cell` wide, at most one mark
// a cell, kept where the cell's hash passes `keep`. Each mark's place,
// size, tilt and flicker are hashed from its cell. Returns how much of the
// mark covers the point and how much of its glow reaches it.
fn marks(q: vec2<f32>, cell: f32, seed: f32, t: f32, fall: f32, keep: f32, soft: f32) -> vec2<f32> {
    let drop = t * fall;
    let g = (q - vec2<f32>(0.0, drop)) / cell;
    let base = floor(g);
    var cover = 0.0;
    var glow = 0.0;
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            let id = base + vec2<f32>(f32(x), f32(y));
            let h1 = hash(id, seed + 1.0);
            let h2 = hash(id, seed + 2.0);
            let h3 = hash(id, seed + 3.0);
            let h4 = hash(id, seed + 4.0);
            if (h4 < keep) {
                continue;
            }
            let sway = sin(t * (0.4 + h2) + h1 * TAU) * 0.06;
            let centre = id + vec2<f32>(0.5) + (vec2<f32>(h1, h2) - vec2<f32>(0.5)) * 0.4 + vec2<f32>(sway, 0.0);
            let tall = 0.45 + 0.35 * h3;
            let tilt = (h1 - 0.5) * 0.7 + sin(t * 0.7 + h3 * TAU) * 0.08;
            let d = (g - centre) / tall;
            let co = cos(tilt);
            let si = sin(tilt);
            let glyph = vec2<f32>(co * d.x - si * d.y, -(si * d.x + co * d.y));
            let dist = question(glyph);
            let ink = 1.0 - smoothstep(0.065, 0.065 + soft, dist);
            let flicker = 0.75 + 0.25 * sin(t * (2.0 + 3.0 * h2) + h4 * TAU);
            cover = max(cover, ink * flicker);
            glow = max(glow, exp(-max(dist - 0.05, 0.0) * 9.0) * flicker);
        }
    }
    return vec2<f32>(cover, glow);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    let side = max(min(frame.size.x, frame.size.y), 1.0);
    let q = uv * frame.size / side;
    let t = frame.time * (0.15 + params.speed / 100.0 * 1.6);
    let px = 1.0 / side;

    let far = marks(q, 0.08, 10.0, t, 0.05, 0.55, max(px / (0.08 * 0.5), 0.14));
    let mid = marks(q, 0.17, 20.0, t, 0.09, 0.6, max(px / (0.17 * 0.6), 0.05));
    let near = marks(q, 0.45, 30.0, t, 0.16, 0.7, max(px / (0.45 * 0.6), 0.02));

    let light = vec3<f32>(0.5, 0.55, 0.62) * far.x * 0.5
        + vec3<f32>(0.8, 0.85, 0.92) * mid.x * 0.85
        + vec3<f32>(1.0) * near.x
        + vec3<f32>(0.6, 0.7, 0.85) * (far.y * 0.06 + mid.y * 0.18 + near.y * 0.45);

    let breathe = 0.9 + 0.1 * sin(t * 1.3);
    let dim = clamp((0.35 + 0.6 * params.background / 100.0) * breathe, 0.0, 1.0);
    let ground = mix(c.rgb, mix(c.rgb * 0.3, vec3<f32>(0.18), 0.3), dim);
    let base = mix(c.rgb, ground, amount);
    return vec4<f32>(screen(base, min(light * amount, vec3<f32>(1.0))), c.a);
}
