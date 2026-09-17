struct Params { glow: f32, scanlines: f32, split: f32, glitch: f32, grain: f32, hue: f32 }

// The hologram from holo-stage (https://github.com/jub0t/holo-stage), cut
// down to the knobs that matter. The source's hue is thrown away and its
// value laid onto a three-stop blue ramp; the edges are drawn back over it
// as glowing contours, then the artifacts of a projection that is not
// quite locked: a prism split from the centre, scan lines drifting, a
// refresh band sweeping, a few rows torn sideways, grain, a flutter and a
// vignette. Exposure, gamma, the sweep, the flicker and the vignette are
// held at the original's defaults.

const SHADOW: vec3<f32> = vec3<f32>(0.008, 0.071, 0.122);
const MID: vec3<f32> = vec3<f32>(0.122, 0.498, 0.722);
const HIGH: vec3<f32> = vec3<f32>(0.682, 0.949, 1.0);
const EDGE: vec3<f32> = vec3<f32>(0.498, 0.910, 1.0);

fn luma_at(uv: vec2<f32>) -> f32 {
    return luma(sample(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))).rgb);
}

// One luminance onto the ramp: this is the whole of the look, the rest is
// dressing.
fn ramp(l: f32) -> vec3<f32> {
    let v = pow(clamp(l, 0.0, 1.0), 0.9);
    var c = mix(SHADOW, MID, smoothstep(0.0, 0.6, v));
    c = mix(c, HIGH, smoothstep(0.55, 1.0, v));
    return c * 1.15;
}

fn effect(at: vec2<f32>) -> vec4<f32> {
    let src = sample(at);
    let glow = params.glow / 100.0 * 2.0;
    let depth = params.scanlines / 100.0;
    let split = params.split / 100.0;
    let glitch = params.glitch / 100.0;
    let grain = params.grain / 100.0 * 0.4;
    let time = frame.time;
    var uv = at;

    // A few bands of rows per frame shoved sideways, and a fine wobble
    // under everything.
    let row = floor(uv.y * 48.0);
    let seed = hash(vec2<f32>(row, floor(time * 9.0)), 1.0);
    let torn = step(1.0 - glitch * 0.25, seed);
    uv.x += torn * (hash(vec2<f32>(seed, 1.0), 2.0) - 0.5) * 0.12;
    uv.x += sin(uv.y * 90.0 + time * 3.0) * 0.0012 * glitch;

    // Red and blue taken along the line from the centre, a prism's fringe.
    let dir = uv - vec2<f32>(0.5);
    let shift = dir * split * 0.02;
    let lr = luma_at(uv + shift);
    let lg = luma_at(uv);
    let lb = luma_at(uv - shift);
    var col = ramp(lg);
    col.r += (lr - lg) * split * 1.5;
    col.b += (lb - lg) * split * 1.5;

    col += edge_at(uv) * glow * EDGE;

    // Scan lines counted across the frame, not in pixels, so the look holds
    // at any size; then the refresh band sweeping through.
    let scan = 1.0 - depth * 0.5 * (0.5 + 0.5 * sin(uv.y * 180.0 * 6.2831853 - time * 6.0));
    col *= scan;
    let sweep = fract(uv.y * 0.5 - time * 0.11);
    let bar = smoothstep(0.0, 0.05, sweep) * smoothstep(0.14, 0.05, sweep);
    col += bar * 0.18 * HIGH;

    col *= 1.0 + 0.036 * sin(time * 31.0) * sin(time * 7.3);
    col += grain_at(at, grain);

    let vig = 1.0 - 0.5 * pow(length(dir) * 1.45, 2.2);
    col *= max(vig, 0.0);

    col = hue_rotate(col, params.hue);
    return vec4<f32>(clamp01(col), src.a);
}
