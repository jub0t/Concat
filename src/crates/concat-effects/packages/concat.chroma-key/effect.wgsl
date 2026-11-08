struct Params {
    color: vec4<f32>,
    strength: f32,
    shadows: f32,
    balance: f32,
    clip_black: f32,
    clip_white: f32,
    grow: f32,
    feather: f32,
    spill: f32,
    restore: f32,
    view: f32,
}

// A colour-difference key, worked in light. A screen is lit in one channel
// above the other two - green over red and blue, or blue over red and
// green - and how far a pixel's own channel stands above the others, set
// against how far the screen's does, is how much of the pixel is screen:
//
//     d(C) = C_p - limit(C),   alpha = 1 - strength * d(C) / d(S)
//
// where p is the screen's channel, S its colour and the limit the larger of
// the other two (see `limit`). Every colour whose own channel does not
// stand above both others - skin, a grey, a yellow, a cyan - keeps its
// whole alpha; a shadow on the screen is the screen with less light, and
// d(S) is measured against the screen about the pixel (the `plate`), so a
// fold or a dark corner goes with the rest of it. The matte is read off the
// picture with its colour smoothed by a pixel or two, which is all the
// colour a 4:2:0 stream holds, so the edge does not crawl from frame to
// frame; it is levelled by the clip knobs, drawn in or let out, and
// softened.
//
// What stays is unmixed: a pixel part screen is `alpha F + (1 - alpha) S`,
// so the screen's share is taken off and what is left divided back up,
// which leaves an edge - hair, a blur of motion - the colour of what was
// in front, not tinged with the screen. Then the spill: the screen's
// channel held to the limit of the others, as much as `spill` says, and
// the light that took off given back as grey, as much as `restore` says.
// The result is straight alpha, as every layer is.

/// The key colour as picked, or nothing - its alpha left at zero - for the
/// key to find the screen in the picture.
fn picked() -> bool {
    return params.color.a > 0.5;
}

/// The screen's own channel: 0 red, 1 green, 2 blue, the brightest of its
/// three.
fn dominant(s: vec3<f32>) -> i32 {
    if (s.g >= s.r && s.g >= s.b) {
        return 1;
    }
    if (s.b >= s.r) {
        return 2;
    }
    return 0;
}

/// A colour turned so the screen's channel comes first and the other two
/// follow in the order red, green, blue.
fn turned(c: vec3<f32>, p: i32) -> vec3<f32> {
    if (p == 1) {
        return vec3<f32>(c.g, c.r, c.b);
    }
    if (p == 2) {
        return vec3<f32>(c.b, c.r, c.g);
    }
    return c;
}

/// `turned` undone.
fn unturned(t: vec3<f32>, p: i32) -> vec3<f32> {
    if (p == 1) {
        return vec3<f32>(t.y, t.x, t.z);
    }
    if (p == 2) {
        return vec3<f32>(t.y, t.z, t.x);
    }
    return t;
}

/// How high the screen's channel of a colour with no screen in it reaches:
/// the larger of the other two, each weighed by the balance - both whole at
/// 50 %, the second fading out towards 100 % and the first towards 0 %.
fn limit(t: vec3<f32>) -> f32 {
    let w = clamp(params.balance / 100.0, 0.0, 1.0);
    return max(min(1.0, 2.0 * w) * t.y, min(1.0, 2.0 * (1.0 - w)) * t.z);
}

/// How far a colour's screen channel stands above the limit of the others.
fn excess(c: vec3<f32>, p: i32) -> f32 {
    let t = turned(c, p);
    return t.x - limit(t);
}

/// Whether a screen colour can be keyed: its own channel stands clear of
/// both others. A black, a white or a grey has none to stand above them,
/// and keying it would take out every grey in the picture.
fn keyable(s: vec4<f32>) -> bool {
    let t = turned(s.rgb, dominant(s.rgb));
    return s.a > 0.5 && t.x > 0.002 && t.x - max(t.y, t.z) >= 0.1 * t.x;
}

