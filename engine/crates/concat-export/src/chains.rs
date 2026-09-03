// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The FFmpeg filter-chain builders: video effects and audio filters.
//!
//! Two catalogues - video effects and audio filters - and the composition
//! that turns a clip's applied entries into one filter string. The strings
//! are the ground truth for exported pixels and sound, and the tests at the
//! bottom of this file pin the exact chain for every catalogue entry at its
//! default, minimum and maximum settings, character for character. Number
//! formatting matters as much as the maths: "0.5" and "0.50" are different
//! bytes in a filtergraph even when they are the same sigma, so every
//! interpolation fixes its precision on purpose.
//!
//! Composition: enabled entries in applied order, comma-joined, bypassed
//! entries and unknown ids skipped, missing parameters falling back to the
//! catalogue defaults. An empty result is the empty string - the spelling
//! of "no chain".

use std::collections::BTreeMap;

use concat_project::model::AppliedFilter;

/// One slider on a catalogue entry: its key, the range the UI allows, and
/// the value an untouched slider means.
struct Param {
    key: &'static str,
    // The bounds are read only by the pinning tests, which push every
    // slider to each end - dead code to a plain build.
    #[cfg_attr(not(test), allow(dead_code))]
    min: f64,
    #[cfg_attr(not(test), allow(dead_code))]
    max: f64,
    default: f64,
}

/// The resolved parameter values for one applied entry: every declared key
/// present, either the user's number or the default.
struct Params(BTreeMap<&'static str, f64>);

impl Params {
    /// The value for a declared key. A chain function asking for a key its
    /// entry does not declare is a bug the tests catch, so this indexes.
    fn get(&self, key: &str) -> f64 {
        self.0[key]
    }
}

/// One catalogue entry: the id projects store, its sliders, and the function
/// from resolved parameters to an FFmpeg filter fragment.
struct Entry {
    id: &'static str,
    params: &'static [Param],
    /// Builds the fragment. `index` is the fragment's emitted position in
    /// the clip's chain: any filtergraph labels MUST embed it, or stacking
    /// the same effect twice duplicates labels and FFmpeg rejects the graph.
    chain: fn(&Params, usize) -> String,
}

/// Formats an `f64` the way JS `value.toFixed(digits)` does.
///
/// Both languages round the same IEEE double to decimal, so the only real
/// divergences are exact decimal ties (JS rounds them away from zero, Rust
/// to even - unreachable from the catalogues' slider grids) and negative
/// zero, which JS prints unsigned and this normalises to match.
fn fixed(value: f64, digits: usize) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value:.digits$}")
}

/// `Math.round` for the non-negative values the catalogues produce: nearest
/// integer, halves up. (JS and Rust disagree only on negative halves, which
/// no slider range here can reach.)
fn round(value: f64) -> i64 {
    value.round() as i64
}

// ─── the video effect catalogue ─────────────────────────────────────────────

fn fx_black_white(_: &Params, _index: usize) -> String {
    "hue=s=0".to_owned()
}

fn fx_sepia(_: &Params, _index: usize) -> String {
    // The standard sepia matrix, the same one the CSS filter defines.
    "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131".to_owned()
}

fn fx_invert(_: &Params, _index: usize) -> String {
    "negate".to_owned()
}

fn fx_sharpen(p: &Params, _index: usize) -> String {
    let amount = fixed(p.get("amount"), 2);
    format!("unsharp=5:5:{amount}:5:5:0")
}

fn fx_gaussian_blur(p: &Params, _index: usize) -> String {
    let sigma = fixed(p.get("radius"), 1);
    format!("gblur=sigma={sigma}")
}

fn fx_box_blur(p: &Params, _index: usize) -> String {
    let radius = round(p.get("radius"));
    format!("boxblur={radius}:1")
}

fn fx_motion_blur(p: &Params, _index: usize) -> String {
    // Gaussian in one axis only is the streak; the tiny vertical sigma keeps
    // the filter happy without visibly blurring that axis.
    let sigma = fixed(p.get("length"), 1);
    format!("gblur=sigma={sigma}:sigmaV=0.1")
}

fn fx_temperature(p: &Params, _index: usize) -> String {
    let temperature = round(p.get("temperature"));
    format!("colortemperature=temperature={temperature}")
}

fn fx_vibrance(p: &Params, _index: usize) -> String {
    let intensity = fixed(p.get("intensity"), 2);
    format!("vibrance=intensity={intensity}")
}

fn fx_contrast_pop(p: &Params, _index: usize) -> String {
    let contrast = fixed(p.get("contrast"), 2);
    format!("eq=contrast={contrast}")
}

