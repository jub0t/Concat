struct Params { amount: f32, speed: f32 }

// The highlights' light - each pixel weighed by how far its display level
// is past 0.55, the knee of the bloom - spread by a Gaussian of twelve of
// the layer's pixels, across and then down at a quarter of them, and
// screened back over the picture by the amount as it breathes. The weight
// is taken to the 2.4th power in light, so a flat field blooms by the same
// share of its display level it always did; the screen is taken in the
// display encoding, a level past white carried on past it.

const SIGMA: f32 = 12.0;

/// How far the taps reach, in sigmas.
const REACH: f32 = 3.0;

fn weight(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

/// The light of the layer's highlights at `uv`, premultiplied.
fn bright_at(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let bright = pow(smoothstep(0.55, 1.0, luma(to_display(c.rgb))), 2.4);
    return vec4<f32>(c.rgb * c.a * bright, c.a);
}

/// The highlights weighed along their row about `uv`, two pixels to a fetch.
fn across(uv: vec2<f32>) -> vec4<f32> {
    let width = frame.size.x;
    let centre = uv.x * width;
    let first = floor(centre - REACH * SIGMA - 0.5) + 0.5;
    let pairs = i32(ceil(REACH * SIGMA)) + 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, SIGMA);
        let wb = weight(a + 1.0 - centre, SIGMA);
        let w = wa + wb;
        sum += bright_at(vec2<f32>((a + wb / w) / width, uv.y)) * w;
        total += w;
    }
    return sum / total;
}

/// `across` weighed down its column about `uv`, in its own pixels.
fn down(uv: vec2<f32>) -> vec4<f32> {
    let rows = 1.0 / across_texel().y;
    let s = SIGMA * rows / frame.size.y;
    let centre = uv.y * rows;
    let first = floor(centre - REACH * s - 0.5) + 0.5;
    let pairs = i32(ceil(REACH * s)) + 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        let w = wa + wb;
        sum += across_at(vec2<f32>(uv.x, (a + wb / w) / rows)) * w;
        total += w;
    }
    return sum / total;
}

/// `over` screened onto `base`, both display-encoded: light added the way
/// two projectors add it, a level of `base` past white kept past it.
fn screen(base: vec3<f32>, over: vec3<f32>) -> vec3<f32> {
    let b = min(base, vec3<f32>(1.0));
    return vec3<f32>(1.0) - (vec3<f32>(1.0) - b) * (vec3<f32>(1.0) - over) + (base - b);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pulse = 0.6 + 0.4 * sin(frame.time * params.speed);
    let strength = params.amount * 0.01 * pulse;
    let bloom = clamp(to_display(down_at(uv).rgb) * strength, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(from_display(screen(to_display(c.rgb), bloom)), c.a);
}