/// One pixel of a 1080-line picture, in this layer's pixels.
fn scale() -> f32 {
    return max(min(frame.size.x, frame.size.y) / 1080.0, 0.25);
}

/// The layer's mean colour over the 16 pixels square of a cell, straight,
/// with the share of it that is there as its alpha: sixteen fetches, each
/// between four pixels.
fn cells(uv: vec2<f32>) -> vec4<f32> {
    let centre = uv * frame.size;
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let at = centre + vec2<f32>(f32(i) * 4.0 - 6.0, f32(j) * 4.0 - 6.0);
            sum += sample_premultiplied(at / frame.size);
        }
    }
    let mean = sum / 16.0;
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), clamp(mean.a, 0.0, 1.0));
}

/// The cells looked through for the screen, across and down.
const LOOK_ACROSS: i32 = 32;
const LOOK_DOWN: i32 = 18;

/// Whether a cell's colour is strongly of the screen's kind: green, or
/// blue, standing above the other two by two fifths of itself.
fn screenish(c: vec4<f32>, green: bool) -> bool {
    let m = max(max(c.r, c.g), c.b);
    let over = select(c.b - max(c.r, c.g), c.g - max(c.r, c.b), green);
    return c.a >= 0.5 && over > max(0.4 * m, 0.004);
}

/// A colour's chromaticity: its share of each channel.
fn shares(c: vec3<f32>) -> vec3<f32> {
    return c / max(c.r + c.g + c.b, 1e-6);
}

/// The screen's colour in light, alpha 1, or alpha 0 for no screen. A
/// picked colour is taken as it is, out of the display encoding it was
/// picked in. Otherwise it is found: of the cells, the strongly green ones
/// and the strongly blue ones are counted, the more numerous are the
/// screen if there are enough of them to be one, and their mean - taken
/// again over those near it in chromaticity, so a green prop or a blue
/// shirt does not pull it - is its colour. Every pixel of this pass finds
/// the same colour.
fn backing(uv: vec2<f32>) -> vec4<f32> {
    if (picked()) {
        return vec4<f32>(from_display(params.color.rgb), 1.0);
    }
    let grid = vec2<f32>(f32(LOOK_ACROSS), f32(LOOK_DOWN));
    var greens = vec4<f32>(0.0);
    var blues = vec4<f32>(0.0);
    var seen = 0.0;
    // No `continue`: naga writes a loop that has one for Direct3D as a
    // `while` with a first-time flag, and this pass - the one shader here
    // with a `continue` - left the device lost on Windows CI's WARP under
    // FXC, Windows' pinned compiler, and every effect drawn after it went
    // black (#673). The `screenish` tests already ask for alpha, so the
    // guard is only for the count.
    for (var j = 0; j < LOOK_DOWN; j++) {
        for (var i = 0; i < LOOK_ACROSS; i++) {
            let c = cells_at((vec2<f32>(f32(i), f32(j)) + 0.5) / grid);
            if (c.a >= 0.5) {
                seen += 1.0;
                if (screenish(c, true)) {
                    greens += vec4<f32>(c.rgb, 1.0);
                }
                if (screenish(c, false)) {
                    blues += vec4<f32>(c.rgb, 1.0);
                }
            }
        }
    }
    let green = greens.w >= blues.w;
    let found = select(blues, greens, green);
    if (found.w < max(2.0, 0.02 * seen)) {
        return vec4<f32>(0.0);
    }
    let first = shares(found.rgb / found.w);
    var near = vec4<f32>(0.0);
    for (var j = 0; j < LOOK_DOWN; j++) {
        for (var i = 0; i < LOOK_ACROSS; i++) {
            let c = cells_at((vec2<f32>(f32(i), f32(j)) + 0.5) / grid);
            if (screenish(c, green) && distance(shares(c.rgb), first) < 0.12) {
                near += vec4<f32>(c.rgb, 1.0);
            }
        }
    }
    let mean = select(found.rgb / found.w, near.rgb / max(near.w, 1.0), near.w >= 1.0);
    return vec4<f32>(mean, 1.0);
}