fn fx_vignette(p: &Params, _index: usize) -> String {
    // The filter's angle runs 0..PI/2, wider being darker corners; the slider
    // maps into the range that reads as a vignette rather than a tunnel.
    let angle = fixed(0.25 + (p.get("strength") / 100.0) * 1.05, 3);
    format!("vignette=angle={angle}")
}

fn fx_film_grain(p: &Params, _index: usize) -> String {
    // Temporal (t) so the grain dances like film instead of sitting still.
    let amount = round(p.get("amount"));
    format!("noise=alls={amount}:allf=t+u")
}

/// Graph labels embed the emitted index so two glows on one clip cannot
/// collide in a single filtergraph: fixed labels would make FFmpeg reject
/// the graph.
fn fx_glow(p: &Params, index: usize) -> String {
    // Screen-blend a blurred copy over itself - the classic bloom.
    let opacity = fixed(p.get("amount") / 100.0, 2);
    format!(
        "split[glowa{index}][glowb{index}];[glowb{index}]gblur=sigma=18[glowg{index}];\
         [glowa{index}][glowg{index}]blend=all_mode=screen:all_opacity={opacity}"
    )
}

fn fx_posterize(p: &Params, _index: usize) -> String {
    let size = round(256.0 / round(p.get("levels")).max(2) as f64);
    let band = format!("trunc(val/{size})*{size}");
    format!("lutrgb=r={band}:g={band}:b={band}")
}

fn fx_pixelate(p: &Params, _index: usize) -> String {
    let size = round(p.get("size"));
    format!("pixelize=width={size}:height={size}")
}

/// Labels embed the emitted index for the same reason as [`fx_glow`]:
/// stacking mirror twice must not duplicate a filtergraph label.
fn fx_mirror(_: &Params, index: usize) -> String {
    format!(
        "crop=iw/2:ih:0:0,split[mirl{index}][mirr{index}];[mirr{index}]hflip[mirf{index}];\
         [mirl{index}][mirf{index}]hstack"
    )
}

fn fx_fisheye(p: &Params, _index: usize) -> String {
    // Negative correction coefficients produce barrel distortion - the bulge.
    let k = p.get("strength") / 100.0;
    let k1 = fixed(-0.55 * k, 3);
    let k2 = fixed(-0.2 * k, 3);
    format!("lenscorrection=k1={k1}:k2={k2}:i=bilinear")
}

fn fx_shake(p: &Params, _index: usize) -> String {
    // A jittering crop window; the decoder's guard scale stretches the
    // slightly smaller window back to full size afterwards.
    let a = round(p.get("amount"));
    let f = round(p.get("speed"));
    let f2 = round(f as f64 * 1.3);
    format!(
        "crop=iw-{}:ih-{}:{a}+{a}*sin(t*{f}):{a}+{a}*cos(t*{f2})",
        2 * a,
        2 * a
    )
}

/// Every video effect, in the catalogue's order. Ids are forever: they are
/// written into project files.
static EFFECTS: &[Entry] = &[
    Entry {
        id: "black-white",
        params: &[],
        chain: fx_black_white,
    },
    Entry {
        id: "sepia",
        params: &[],
        chain: fx_sepia,
    },
    Entry {
        id: "invert",
        params: &[],
        chain: fx_invert,
    },
    Entry {
        id: "sharpen",
        params: &[Param {
            key: "amount",
            min: 0.2,
            max: 3.0,
            default: 1.0,
        }],
        chain: fx_sharpen,
    },
    Entry {
        id: "gaussian-blur",
        params: &[Param {
            key: "radius",
            min: 1.0,
            max: 50.0,
            default: 10.0,
        }],
        chain: fx_gaussian_blur,
    },
    Entry {
        id: "box-blur",
        params: &[Param {
            key: "radius",
            min: 1.0,
            max: 30.0,
            default: 6.0,
        }],
        chain: fx_box_blur,
    },
    Entry {
        id: "motion-blur",
        params: &[Param {
            key: "length",
            min: 2.0,
            max: 60.0,
            default: 18.0,
        }],
        chain: fx_motion_blur,
    },
    Entry {
        id: "warm",
        params: &[Param {
            key: "temperature",
            min: 3000.0,
            max: 6000.0,
            default: 4600.0,
        }],
        chain: fx_temperature,
    },
    Entry {
        id: "cool",
        params: &[Param {
            key: "temperature",
            min: 7000.0,
            max: 11000.0,
            default: 8500.0,
        }],
        chain: fx_temperature,
    },
    Entry {
        id: "vibrance",
        params: &[Param {
            key: "intensity",
            min: 0.1,
            max: 2.0,
            default: 0.7,
        }],
        chain: fx_vibrance,
    },
    Entry {
        id: "contrast-pop",
        params: &[Param {
            key: "contrast",
            min: 1.0,
            max: 2.0,
            default: 1.25,
        }],
        chain: fx_contrast_pop,
    },
    Entry {
        id: "vignette",
        params: &[Param {
            key: "strength",
            min: 10.0,
            max: 100.0,
            default: 50.0,
        }],
        chain: fx_vignette,
    },
    Entry {
        id: "film-grain",
        params: &[Param {
            key: "amount",
            min: 2.0,
            max: 40.0,
            default: 12.0,
        }],
        chain: fx_film_grain,
    },
    Entry {
        id: "glow",
        params: &[Param {
            key: "amount",
            min: 10.0,
            max: 100.0,
            default: 45.0,
        }],
        chain: fx_glow,
    },
    Entry {
        id: "posterize",
        params: &[Param {
            key: "levels",
            min: 2.0,
            max: 8.0,
            default: 4.0,
        }],
        chain: fx_posterize,
    },
    Entry {
        id: "pixelate",
        params: &[Param {
            key: "size",
            min: 2.0,
            max: 64.0,
            default: 16.0,
        }],
        chain: fx_pixelate,
    },
    Entry {
        id: "mirror",
        params: &[],
        chain: fx_mirror,
    },
    Entry {
        id: "fisheye",
        params: &[Param {
            key: "strength",
            min: 5.0,
            max: 100.0,
            default: 50.0,
        }],
        chain: fx_fisheye,
    },
    Entry {
        id: "shake",
        params: &[
            Param {
                key: "amount",
                min: 2.0,
                max: 40.0,
                default: 12.0,
            },
            Param {
                key: "speed",
                min: 2.0,
                max: 30.0,
                default: 13.0,
            },
        ],
        chain: fx_shake,
    },
];

