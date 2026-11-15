// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Cube: the shots are two faces of a box turning about its upright axis.
// Each point of the frame is a ray from a camera in front; it is traced to
// the faces it meets, the nearer one that faces the camera wins, and a
// face loses light as it turns from the camera. The box backs off a
// little at the half-turn so its leading edge stays in frame.

// Smoothstep's curve: leaves and lands with no jolt.
fn ease(p: f32) -> f32 {
    let q = clamp(p, 0.0, 1.0);
    return q * q * (3.0 - 2.0 * q);
}
// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}

// The camera's distance from the front face, in face widths.
const EYE: f32 = 1.6;

// Where a ray from the eye through screen point `s` meets a face, as the
// face's own uv, and how squarely the face looks at the eye; a negative
// `z` when the ray misses or meets its back.
// `corner` is the face's left edge, `run` its width along the floor.
fn meet(s: vec2<f32>, corner: vec2<f32>, run: vec2<f32>, aspect: f32) -> vec4<f32> {
    // The ray: from (0, -EYE) in the floor's x and depth, through (s.x, 0).
    let dir = vec2<f32>(s.x, EYE);
    let normal = vec2<f32>(run.y, -run.x);
    let denom = dot(dir, normal);
    let facing = -denom / length(dir);
    let t = dot(corner - vec2<f32>(0.0, -EYE), normal) / denom;
    let hit = vec2<f32>(0.0, -EYE) + dir * t;
    let u = dot(hit - corner, run) / dot(run, run);
    // Height scales with depth: the ray's own rise.
    let v = s.y * (t * EYE) / EYE;
    let ok = facing > 0.0 && t > 0.0 && u >= 0.0 && u <= 1.0 && abs(v) <= 0.5;
    return vec4<f32>(u, v + 0.5, select(-1.0, t, ok), facing);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let turn = ease(p) * 1.57079633;
    // Back off by up to a sixth, most at the half-turn.
    let s = (uv - vec2<f32>(0.5)) * (1.0 + 0.2 * bell(p) * bell(p));
    let aspect = frame.size.x / frame.size.y;
    // The box, one face wide, centred half a face behind the front face,
    // turned by `turn` - its front face swinging left and away.
    let c = cos(turn);
    let sn = sin(turn);
    let centre = vec2<f32>(0.0, 0.5);
    let fa = centre + vec2<f32>(-0.5 * c - 0.5 * sn, -0.5 * c + 0.5 * sn) ;
    let fb = centre + vec2<f32>(0.5 * c - 0.5 * sn, -0.5 * c - 0.5 * sn);
    let bb = centre + vec2<f32>(0.5 * c + 0.5 * sn, 0.5 * c - 0.5 * sn);
    let a = meet(s, fa, fb - fa, aspect);
    let b = meet(s, fb, bb - fb, aspect);
    var colour = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var depth = 1e9;
    if (a.z > 0.0) {
        let shade = mix(0.45, 1.0, a.w);
        let f = from_at(a.xy);
        colour = vec4<f32>(f.rgb * shade, f.a);
        depth = a.z;
    }
    if (b.z > 0.0 && b.z < depth) {
        let shade = mix(0.45, 1.0, b.w);
        let t = to_at(b.xy);
        colour = vec4<f32>(t.rgb * shade, t.a);
    }
    return colour;
}
