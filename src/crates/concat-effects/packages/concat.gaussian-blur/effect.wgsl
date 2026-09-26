struct Params { radius: f32 }

// A Gaussian of sigma `radius` of the layer's pixels, in three passes:
// `across` weighs the layer along its rows, `down` weighs that down its
// columns, and `effect` reads the result back over the layer. A 2-D
// Gaussian is the product of the two, so this is the whole of it; the
// manifest draws the first two at a power of two fewer pixels where the
// radius is wide, which the last pass's bilinear reading fills back in.
//
// Each pass weighs the pixel centres within three sigmas by the Gaussian of
// their distance, two at a time: one bilinear fetch placed between two
// centres by their weights is both of them. Light is blurred premultiplied
// by its alpha, so a transparent pixel's colour never bleeds into the
// picture beside it, and `effect` divides it back out.

/// How far the taps reach, in sigmas: all but 0.27 % of the Gaussian.
const REACH: f32 = 3.0;

fn sigma() -> f32 {
    return max(params.radius, 0.5);
}

/// The Gaussian's weight at `d` pixels from its centre, of sigma `s`.
fn weight(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

/// Two pixels of the layer side by side on row `y`, the left one's centre
/// at `x`, premultiplied and weighed `wa` and `wb`. A colour cannot be
/// premultiplied once it is filtered, so the one fetch between them serves
/// only where they are as opaque as each other - all of an opaque picture;
/// elsewhere each is read on its own. The layer is read as the texture
/// holds it, which is what `sample` hands a scene-linear package, without
/// derivatives, so the reading may turn on what it reads.
fn pair(x: f32, y: f32, wa: f32, wb: f32) -> vec4<f32> {
    let w = wa + wb;
    let both = textureSampleLevel(source, source_sampler, vec2<f32>(x + wb / w, y) / frame.size, 0.0);
    if (abs(both.a - round(both.a)) < 1e-4) {
        return vec4<f32>(both.rgb * both.a, both.a) * w;
    }
    let row = i32(floor(y));
    let last = i32(frame.size.x) - 1;
    let left = i32(floor(x));
    let a = textureLoad(source, vec2<i32>(clamp(left, 0, last), row), 0);
    let b = textureLoad(source, vec2<i32>(clamp(left + 1, 0, last), row), 0);
    return vec4<f32>(a.rgb * a.a, a.a) * wa + vec4<f32>(b.rgb * b.a, b.a) * wb;
}

/// The layer weighed along its row about `uv`.
fn across(uv: vec2<f32>) -> vec4<f32> {
    let s = sigma();
    let centre = uv.x * frame.size.x;
    let row = uv.y * frame.size.y;
    let first = floor(centre - REACH * s - 0.5) + 0.5;
    let pairs = i32(ceil(REACH * s)) + 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < pairs; i++) {
        let a = first + f32(2 * i);
        let wa = weight(a - centre, s);
        let wb = weight(a + 1.0 - centre, s);
        sum += pair(a, row, wa, wb);
        total += wa + wb;
    }
    return sum / total;
}

/// `across` weighed down its column about `uv`, in its own pixels.
fn column(uv: vec2<f32>) -> vec4<f32> {
    let rows = 1.0 / across_texel().y;
    let s = sigma() * rows / frame.size.y;
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

fn down(uv: vec2<f32>) -> vec4<f32> {
    return column(uv);
}

/// `down` read back over the layer. Between its pixel centres that is its
/// bilinear filtering; past the outermost ones - the layer's last few
/// pixels at an edge, when `down` has fewer - the blur goes on as it was
/// going, its slope taken from the pixel in from the edge, where clamping
/// would hold it flat.
fn spread(uv: vec2<f32>) -> vec4<f32> {
    let size = 1.0 / down_texel();
    let at = uv * size - 0.5;
    let inside = clamp(at, vec2<f32>(0.0), size - 1.0);
    var blurred = down_at((inside + 0.5) / size);
    let over = at - inside;
    if (any(over != vec2<f32>(0.0))) {
        let back = -sign(over);
        let x = down_at((inside + vec2<f32>(back.x, 0.0) + 0.5) / size);
        let y = down_at((inside + vec2<f32>(0.0, back.y) + 0.5) / size);
        let xy = down_at((inside + back + 0.5) / size);
        let d = abs(over);
        blurred += (blurred - x) * d.x + (blurred - y) * d.y + (blurred - x - y + xy) * d.x * d.y;
    }
    return blurred;
}

/// Premultiplied light back to a straight colour and its alpha.
fn straight(blurred: vec4<f32>) -> vec4<f32> {
    let alpha = clamp(blurred.a, 0.0, 1.0);
    return vec4<f32>(blurred.rgb / max(alpha, 1e-6), alpha);
}

/// Below a radius of six `across` keeps the layer's width (see the
/// manifest), and blurring down it here spares drawing `down` the layer's
/// size only to read it back; `down` is then a sliver nothing reads.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    if (across_texel().x * frame.size.x < 1.5) {
        return straight(column(uv));
    }
    return straight(spread(uv));
}