// ─── the audio filter catalogue ─────────────────────────────────────────────

/// Pitch shift that moves the formants with the pitch.
///
/// Raising the sample rate and then resampling back shifts everything - which
/// is why it sounds like a different voice rather than the same voice
/// transposed. `atempo` then puts the duration back, because `asetrate` alone
/// would also make the clip shorter.
///
/// This is the technique the whole Voice category is built on.
fn pitch_shift(semitone_shift: f64) -> Vec<String> {
    let ratio = 2f64.powf(semitone_shift / 12.0);
    vec![
        "aresample=48000".to_owned(),
        format!("asetrate={}", fixed(48000.0 * ratio, 6)),
        "aresample=48000".to_owned(),
        format!("atempo={}", fixed(1.0 / ratio, 8)),
    ]
}

fn af_sweet(p: &Params, _index: usize) -> String {
    let strength = p.get("amount").clamp(0.0, 100.0) / 100.0;
    let pitch = p.get("pitch");

    // The coefficients are steeper than a textbook "sweet voice" chain on
    // purpose: at the slider's midpoint the change has to be audible.
    let shift = if pitch > 0.0 {
        pitch
    } else {
        1.2 + 3.8 * strength
    };
    let presence = fixed(0.8 + 5.2 * strength, 3);
    let air = fixed(1.0 + 7.0 * strength, 3);
    let deess = fixed(0.18 + 0.32 * strength, 3);
    let echo = fixed(0.012 + 0.048 * strength, 4);

    let mut parts = pitch_shift(shift);
    parts.extend([
        "highpass=f=85".to_owned(),
        "equalizer=f=300:t=q:w=1.1:g=-1.4".to_owned(),
        format!("equalizer=f=4200:t=q:w=0.8:g={presence}"),
        format!("equalizer=f=10500:t=q:w=0.7:g={air}"),
        "compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1"
            .to_owned(),
        format!("deesser=i={deess}:m=0.45:f=0.55"),
        format!("aecho=0.8:0.82:38:{echo}"),
        "alimiter=limit=0.94:attack=5:release=60".to_owned(),
    ]);
    parts.join(",")
}

fn af_deep(p: &Params, _index: usize) -> String {
    let body = fixed(p.get("body"), 2);
    let mut parts = pitch_shift(p.get("pitch"));
    parts.extend([
        "highpass=f=55".to_owned(),
        format!("equalizer=f=140:t=q:w=1.0:g={body}"),
        "equalizer=f=2600:t=q:w=0.9:g=-1.2".to_owned(),
        "alimiter=limit=0.94:attack=5:release=60".to_owned(),
    ]);
    parts.join(",")
}

fn af_chipmunk(p: &Params, _index: usize) -> String {
    let mut parts = pitch_shift(p.get("pitch"));
    parts.extend([
        "highpass=f=120".to_owned(),
        "alimiter=limit=0.94".to_owned(),
    ]);
    parts.join(",")
}

