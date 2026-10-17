// Corona: a cool blue-white glow gathered off the picture's blown
// highlights. Where halation is light that went through the emulsion and
// bounced off the backing, this is the opposite fiction: light too bright
// for the frame spills outward as a cold corona, widened by one blur pass
// of twelve taps laid on a golden-angle spiral.
//
// A flat picture survives the normalised spiral unchanged - the weights
// sum to themselves - so on flat input the pass returns the gate's own
// value and fixtures.toml is closed form.

// The two things a package declares: its knobs, in the manifest's order,
// and `fn effect`. Everything else is the host's.
struct Params { threshold: f32, radius: f32, amount: f32, cool: f32 }

/// A tap's Gaussian weight at a pixel distance under a sigma in pixels.
fn spike(d: f32, s: f32) -> f32 {
    return exp(-d * d / (2.0 * s * s));
}

/// The layer's light where it is blown, in the display encoding, gated
/// above the threshold and premultiplied by alpha so a clear pixel adds
/// none. Straight colour out; the corona takes its hue later.
fn gate_at(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let d = to_display(max(c.rgb, vec3<f32>(0.0)));
    let level = luma(d) * max(c.a, 0.0);
    let th = params.threshold / 100.0;
    let opened = smoothstep(th, min(th + 0.3, 1.0), level);
    return vec4<f32>(d * opened * max(c.a, 0.0), c.a);
}

/// The blur pass itself: twelve taps on a golden-angle spiral, denser at
/// the middle, every tap weighed by how far it strayed. The reach is the
/// radius as a share of the picture, so a corona reads the same on a card
/// and a 4K cut.
fn bloom(uv: vec2<f32>) -> vec4<f32> {
    let size = frame.size;
    if (size.x < 1.0) {
        return gate_at(uv);
    }
    let sigma = params.radius / 100.0 * max(size.x, size.y) * 0.5;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < 12; i++) {
        let fi = f32(i) + 0.5;
        let ang = fi * 2.399963229728653;
        let rad = sigma * sqrt(fi / 12.0) * 2.0;
        let dir = vec2<f32>(cos(ang), sin(ang));
        let w = spike(rad, sigma);
        sum += gate_at(uv + dir * rad / size) * w;
        total += w;
    }
    if (total <= 0.0) {
        return gate_at(uv);
    }
    return sum / total;
}

/// The corona's colour at a coolness: zero a plain white spill, one the
/// icy blue of a lamp seen through a cold window.
fn corona_colour(k: f32) -> vec3<f32> {
    return mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(0.55, 0.78, 1.0), clamp(k / 100.0, 0.0, 1.0));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let base = to_display(c.rgb);
    let glow = bloom_at(uv);
    let bloom_level = luma(max(glow.rgb, vec3<f32>(0.0)));
    let lit = base + corona_colour(params.cool) * bloom_level * (params.amount / 100.0) * 1.8;
    return vec4<f32>(from_display(lit), c.a);
}
