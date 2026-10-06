struct Params { amount: f32, speed: f32, columns: f32 }

const LAYERS: i32 = 2;

// How hard column `col` is jolted at `t`: once every `period` seconds each
// column snaps out and settles back, a little after the one to its left.
// Returns the strength, 0..1, and the beat it belongs to.
fn jolt(t: f32, col: f32, period: f32) -> vec2<f32> {
    let at = t - col * period * 0.04;
    let beat = floor(at / period);
    let local = (at - beat * period) / period;
    let rise = smoothstep(0.0, 0.05, local);
    let fall = 1.0 - smoothstep(0.05, 0.32, local);
    return vec2<f32>(rise * fall, beat);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    let n = max(round(params.columns), 1.0);
    let col = floor(clamp(uv.x, 0.0, 0.9999) * n);
    let period = mix(2.2, 0.35, params.speed / 100.0);
    let j = jolt(frame.time, col, period);
    let env = j.x * amount;
    let up = select(-1.0, 1.0, hash(vec2<f32>(col, j.y), 3.0) < 0.5);
    let reach = 0.08 * (0.5 + 0.9 * hash(vec2<f32>(col, j.y), 7.0));
    let edge = vec2<f32>(0.0);
    let far = vec2<f32>(1.0);

    let moved = sample(clamp(uv + vec2<f32>(0.0, env * reach * up), edge, far));
    var ghost = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k: i32 = 1; k <= LAYERS; k++) {
        let layer = f32(k) / f32(LAYERS);
        let shift = env * reach * (1.0 + layer * 0.9) * up;
        let g = sample(clamp(uv + vec2<f32>(0.0, -shift * 0.6), edge, far)).rgb;
        let w = 1.0 - layer * 0.4;
        ghost += g * w;
        weight += w;
    }
    ghost = ghost / max(weight, 1e-5);
    ghost = mix(ghost, ghost * vec3<f32>(1.0, 0.94, 0.78), 0.35);

    let leak = vec3<f32>(1.0, 0.72, 0.45) * env * 0.08 * (0.6 + 0.4 * sin((uv.x + uv.y) * 12.0));
    var out = mix(moved.rgb, max(moved.rgb, ghost), 0.55 * env);
    out = screen(out, leak);
    out = mix(c.rgb, out, min(env * 3.0, 1.0));
    out = out + grain_at(uv, 0.06 * env);
    return vec4<f32>(out, c.a);
}
