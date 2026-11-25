// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Doors: one shot cut in two at the middle, each half sliding out its own
// way on an ease that leaves and lands gently. The shot behind settles
// from a little way back as the doors clear it, and the doors' inner
// edges cast a soft shadow on it. Closing is the same move run the other
// way, with the shots' roles swapped.

struct Params { direction: f32 }

fn ease(p: f32) -> f32 {
    let q = clamp(p, 0.0, 1.0);
    return q * q * q * (q * (q * 6.0 - 15.0) + 10.0);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let pick = i32(round(clamp(params.direction, 0.0, 3.0)));
    let closing = pick >= 2;
    let across = pick == 0 || pick == 2;
    let p = clamp(progress, 0.0, 1.0);
    let open = ease(select(p, 1.0 - p, closing));
    // Where along the split's axis, and which door's half of the frame.
    let a = select(uv.y, uv.x, across);
    let side = select(1.0, -1.0, a < 0.5);
    // A door has slid half a frame by the time it is clear: the point
    // under `uv` on it is that far back towards the middle.
    let shift = -side * 0.5 * open;
    let door_a = a + shift;
    let on_door = select(door_a > 0.5, door_a < 0.5, side < 0.0);
    let door_uv = select(vec2<f32>(uv.x, door_a), vec2<f32>(door_a, uv.y), across);
    // The shot behind comes forward from 92 % as the doors open.
    let zoom = mix(0.92, 1.0, open);
    let behind_uv = (uv - vec2<f32>(0.5)) * zoom + vec2<f32>(0.5);
    var door = from_at(door_uv);
    var behind = to_at(behind_uv);
    if (closing) {
        door = to_at(door_uv);
        behind = from_at(behind_uv);
    }
    // The gap's distance from the nearer door edge, in frames.
    let gap = abs(a - 0.5) - 0.5 * open;
    let shade = 1.0 - 0.45 * (1.0 - smoothstep(0.0, 0.06, gap)) * smoothstep(0.0, 0.05, open);
    if (on_door) {
        return door;
    }
    return vec4<f32>(behind.rgb * shade, behind.a);
}