/// The screen's colour, whichever pixel asks.
fn screen_colour() -> vec4<f32> {
    return backing_at(vec2<f32>(0.5));
}

/// How strongly the screen shows about a cell: the most any of it and the
/// eight about it stand above their limit, or -1 where none of them is
/// there. On the screen that is the screen's own strength there, lit or in
/// shadow; at the subject's edge it is the screen's beside it.
fn plate(uv: vec2<f32>) -> vec4<f32> {
    let s = screen_colour();
    let p = dominant(s.rgb);
    let pitch = cells_texel();
    var most = -1.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = cells_at(uv + vec2<f32>(f32(i), f32(j)) * pitch);
            if (c.a >= 0.5) {
                most = max(most, excess(c.rgb, p));
            }
        }
    }
    return vec4<f32>(most, 0.0, 0.0, 1.0);
}

/// What the screen's excess is measured against at `uv`: the screen about
/// the pixel where that is near enough the key colour to be the screen in
/// shadow - within `shadows` of it, or brighter - and the key colour's own
/// elsewhere, so a subject with a little spill on it, far from the screen,
/// is not keyed against its own faint green.
fn reach(uv: vec2<f32>, s: vec3<f32>, p: i32) -> f32 {
    let whole = max(excess(s, p), 1e-4);
    let local = plate_at(uv).r;
    let least = 1.0 - params.shadows / 100.0;
    let near = smoothstep(least - 0.15, least, local / whole);
    return mix(whole, max(local, 1e-4), near);
}

/// The matte, levelled: 0 the screen, 1 the subject. Read off the picture
/// with its colour smoothed by a five-by-five binomial (sigma one pixel) in
/// nine fetches, each between pixels; the excess is the same whatever grey
/// is added to a colour, so the smoothing is of its colour alone.
fn matte(uv: vec2<f32>) -> vec4<f32> {
    let s = screen_colour();
    if (!keyable(s)) {
        return vec4<f32>(1.0);
    }
    let p = dominant(s.rgb);
    let pitch = texel() * 1.2;
    var sum = vec4<f32>(0.0);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let w = select(5.0, 6.0, i == 0) * select(5.0, 6.0, j == 0);
            sum += sample_premultiplied(uv + vec2<f32>(f32(i), f32(j)) * pitch) * w;
        }
    }
    let smooth_colour = sum.rgb / max(sum.a, 1e-6);
    let raw = 1.0 - params.strength / 100.0 * excess(smooth_colour, p) / reach(uv, s.rgb, p);
    let black = params.clip_black / 100.0;
    let white = max(params.clip_white / 100.0, black + 0.01);
    return vec4<f32>(clamp((raw - black) / (white - black), 0.0, 1.0), 0.0, 0.0, 1.0);
}

/// How many pixels the matte is drawn in or let out by.
fn grown() -> i32 {
    return min(i32(round(abs(params.grow) * scale())), 64);
}

/// The matte let out (the most about it) or drawn in (the least), along
/// one line.
fn grow_across(uv: vec2<f32>) -> vec4<f32> {
    let along = vec2<f32>(texel().x, 0.0);
    var v = matte_at(uv).r;
    for (var i = 1; i <= grown(); i++) {
        let a = matte_at(uv - along * f32(i)).r;
        let b = matte_at(uv + along * f32(i)).r;
        v = select(min(v, min(a, b)), max(v, max(a, b)), params.grow > 0.0);
    }
    return vec4<f32>(v, 0.0, 0.0, 1.0);
}

fn grow_down(uv: vec2<f32>) -> vec4<f32> {
    let along = vec2<f32>(0.0, texel().y);
    var v = grow_across_at(uv).r;
    for (var i = 1; i <= grown(); i++) {
        let a = grow_across_at(uv - along * f32(i)).r;
        let b = grow_across_at(uv + along * f32(i)).r;
        v = select(min(v, min(a, b)), max(v, max(a, b)), params.grow > 0.0);
    }
    return vec4<f32>(v, 0.0, 0.0, 1.0);
}

