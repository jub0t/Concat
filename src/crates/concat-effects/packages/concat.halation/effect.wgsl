// Halation: the red-orange halo film lays around its blown highlights. Light
// passes through the emulsion, scatters off the backing and comes back, so
// the halo belongs to the picture's brightest parts and is drawn a little
// wider than they are - never a glow over the whole frame.
//
// Three passes, each at a quarter of the layer's pixels both ways:
//
//  - `light` takes the picture's own light where it is bright, premultiplied
//    by alpha, as the average of the 4x4 block of the layer's pixels it
//    covers: four bilinear fetches, each the mean of a 2x2;
//  - `across` and `down` blur that light with one Gaussian, the same width
//    both ways, so a highlight's halo is round.
//
// The last pass reads the bloom's luminance and lays the tint on with it,
// graded in the display encoding, where the halo of a film burn is judged
// and where 1.0 is white.
//
// What a frame costs is held whatever the knobs and the layer's size: the
// threshold and the display encoding are worked out once per layer pixel in
// `light`, not once per tap, and a blur takes at most `MOST_PAIRS` fetches a
// pixel. At 4K with the radius at its widest the Gaussian wants about 245;
// past `MOST_PAIRS` (an 8K layer) its tails are cut short, still normalised.
// Halation 2 measured its blur in the layer's own pixels and scaled it with
// each pixel's distance from the centre: up to 1,800 fetches a pixel at 4K,
// each doing the threshold again - long enough to cost a small card its
// device - with no halo at all in the middle of the frame and a down pass
// one or two pixels tall.
//
// A flat picture survives a normalised convolution unchanged, so on flat
// input `down_at` is the light `light` worked out - which is what
// `fixtures.toml` pins.

// A WGSL package declares two things and nothing else: the `Params` struct
// whose fields are its knobs, in the manifest's order, and `fn effect`. The
// host reads the struct back through naga for each field's offset, so the
// order here is the layout there.
struct Params { threshold: f32, radius: f32, amount: f32, tint: f32 }

/// How far the taps reach, in sigmas: all but 0.27 % of the Gaussian.
const REACH: f32 = 3.0;

/// The most fetches a blur pass makes for one pixel, two of the light's
/// pixels to a fetch: what keeps a frame's cost bounded on any layer.
const MOST_PAIRS: i32 = 256;

fn weight(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

/// The picture's own light where it is bright, premultiplied by alpha so a
/// clear pixel adds none, with colour kept: the halo takes its hue later.
fn bright(c: vec4<f32>) -> vec4<f32> {
    let d = to_display(max(c.rgb, vec3<f32>(0.0)));
    let level = luma(d) * max(c.a, 0.0);
    let th = params.threshold / 100.0;
    let passed = smoothstep(th, min(th + 0.25, 1.0), level);
    return vec4<f32>(c.rgb * passed * c.a, c.a);
}

/// The bright light of the 4x4 block of layer pixels under `uv`: four
/// bilinear fetches a layer pixel from its centre, each the mean of a 2x2.
fn light(uv: vec2<f32>) -> vec4<f32> {
    let px = texel();
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let offset = vec2<f32>(f32(2 * x - 1), f32(2 * y - 1)) * px;
            sum += bright(sample(uv + offset));
        }
    }
    return sum * 0.25;
}

/// The Gaussian's sigma in the light's pixels: `params.radius` as a share
/// of the layer's height, so a halo reads the same on a phone and a 4K cut.
/// A quarter of the radius, so the default 22 % is a sigma of 5.5 % of the
/// height - about 59 pixels on a 1080p layer.
fn sigma_of(rows: f32) -> f32 {
    return max(params.radius / 100.0 * rows * 0.25, 0.5);
}

/// How many fetches cover the Gaussian of sigma `s` to `REACH` sigmas.
fn pairs_of(s: f32) -> i32 {
    return min(i32(ceil(REACH * s)) + 1, MOST_PAIRS);
}

/// The light weighed along its row about `uv`, two pixels to a fetch.
fn across(uv: vec2<f32>) -> vec4<f32> {
    let size = 1.0 / light_texel();
    let s = sigma_of(size.y);
    let pairs = pairs_of(s);
    let centre = uv.x * size.x;
    let first = floor(centre - f32(pairs) + 0.5) + 0.5;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        let w = wa + wb;
        if (w > 0.0) {
            sum += light_at(vec2<f32>((a + wb / w) / size.x, uv.y)) * w;
            total += w;
        }
    }
    if (total <= 0.0) {
        return light_at(uv);
    }
    return sum / total;
}

/// `across` weighed down its column about `uv`, the same Gaussian.
fn down(uv: vec2<f32>) -> vec4<f32> {
    let size = 1.0 / across_texel();
    let s = sigma_of(size.y);
    let pairs = pairs_of(s);
    let centre = uv.y * size.y;
    let first = floor(centre - f32(pairs) + 0.5) + 0.5;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        let w = wa + wb;
        if (w > 0.0) {
            sum += across_at(vec2<f32>(uv.x, (a + wb / w) / size.y)) * w;
            total += w;
        }
    }
    if (total <= 0.0) {
        return across_at(uv);
    }
    return sum / total;
}

/// The halo's colour at a tint: negative the deep red of a dense negative,
/// positive the orange of a print, zero the red-orange of the stock.
fn halation_tint(t: f32) -> vec3<f32> {
    let s = t / 100.0;
    return vec3<f32>(1.0, 0.42 + 0.30 * s, 0.16 + 0.12 * s);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let base = to_display(c.rgb);
    let bloom = luma(down_at(uv).rgb);
    let lit = base + halation_tint(params.tint) * bloom * (params.amount / 100.0) * 1.6;
    return vec4<f32>(from_display(lit), c.a);
}
