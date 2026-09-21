# Writing effects for Concat

Every effect, filter, transition and audio effect in Concat is a **package**:
a folder with a short manifest and, beside it, a filter chain or a shader.
The ones that ship with the app are packages too, and nothing in the app's
code names any of them. A package you write goes in a folder on your
machine and shows up on the same shelves as the built-ins, and a package you
share is that folder, zipped.

## Where packages live

Concat reads every folder under its effects directory:

| Platform | Folder |
|---|---|
| macOS | `~/Library/Application Support/concat/effects` |
| Windows | `%APPDATA%\concat\effects` |
| Linux | `~/.config/concat/effects` (or `$XDG_CONFIG_HOME/concat/effects`) |
| Portable build | `portable/effects` beside the executable |

The app watches that folder while it runs. Add a package, edit its files,
or delete it, and the shelves follow within a few seconds. The **Reload
custom effects** button on the Effects and Filters pages does the same on
demand, and its notice names the folder, so you never have to remember the
path. A package that will not load is skipped, the reason appears in the
window, and every reason is in the log (Settings › About opens it).

## The smallest package

A package is a folder named after its id, holding `effect.toml`:

```
effects/
└── ada.warm-shadows/
    └── effect.toml
```

```toml
format = 1

[effect]
id = "ada.warm-shadows"
name = "Warm Shadows"
kind = "filter"
category = "Colour"
description = "A little amber in the shadows, the highlights left alone."

[[param]]
key = "amount"
label = "Amount"
min = 0
max = 100
default = 50
unit = "%"

[ffmpeg]
let = ["a = amount / 100 * 0.3"]
chain = "colorbalance=rs={fixed(a, 3)}:gs={fixed(a * 0.4, 3)}"
```

Save it, and Warm Shadows is on the Filters page under Colour, with one
slider. That is a whole package.

## The manifest

`effect.toml` is TOML. `format = 1` at the top says which package format
the file is written for; it may be left out, which means 1. A future Concat
that changes the format will bump the number, and a package written for a
newer one is refused with a message saying so rather than failing somewhere
confusing.

### `[effect]`

| Key | Meaning |
|---|---|
| `id` | `author.name`, lower-case letters, digits and hyphens. Stored in project files forever, so pick it once. The folder must be named after it. `concat.` is the built-ins' author and is refused for your own. |
| `name` | What the card says. |
| `kind` | `effect`, `filter`, `audio`, `transition` or `generator`. See below. |
| `category` | The shelf the card sits on, free text. `Imported` is where LUT imports go. |
| `order` | Sort key within the category; ties break on name. |
| `version` | Your own number, bumped when the output changes. Default 1. |
| `intensity` | For an effect: the parameter the simple view shows as its one slider. |
| `aliases` | Earlier ids this package answers to, so old projects keep rendering. |
| `description` | One sentence for the tooltip. |

### Kinds

- **filter**: a colour look, picture in, picture out. Every filter gets an
  intensity control for free. Below full intensity the host mixes your result
  with the untouched picture, so your chain or shader always renders the look
  at full strength and never has to mix itself.
- **effect**: picture in, picture out, no automatic mixing. Blur,
  distortion, texture, motion.
- **audio**: sound in, sound out. An FFmpeg chain of audio filters.
- **transition**: two pictures and a progress in, one picture out. A shader
  under `[transition]`, never a chain.
- **generator**: no picture in, a picture out. A shader.

### `[[param]]`

One table per control, in the order the inspector shows them.

| Key | Meaning |
|---|---|
| `key` | The name the backend reads and the document stores. Lower-case letters, digits and underscores. `index` and `intensity` are reserved. |
| `label` | What the control is called. |
| `type` | `float` (default), `int`, `bool`, `enum`, `color`, `point`. |
| `min`, `max`, `default`, `step` | The range, the untouched value, and the slider increment. `max` defaults to 1, `step` 0 means continuous. |
| `unit` | Shown after the number: `%`, `dB`, `K`, `s`. |
| `group` | A subhead; consecutive params with the same group share one. |
| `animate` | Whether the control can carry keyframes. |
| `values`, `labels` | For `enum`: what the document may hold, and what each is called. |

Every declared parameter must be read by the backend, and a backend may not
read one the manifest does not declare. Both are caught at load.

## The FFmpeg backend

```toml
[ffmpeg]
let = ["k = clamp(strength / 100, 0, 1)", "r = round(k * 20)"]
chain = "gblur=sigma={fixed(k * 4, 2)},unsharp=5:5:{fixed(r / 10, 2)}"
```

`chain` is FFmpeg filter syntax with `{expression}` slots. `let` names
intermediate values, evaluated in order; each may read the parameters and
the names before it. `{{` and `}}` are literal braces.

An expression is a number, a parameter, arithmetic with `+ - * /`, a
comparison with `< > <= >= == !=` (1 or 0), and these functions:

| Function | Result |
|---|---|
| `round(x)`, `floor(x)`, `ceil(x)` | A whole number, printed without a decimal point. |
| `abs(x)`, `sqrt(x)`, `pow(x, y)` | As you would expect. |
| `min(a, b)`, `max(a, b)`, `clamp(x, lo, hi)` | Bounds. |
| `if(cond, a, b)` | `a` when `cond` is not 0, else `b`. |
| `fixed(x, n)` | Text with exactly `n` decimals, which is how a filtergraph wants most numbers. |

Two names are always there: `index`, the fragment's position in the clip's
chain, which any filtergraph labels you write must embed
(`split[a{index}][b{index}]`); and `lut`, the quoted path of the table a
`[lut]` section names.