/// The matte with its growing done, or as it came while there is none to
/// do; the manifest draws the growing passes at a sliver then.
fn shaped(uv: vec2<f32>) -> f32 {
    if (abs(params.grow) < 0.1) {
        return matte_at(uv).r;
    }
    return grow_down_at(uv).r;
}

/// The softening's sigma, in this layer's pixels.
fn sigma() -> f32 {
    return max(params.feather * scale(), 0.3);
}

/// The Gaussian's weight `d` pixels out.
fn weight(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

fn soften_across(uv: vec2<f32>) -> vec4<f32> {
    let s = sigma();
    let n = min(i32(ceil(3.0 * s)), 64);
    let along = vec2<f32>(texel().x, 0.0);
    var sum = 0.0;
    var total = 0.0;
    for (var i = -n; i <= n; i++) {
        let w = weight(f32(i), s);
        sum += shaped(uv + along * f32(i)) * w;
        total += w;
    }
    return vec4<f32>(sum / total, 0.0, 0.0, 1.0);
}

/// The finished matte at `uv`: softened down what `soften_across` drew, or
/// as it was shaped while there is no softening.
fn alpha_at(uv: vec2<f32>) -> f32 {
    if (params.feather < 0.1) {
        return shaped(uv);
    }
    let s = sigma();
    let n = min(i32(ceil(3.0 * s)), 64);
    let along = vec2<f32>(0.0, texel().y);
    var sum = 0.0;
    var total = 0.0;
    for (var i = -n; i <= n; i++) {
        let w = weight(f32(i), s);
        sum += soften_across_at(uv + along * f32(i)).r * w;
        total += w;
    }
    return sum / total;
}

/// What stays of a pixel: its colour with the screen's share taken off and
/// divided back up by its alpha, then its spill cleaned off and the light
/// that took given back as grey. Where next to nothing stays, the division
/// is of next to nothing, and the colour is the pixel's own with its spill
/// cleaned off instead: the layer is filtered as straight colour when it is
/// scaled, so the colour of a pixel that is all screen bleeds into the edge
/// beside it, and a grey bleeds in more quietly than a black or a noise.
fn foreground(c: vec3<f32>, alpha: f32, screen: vec3<f32>, p: i32) -> vec3<f32> {
    let left = max(c - (1.0 - alpha) * screen, vec3<f32>(0.0));
    let most = max(max(max(c.r, c.g), c.b), 1.0);
    let unmixed = min(left / max(alpha, 1e-3), vec3<f32>(most));
    let front = mix(c, unmixed, smoothstep(0.0, 0.25, alpha));
    let t = turned(front, p);
    let held = vec3<f32>(mix(t.x, min(t.x, limit(t)), clamp(params.spill / 100.0, 0.0, 1.0)), t.y, t.z);
    let cleaned = unturned(held, p);
    let lost = luma(front) - luma(cleaned);
    return cleaned + vec3<f32>(lost * clamp(params.restore / 100.0, 0.0, 1.0));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let s = screen_colour();
    let view = i32(round(params.view));
    var alpha = 1.0;
    var rgb = c.rgb;
    if (keyable(s)) {
        let p = dominant(s.rgb);
        alpha = alpha_at(uv);
        // The screen as it is about this pixel: the key colour, as bright
        // as the screen there is.
        let here = s.rgb * (reach(uv, s.rgb, p) / max(excess(s.rgb, p), 1e-4));
        rgb = foreground(c.rgb, alpha, here, p);
    }
    let a = alpha * c.a;
    if (view == 1) {
        // The matte as a grey whose display level is the alpha.
        return vec4<f32>(vec3<f32>(pow(a, 2.4)), 1.0);
    }
    if (view == 2) {
        // Black what goes, white what stays, grey what is part of both.
        let state = select(select(0.18, 1.0, a >= 0.998), 0.0, a <= 0.002);
        return vec4<f32>(vec3<f32>(state), 1.0);
    }
    return vec4<f32>(rgb, a);
}