fn af_robot(p: &Params, _index: usize) -> String {
    // aphaser caps speed at 2, so depth maps into 0.4-2.0 rather than being
    // halved - which put it out of range and made the filter refuse.
    let speed = fixed(0.4 + (p.get("depth") / 10.0) * 1.6, 2);
    [
        format!("aphaser=type=t:speed={speed}:decay=0.6"),
        "flanger=delay=2:depth=4:speed=0.8".to_owned(),
        "equalizer=f=1800:t=q:w=1.2:g=3".to_owned(),
        "alimiter=limit=0.94".to_owned(),
    ]
    .join(",")
}

fn af_bass(p: &Params, _index: usize) -> String {
    let gain = fixed(p.get("gain"), 2);
    format!("bass=g={gain}:f=110:w=0.6,alimiter=limit=0.95")
}

fn af_treble(p: &Params, _index: usize) -> String {
    let gain = fixed(p.get("gain"), 2);
    format!("treble=g={gain}:f=9000:w=0.7,alimiter=limit=0.95")
}

fn af_telephone(p: &Params, _index: usize) -> String {
    let gain = fixed(3.0 + p.get("drive") / 2.0, 2);
    [
        "highpass=f=400".to_owned(),
        "lowpass=f=3400".to_owned(),
        format!("equalizer=f=1600:t=q:w=1.4:g={gain}"),
        "alimiter=limit=0.92".to_owned(),
    ]
    .join(",")
}

fn af_echo(p: &Params, _index: usize) -> String {
    let delay = round(p.get("delay") * 1000.0);
    let decay = fixed(p.get("decay"), 2);
    format!("aecho=0.8:0.85:{delay}:{decay}")
}

fn af_room(p: &Params, _index: usize) -> String {
    let scale = 0.4 + (p.get("size") / 100.0) * 1.6;
    let taps = [23.0, 41.0, 67.0, 97.0]
        .iter()
        .map(|ms| round(ms * scale).to_string())
        .collect::<Vec<_>>()
        .join("|");
    let gains = [0.32, 0.24, 0.17, 0.11]
        .iter()
        .map(|g| fixed(*g, 3))
        .collect::<Vec<_>>()
        .join("|");
    format!("aecho=0.8:0.88:{taps}:{gains}")
}

/// Every audio filter, in the catalogue's order. Same forever-ids rule as
/// the effects.
static FILTERS: &[Entry] = &[
    Entry {
        id: "sweet",
        params: &[
            Param {
                key: "amount",
                min: 0.0,
                max: 100.0,
                default: 65.0,
            },
            // 0 means "follow amount", which is what the original did when
            // no explicit pitch was passed.
            Param {
                key: "pitch",
                min: 0.0,
                max: 8.0,
                default: 0.0,
            },
        ],
        chain: af_sweet,
    },
    Entry {
        id: "deep",
        params: &[
            Param {
                key: "pitch",
                min: -8.0,
                max: -1.0,
                default: -3.0,
            },
            Param {
                key: "body",
                min: 0.0,
                max: 8.0,
                default: 3.0,
            },
        ],
        chain: af_deep,
    },
    Entry {
        id: "chipmunk",
        params: &[Param {
            key: "pitch",
            min: 3.0,
            max: 12.0,
            default: 7.0,
        }],
        chain: af_chipmunk,
    },
    Entry {
        id: "robot",
        params: &[Param {
            key: "depth",
            min: 1.0,
            max: 10.0,
            default: 5.0,
        }],
        chain: af_robot,
    },
    Entry {
        id: "bass",
        params: &[Param {
            key: "gain",
            min: 0.0,
            max: 12.0,
            default: 5.0,
        }],
        chain: af_bass,
    },
    Entry {
        id: "treble",
        params: &[Param {
            key: "gain",
            min: 0.0,
            max: 12.0,
            default: 4.0,
        }],
        chain: af_treble,
    },
    Entry {
        id: "telephone",
        params: &[Param {
            key: "drive",
            min: 0.0,
            max: 10.0,
            default: 3.0,
        }],
        chain: af_telephone,
    },
    Entry {
        id: "echo",
        params: &[
            Param {
                key: "delay",
                min: 0.05,
                max: 1.0,
                default: 0.25,
            },
            Param {
                key: "decay",
                min: 0.1,
                max: 0.9,
                default: 0.4,
            },
        ],
        chain: af_echo,
    },
    Entry {
        id: "room",
        params: &[Param {
            key: "size",
            min: 0.0,
            max: 100.0,
            default: 40.0,
        }],
        chain: af_room,
    },
];

// ─── composition ────────────────────────────────────────────────────────────

/// Fills in any parameter the clip did not set, and drops any stray key the
/// entry does not declare.
fn resolve(entry: &Entry, set: &BTreeMap<String, f64>) -> Params {
    Params(
        entry
            .params
            .iter()
            .map(|param| {
                (
                    param.key,
                    set.get(param.key).copied().unwrap_or(param.default),
                )
            })
            .collect(),
    )
}

