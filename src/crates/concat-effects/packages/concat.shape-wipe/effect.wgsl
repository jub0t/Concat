// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Shape Wipe: an opening in the shape of a circle, a heart or a star,
// round on any frame, that grows from the centre until even its narrowest
// part is past the corners. Each shape is a distance field - how far a
// point is outside the shape, in units of its size - so the rim can be
// softened evenly all round.

struct Params { shape: f32, softness: f32 }

// A point in pixels about the centre of the frame, and back.
fn centred(uv: vec2<f32>) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) * frame.size;
}

fn uncentred(d: vec2<f32>) -> vec2<f32> {
    return d / frame.size + vec2<f32>(0.5);
}

// A unit circle.
fn circle(q: vec2<f32>) -> f32 {
    return length(q) - 1.0;
}

// A heart: a square stood on its corner, with a round lobe on each upper
// side, the whole about two units high and centred.
fn heart(q0: vec2<f32>) -> f32 {
    let q = vec2<f32>(abs(q0.x), q0.y + 0.15);
    // The square, turned forty-five degrees, side 1.2.
    let r = vec2<f32>(q.x + q.y, q.y - q.x) * 0.70710678;
    let box = abs(r - vec2<f32>(0.0, 0.0)) - vec2<f32>(0.6);
    let square = length(max(box, vec2<f32>(0.0))) + min(max(box.x, box.y), 0.0);
    // The lobe: centred on the square's upper side.
    let lobe = length(q - vec2<f32>(0.4243, 0.4243)) - 0.6;
    return min(square, lobe);
}

// A five-pointed star of outer radius one, point up.
fn star(q0: vec2<f32>) -> f32 {
    let sector = 1.25663706;
    // Fold the plane into one tenth: half a point.
    var a = atan2(q0.x, q0.y);
    a = abs(a - sector * round(a / sector));
    let q = vec2<f32>(sin(a), cos(a)) * length(q0);
    let tip = vec2<f32>(0.0, 1.0);
    let notch = vec2<f32>(sin(0.62831853), cos(0.62831853)) * 0.4;
    let e = notch - tip;
    let h = clamp(dot(q - tip, e) / dot(e, e), 0.0, 1.0);
    let d = length(q - tip - e * h);
    // Inside when on the centre's side of the edge.
    let side = e.x * (q.y - tip.y) - e.y * (q.x - tip.x);
    return select(d, -d, side < 0.0);
}

// The shape's distance at `q` and the size it must reach to clear the
// corners: the corner distance over its narrowest radius.
fn shape(q: vec2<f32>, kind: i32) -> f32 {
    if (kind == 1) {
        return heart(q);
    }
    if (kind == 2) {
        return star(q);
    }
    return circle(q);
}

fn reach(kind: i32) -> f32 {
    if (kind == 1) {
        return 1.6;
    }
    if (kind == 2) {
        return 2.7;
    }
    return 1.0;
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let kind = i32(round(clamp(params.shape, 0.0, 2.0)));
    let corner = 0.5 * length(frame.size);
    let half = max(clamp(params.softness, 0.0, 100.0) / 100.0 * frame.size.y / 10.0, 0.75);
    // Grows slowly, then fast: the shape is seen whole before it fills.
    let size = max((corner * reach(kind) + 2.0 * half) * p * p * (2.0 - p), 1e-3);
    let d = centred(uv) * vec2<f32>(1.0, -1.0);
    let dist = shape(d / size, kind) * size;
    let w = 1.0 - smoothstep(-half, half, dist);
    return mix(from_at(uv), to_at(uv), w);
}