**Which filters a chain may use.** A chain runs inside the app's own
process, and FFmpeg has filters that open files, load plugins and take
commands. A package is a folder anyone can share, so a chain is held to a
list of filters that read the frame and write the frame and nothing else.
The list is in
[`src/crates/concat-effects/src/filters.rs`](src/crates/concat-effects/src/filters.rs):
the colour, blur, distortion, keying and stacking filters, `split`, `blend`
and `overlay` for graphs, `lut3d` and `lut1d` for tables, and the tone,
dynamics, delay and pitch filters for sound. A chain naming anything else is
refused at load, and a filter's name has to be spelt out, never come from a
slot. If a pure pixel or sound filter you need is missing, add it to the
list in a pull request.

## The WGSL backend

```toml
[wgsl]
entry = "effect.wgsl"
```

`effect.wgsl` beside the manifest declares a `Params` struct with one field
per parameter and the entry the host calls:

```wgsl
struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let warm = split_tone(c.rgb, vec3<f32>(1.0, 0.75, 0.5), vec3<f32>(1.0), params.amount / 100.0);
    return vec4<f32>(warm, c.a);
}
```

A `float`, `int`, `bool` or `enum` parameter is an `f32` field; a `point` is
a `vec2<f32>`; a `color` is a `vec4<f32>`. Every declared parameter must be
a field; a field the manifest does not declare stays zero.

The host wraps your file in its half of the contract. You get:

- `sample(uv)`: the layer's colour at `uv`, straight alpha.
- `texel()`: one pixel as a fraction of the layer.
- `luma(rgb)`: Rec. 709 luminance.
- `lut(rgb)`: the package's table applied, the identity when it ships none.
- `hash(p, seed)`: a number in 0..1 for grain and dither.
- `reveal_order(uv)`: over a title, which word is painted here, 0..1; 0
  everywhere else.
- `frame.size` in pixels, `frame.time` in seconds into the timeline,
  `frame.clip_time` in seconds since this clip began, and `frame.intensity`,
  which the host applies for you after `effect` returns.
- The grading library: `saturation`, `vibrance`, `contrast`, `s_curve`,
  `fade`, `lift_gamma_gain`, `split_tone`, `tint_midtones`, `white_balance`,
  `kelvin`, `vignette`, `mono`, `matte`, `film_curve`, `hue_rotate`,
  `hsl_band`, `halation`, `soften`, `grain_at`, `edge_at`, and the masks
  `hue_mask`, `skin_mask`, `shadows`, `highlights`, `midtones`. Their
  signatures are at the top of
  [`src/crates/concat-effects/src/shader.rs`](src/crates/concat-effects/src/shader.rs).

Two rules the loader enforces: every loop must have a `break` or a
`break if` in it, and the shader may not declare bindings of its own. Both
keep a shared shader from hanging or reaching past its inputs.

A package may carry both backends. The shader renders wherever there is a
GPU; the chain is what a machine without one gets, and what the card
preview is rendered from. The built-in looks do this.

### Transitions

```toml
[transition]
entry = "effect.wgsl"
xfade = "dissolve"
fallback = "cross-fade"
```

The shader declares `fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>`
and reads `from_at(uv)` and `to_at(uv)`, the outgoing and incoming pictures;
`sample` reads the outgoing one, so the grading helpers work on it unchanged.
`progress` runs 0 to 1 across the cut, and the shader owns the whole blend.
`xfade` names a built-in FFmpeg `xfade` transition for exports without a
GPU, and `fallback` a built-in transition id to degrade to; both are
optional.

### Tables

```toml
[lut]
file = "look.cube"
```

A `.cube` table beside the manifest. The shader reads it through `lut(rgb)`;
a chain names its path as `{lut}`, as in `lut3d=file={lut}`. Importing a
`.cube` from the Filters page writes exactly this package for you.

## Fixtures

`fixtures.toml` pins what the chain renders for given parameters, so a
change to the package, or to the expression language, that alters the
output is caught rather than shipped:

```toml
[[case]]
name = "as shot"
chain = "colorbalance=rs=0.150:gs=0.060"

[[case]]
name = "all the way"
at = "max"
chain = "colorbalance=rs=0.300:gs=0.120"

[[case]]
name = "a little"
params = { amount = 10 }
chain = "colorbalance=rs=0.030:gs=0.012"
```

`at` is `default`, `min` or `max` for every parameter, and `params` sets
some on top of it. `index` sets the fragment position. Every FFmpeg package
that ships with the app pins its default and both ends of every slider.

## The card

A package with a chain gets its card rendered for you the first time it
loads: the reference picture through the chain at its defaults, written
beside the manifest as `preview.jpg`. Put your own `preview.png` there to
use instead, which is the only way a shader-only package gets a picture.

## Checking a package

The command line does everything the app does on load, and runs the
fixtures, and says what is wrong:

```
concat-cli check ~/Library/Application\ Support/concat/effects/ada.warm-shadows
concat-cli check ~/Library/Application\ Support/concat/effects
```

One folder checks one package; a folder of folders checks them all. It
exits non-zero when any fails, so it runs in a CI job on a repository of
packages. Build it from the source tree with `cargo build -p concat-cli`
in `src/`.

## Sharing

A package is its folder. Zip it, and whoever receives it unzips it into
their effects folder. Keep the `format` line, pin fixtures for anything with
a chain, and include a `preview.png` if the package is shader-only. The
built-ins under
[`src/crates/concat-effects/packages/`](src/crates/concat-effects/packages/)
are 170-odd worked examples: `concat.sepia` is a chain and a shader,
`concat.bass` is audio, `concat.blur-dissolve` is a transition, and
`concat.adjust` shows parameters with groups and a full set of fixtures.