/// The shared composition rule: enabled entries in applied order, comma-
/// joined; bypassed entries and ids the catalogue does not know contribute
/// nothing. Empty result is the empty string.
fn compose(catalogue: &[Entry], applied: &[AppliedFilter]) -> String {
    // The index passed to each builder is the fragment's *emitted* position,
    // not its list position - labels stay stable when a bypassed entry sits
    // earlier in the list. Mirrors buildEffectChain exactly.
    let mut fragments: Vec<String> = Vec::new();
    for applied in applied.iter().filter(|applied| applied.enabled) {
        let Some(entry) = catalogue.iter().find(|entry| entry.id == applied.id) else {
            continue;
        };
        let fragment = (entry.chain)(&resolve(entry, &applied.params), fragments.len());
        fragments.push(fragment);
    }
    fragments.join(",")
}

/// The complete FFmpeg *video* filter string for a clip's effects, or the
/// empty string if it has none. Effects apply in the order they were added.
pub fn video_effect_chain(effects: &[AppliedFilter]) -> String {
    compose(EFFECTS, effects)
}

/// The complete FFmpeg *audio* filter string for a clip's filters, or the
/// empty string if it has none. Filters apply in the order they were added:
/// EQ before a limiter is a different sound from the reverse.
pub fn audio_filter_chain(filters: &[AppliedFilter]) -> String {
    compose(FILTERS, filters)
}

