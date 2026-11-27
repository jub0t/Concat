// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Mirror: the picture folded at its middle, one side reflected over the
// other as a mirror stood on the line would show it.

struct Params { mode: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let pick = i32(round(clamp(params.mode, 0.0, 4.0)));
    var p = uv;
    if (pick == 0) {
        p.x = min(p.x, 1.0 - p.x);
    } else if (pick == 1) {
        p.x = max(p.x, 1.0 - p.x);
    } else if (pick == 2) {
        p.y = min(p.y, 1.0 - p.y);
    } else if (pick == 3) {
        p.y = max(p.y, 1.0 - p.y);
    } else {
        p = min(p, vec2<f32>(1.0) - p);
    }
    return sample(p);
}