// ─── tests: the pinned strings ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(id: &str, params: &[(&str, f64)]) -> AppliedFilter {
        AppliedFilter {
            id: id.to_owned(),
            params: params
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect(),
            enabled: true,
        }
    }

    fn one_effect(id: &str, params: &[(&str, f64)]) -> String {
        video_effect_chain(&[applied(id, params)])
    }

    fn one_filter(id: &str, params: &[(&str, f64)]) -> String {
        audio_filter_chain(&[applied(id, params)])
    }

    /// The definition's sliders, all pushed to one bound.
    fn at_bound(catalogue: &[Entry], id: &str, minimum: bool) -> AppliedFilter {
        let entry = catalogue
            .iter()
            .find(|entry| entry.id == id)
            .expect("known id");
        AppliedFilter {
            id: id.to_owned(),
            params: entry
                .params
                .iter()
                .map(|param| {
                    (
                        param.key.to_owned(),
                        if minimum { param.min } else { param.max },
                    )
                })
                .collect(),
            enabled: true,
        }
    }

    /// Asserts a fixture table covers exactly the given ids - a new
    /// catalogue entry must pin its strings here.
    fn assert_covers(table: &[(&str, &str)], mut ids: Vec<&str>, which: &str) {
        let mut covered: Vec<&str> = table.iter().map(|(id, _)| *id).collect();
        covered.sort_unstable();
        ids.sort_unstable();
        assert_eq!(
            covered, ids,
            "the {which} fixture table must cover the catalogue"
        );
    }

    fn parameterised(catalogue: &[Entry]) -> Vec<&'static str> {
        catalogue
            .iter()
            .filter(|entry| !entry.params.is_empty())
            .map(|entry| entry.id)
            .collect()
    }

    // The expected strings below pin the exporter's bytes.

    const EFFECT_DEFAULTS: &[(&str, &str)] = &[
        ("black-white", "hue=s=0"),
        (
            "sepia",
            "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131",
        ),
        ("invert", "negate"),
        ("sharpen", "unsharp=5:5:1.00:5:5:0"),
        ("gaussian-blur", "gblur=sigma=10.0"),
        ("box-blur", "boxblur=6:1"),
        ("motion-blur", "gblur=sigma=18.0:sigmaV=0.1"),
        ("warm", "colortemperature=temperature=4600"),
        ("cool", "colortemperature=temperature=8500"),
        ("vibrance", "vibrance=intensity=0.70"),
        ("contrast-pop", "eq=contrast=1.25"),
        ("vignette", "vignette=angle=0.775"),
        ("film-grain", "noise=alls=12:allf=t+u"),
        (
            "glow",
            "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];\
             [glowa0][glowg0]blend=all_mode=screen:all_opacity=0.45",
        ),
        (
            "posterize",
            "lutrgb=r=trunc(val/64)*64:g=trunc(val/64)*64:b=trunc(val/64)*64",
        ),
        ("pixelate", "pixelize=width=16:height=16"),
        (
            "mirror",
            "crop=iw/2:ih:0:0,split[mirl0][mirr0];[mirr0]hflip[mirf0];[mirl0][mirf0]hstack",
        ),
        ("fisheye", "lenscorrection=k1=-0.275:k2=-0.100:i=bilinear"),
        ("shake", "crop=iw-24:ih-24:12+12*sin(t*13):12+12*cos(t*17)"),
    ];

    const EFFECT_MINIMA: &[(&str, &str)] = &[
        ("sharpen", "unsharp=5:5:0.20:5:5:0"),
        ("gaussian-blur", "gblur=sigma=1.0"),
        ("box-blur", "boxblur=1:1"),
        ("motion-blur", "gblur=sigma=2.0:sigmaV=0.1"),
        ("warm", "colortemperature=temperature=3000"),
        ("cool", "colortemperature=temperature=7000"),
        ("vibrance", "vibrance=intensity=0.10"),
        ("contrast-pop", "eq=contrast=1.00"),
        ("vignette", "vignette=angle=0.355"),
        ("film-grain", "noise=alls=2:allf=t+u"),
        (
            "glow",
            "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];\
             [glowa0][glowg0]blend=all_mode=screen:all_opacity=0.10",
        ),
        (
            "posterize",
            "lutrgb=r=trunc(val/128)*128:g=trunc(val/128)*128:b=trunc(val/128)*128",
        ),
        ("pixelate", "pixelize=width=2:height=2"),
        ("fisheye", "lenscorrection=k1=-0.028:k2=-0.010:i=bilinear"),
        ("shake", "crop=iw-4:ih-4:2+2*sin(t*2):2+2*cos(t*3)"),
    ];

    const EFFECT_MAXIMA: &[(&str, &str)] = &[
        ("sharpen", "unsharp=5:5:3.00:5:5:0"),
        ("gaussian-blur", "gblur=sigma=50.0"),
        ("box-blur", "boxblur=30:1"),
        ("motion-blur", "gblur=sigma=60.0:sigmaV=0.1"),
        ("warm", "colortemperature=temperature=6000"),
        ("cool", "colortemperature=temperature=11000"),
        ("vibrance", "vibrance=intensity=2.00"),
        ("contrast-pop", "eq=contrast=2.00"),
        ("vignette", "vignette=angle=1.300"),
        ("film-grain", "noise=alls=40:allf=t+u"),
        (
            "glow",
            "split[glowa0][glowb0];[glowb0]gblur=sigma=18[glowg0];\
             [glowa0][glowg0]blend=all_mode=screen:all_opacity=1.00",
        ),
        (
            "posterize",
            "lutrgb=r=trunc(val/32)*32:g=trunc(val/32)*32:b=trunc(val/32)*32",
        ),
        ("pixelate", "pixelize=width=64:height=64"),
        ("fisheye", "lenscorrection=k1=-0.550:k2=-0.200:i=bilinear"),
        ("shake", "crop=iw-80:ih-80:40+40*sin(t*30):40+40*cos(t*39)"),
    ];

    const FILTER_DEFAULTS: &[(&str, &str)] = &[
        (
            "sweet",
            "aresample=48000,asetrate=59334.357554,aresample=48000,atempo=0.80897480,\
             highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4,\
             equalizer=f=4200:t=q:w=0.8:g=4.180,equalizer=f=10500:t=q:w=0.7:g=5.550,\
             compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1,\
             deesser=i=0.388:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0432,\
             alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            "deep",
            "aresample=48000,asetrate=40363.027932,aresample=48000,atempo=1.18920712,\
             highpass=f=55,equalizer=f=140:t=q:w=1.0:g=3.00,\
             equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            "chipmunk",
            "aresample=48000,asetrate=71918.739690,aresample=48000,atempo=0.66741993,\
             highpass=f=120,alimiter=limit=0.94",
        ),
        (
            "robot",
            "aphaser=type=t:speed=1.20:decay=0.6,flanger=delay=2:depth=4:speed=0.8,\
             equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
        ),
        ("bass", "bass=g=5.00:f=110:w=0.6,alimiter=limit=0.95"),
        ("treble", "treble=g=4.00:f=9000:w=0.7,alimiter=limit=0.95"),
        (
            "telephone",
            "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=4.50,alimiter=limit=0.92",
        ),
        ("echo", "aecho=0.8:0.85:250:0.40"),
        (
            "room",
            "aecho=0.8:0.88:24|43|70|101:0.320|0.240|0.170|0.110",
        ),
    ];

    const FILTER_MINIMA: &[(&str, &str)] = &[
        (
            "sweet",
            "aresample=48000,asetrate=51445.126202,aresample=48000,atempo=0.93303299,\
             highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4,\
             equalizer=f=4200:t=q:w=0.8:g=0.800,equalizer=f=10500:t=q:w=0.7:g=1.000,\
             compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1,\
             deesser=i=0.180:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0120,\
             alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            "deep",
            "aresample=48000,asetrate=30238.105197,aresample=48000,atempo=1.58740105,\
             highpass=f=55,equalizer=f=140:t=q:w=1.0:g=0.00,\
             equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            "chipmunk",
            "aresample=48000,asetrate=57081.941520,aresample=48000,atempo=0.84089642,\
             highpass=f=120,alimiter=limit=0.94",
        ),
        (
            "robot",
            "aphaser=type=t:speed=0.56:decay=0.6,flanger=delay=2:depth=4:speed=0.8,\
             equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
        ),
        ("bass", "bass=g=0.00:f=110:w=0.6,alimiter=limit=0.95"),
        ("treble", "treble=g=0.00:f=9000:w=0.7,alimiter=limit=0.95"),
        (
            "telephone",
            "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=3.00,alimiter=limit=0.92",
        ),
        ("echo", "aecho=0.8:0.85:50:0.10"),
        ("room", "aecho=0.8:0.88:9|16|27|39:0.320|0.240|0.170|0.110"),
    ];

    const FILTER_MAXIMA: &[(&str, &str)] = &[
        (
            "sweet",
            "aresample=48000,asetrate=76195.250494,aresample=48000,atempo=0.62996052,\
             highpass=f=85,equalizer=f=300:t=q:w=1.1:g=-1.4,\
             equalizer=f=4200:t=q:w=0.8:g=6.000,equalizer=f=10500:t=q:w=0.7:g=8.000,\
             compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1,\
             deesser=i=0.500:m=0.45:f=0.55,aecho=0.8:0.82:38:0.0600,\
             alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            "deep",
            "aresample=48000,asetrate=45305.967009,aresample=48000,atempo=1.05946309,\
             highpass=f=55,equalizer=f=140:t=q:w=1.0:g=8.00,\
             equalizer=f=2600:t=q:w=0.9:g=-1.2,alimiter=limit=0.94:attack=5:release=60",
        ),
        (
            // +12 semitones is exactly double: the one point the maths comes
            // out round.
            "chipmunk",
            "aresample=48000,asetrate=96000.000000,aresample=48000,atempo=0.50000000,\
             highpass=f=120,alimiter=limit=0.94",
        ),
        (
            // Depth 10 must land exactly on aphaser's speed cap of 2, never
            // above it - above the cap the filter refuses the whole export.
            "robot",
            "aphaser=type=t:speed=2.00:decay=0.6,flanger=delay=2:depth=4:speed=0.8,\
             equalizer=f=1800:t=q:w=1.2:g=3,alimiter=limit=0.94",
        ),
        ("bass", "bass=g=12.00:f=110:w=0.6,alimiter=limit=0.95"),
        ("treble", "treble=g=12.00:f=9000:w=0.7,alimiter=limit=0.95"),
        (
            "telephone",
            "highpass=f=400,lowpass=f=3400,equalizer=f=1600:t=q:w=1.4:g=8.00,alimiter=limit=0.92",
        ),
        ("echo", "aecho=0.8:0.85:1000:0.90"),
        (
            "room",
            "aecho=0.8:0.88:46|82|134|194:0.320|0.240|0.170|0.110",
        ),
    ];

    #[test]
    fn every_effect_at_its_default_settings() {
        let ids: Vec<&str> = EFFECTS.iter().map(|entry| entry.id).collect();
        assert_covers(EFFECT_DEFAULTS, ids, "effect defaults");
        for (id, expected) in EFFECT_DEFAULTS {
            // No params passed: resolve must fill every default.
            assert_eq!(one_effect(id, &[]), *expected, "{id}");
        }
    }

    #[test]
    fn every_effect_at_its_sliders_minimum() {
        assert_covers(EFFECT_MINIMA, parameterised(EFFECTS), "effect minima");
        for (id, expected) in EFFECT_MINIMA {
            let chain = video_effect_chain(&[at_bound(EFFECTS, id, true)]);
            assert_eq!(chain, *expected, "{id}");
        }
    }

    #[test]
    fn every_effect_at_its_sliders_maximum() {
        assert_covers(EFFECT_MAXIMA, parameterised(EFFECTS), "effect maxima");
        for (id, expected) in EFFECT_MAXIMA {
            let chain = video_effect_chain(&[at_bound(EFFECTS, id, false)]);
            assert_eq!(chain, *expected, "{id}");
        }
    }

    #[test]
    fn every_filter_at_its_default_settings() {
        let ids: Vec<&str> = FILTERS.iter().map(|entry| entry.id).collect();
        assert_covers(FILTER_DEFAULTS, ids, "filter defaults");
        for (id, expected) in FILTER_DEFAULTS {
            assert_eq!(one_filter(id, &[]), *expected, "{id}");
        }
    }

    #[test]
    fn every_filter_at_its_sliders_minimum() {
        assert_covers(FILTER_MINIMA, parameterised(FILTERS), "filter minima");
        for (id, expected) in FILTER_MINIMA {
            let chain = audio_filter_chain(&[at_bound(FILTERS, id, true)]);
            assert_eq!(chain, *expected, "{id}");
        }
    }

    #[test]
    fn every_filter_at_its_sliders_maximum() {
        assert_covers(FILTER_MAXIMA, parameterised(FILTERS), "filter maxima");
        for (id, expected) in FILTER_MAXIMA {
            let chain = audio_filter_chain(&[at_bound(FILTERS, id, false)]);
            assert_eq!(chain, *expected, "{id}");
        }
    }

    #[test]
    fn stacked_effects_join_with_commas_in_applied_order() {
        assert_eq!(
            video_effect_chain(&[applied("gaussian-blur", &[]), applied("black-white", &[])]),
            "gblur=sigma=10.0,hue=s=0"
        );
        // The reverse order is a different picture and a different string.
        assert_eq!(
            video_effect_chain(&[applied("black-white", &[]), applied("gaussian-blur", &[])]),
            "hue=s=0,gblur=sigma=10.0"
        );
    }

    #[test]
    fn a_bypassed_effect_contributes_nothing() {
        let mut sepia = applied("sepia", &[]);
        sepia.enabled = false;
        assert_eq!(
            video_effect_chain(&[applied("invert", &[]), sepia, applied("black-white", &[])]),
            "negate,hue=s=0"
        );
    }

    #[test]
    fn stacking_one_labelled_effect_twice_keeps_its_graph_labels_distinct() {
        // Labels embed the emitted index. With fixed labels, two glows on
        // one clip would duplicate [glowa] in a single filtergraph and
        // FFmpeg would reject it - the whole export would fail.
        let chain = video_effect_chain(&[applied("glow", &[]), applied("glow", &[])]);
        assert!(chain.contains("[glowa0]"), "was: {chain}");
        assert!(chain.contains("[glowa1]"), "was: {chain}");

        // A bypassed entry does not consume an index: labels follow emission.
        let mut sepia = applied("sepia", &[]);
        sepia.enabled = false;
        let skipped = video_effect_chain(&[sepia, applied("mirror", &[])]);
        assert!(skipped.contains("[mirl0]"), "was: {skipped}");
    }

    #[test]
    fn an_unknown_effect_id_is_skipped_not_exported_as_garbage() {
        // A project written by a newer Concat may carry effects this build
        // does not know. The chain must stay valid for the ones it does.
        assert_eq!(
            video_effect_chain(&[applied("from-the-future", &[]), applied("invert", &[])]),
            "negate"
        );
    }

    #[test]
    fn no_effects_means_the_empty_string() {
        // The spelling of "no chain" is "".
        assert_eq!(video_effect_chain(&[]), "");
        let mut off = applied("invert", &[]);
        off.enabled = false;
        assert_eq!(video_effect_chain(&[off]), "");
    }

    #[test]
    fn stray_parameter_keys_are_dropped_and_missing_ones_default() {
        assert_eq!(
            one_effect("sharpen", &[("amount", 2.0), ("bogus", 99.0)]),
            "unsharp=5:5:2.00:5:5:0"
        );
        assert_eq!(
            one_effect("shake", &[("amount", 20.0)]),
            "crop=iw-40:ih-40:20+20*sin(t*13):20+20*cos(t*17)"
        );
    }

    #[test]
    fn stacked_filters_join_with_commas_in_applied_order() {
        // EQ into a limiter is a different sound from a limiter into EQ; the
        // array's order is the contract.
        assert_eq!(
            audio_filter_chain(&[applied("bass", &[]), applied("echo", &[])]),
            "bass=g=5.00:f=110:w=0.6,alimiter=limit=0.95,aecho=0.8:0.85:250:0.40"
        );
    }

    #[test]
    fn a_bypassed_filter_contributes_nothing() {
        let mut bass = applied("bass", &[]);
        bass.enabled = false;
        assert_eq!(
            audio_filter_chain(&[bass, applied("echo", &[])]),
            "aecho=0.8:0.85:250:0.40"
        );
    }

    #[test]
    fn an_unknown_filter_id_is_skipped_not_exported_as_garbage() {
        assert_eq!(
            audio_filter_chain(&[applied("from-the-future", &[]), applied("echo", &[])]),
            "aecho=0.8:0.85:250:0.40"
        );
    }

    #[test]
    fn no_audible_filters_means_the_empty_string() {
        assert_eq!(audio_filter_chain(&[]), "");
        let mut off = applied("bass", &[]);
        off.enabled = false;
        assert_eq!(audio_filter_chain(&[off]), "");
    }

    #[test]
    fn a_set_parameter_overrides_the_default() {
        // One filter with one slider moved off its default.
        assert_eq!(
            one_filter("echo", &[("delay", 0.5)]),
            "aecho=0.8:0.85:500:0.40"
        );
    }
}
