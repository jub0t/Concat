// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The FFmpeg filters a package's chain may name.
//!
//! A chain runs inside the decoder's filtergraph with the process's own
//! rights, and FFmpeg has filters that reach past the frame: `movie` opens
//! any file, `drawtext` reads a text file and a font file, `frei0r`,
//! `ladspa` and `lv2` load plugins, `sendcmd` and `zmq` take commands,
//! `vidstabdetect` and `signature` write files, `arnndn` loads a model. A
//! package is a folder anyone can share, so its chain is held to this
//! list - filters that read the frame and write the frame - and one that
//! names anything else is refused at load, before it is ever run.
//!
//! The list is the union of what the built-ins use and the plain pixel
//! and sound filters an author is likely to reach for next. Adding one is
//! adding a line, in order; the test keeps the order.

/// Every filter a chain may use, sorted for [`allowed`]'s search.
pub const ALLOWED: &[&str] = &[
    // ── sound ──
    "acompressor",
    "acontrast",
    "acrusher",
    "adeclick",
    "adeclip",
    "adelay",
    "adenorm",
    "aecho",
    "aemphasis",
    "aexciter",
    "afade",
    "afftdn",
    "aformat",
    "afreqshift",
    "agate",
    "alimiter",
    "allpass",
    "anlmdn",
    "apad",
    "aphaser",
    "aphaseshift",
    "apulsator",
    "aresample",
    "asetrate",
    "asoftclip",
    "asubboost",
    "asubcut",
    "asupercut",
    "atempo",
    "atilt",
    "bandpass",
    "bandreject",
    "bass",
    "biquad",
    "bs2b",
    "chorus",
    "compand",
    "compensationdelay",
    "crossfeed",
    "crystalizer",
    "dcshift",
    "deesser",
    "dynaudnorm",
    "earwax",
    "equalizer",
    "extrastereo",
    "firequalizer",
    "flanger",
    "haas",
    "highpass",
    "highshelf",
    "loudnorm",
    "lowpass",
    "lowshelf",
    "pan",
    "rubberband",
    "speechnorm",
    "stereotools",
    "stereowiden",
    "superequalizer",
    "surround",
    "treble",
    "tremolo",
    "vibrato",
    "virtualbass",
    "volume",
    // ── picture ──
    "avgblur",
    "bilateral",
    "blend",
    "boxblur",
    "bwdif",
    "cas",
    "chromahold",
    "chromakey",
    "chromanr",
    "chromashift",
    "colorbalance",
    "colorchannelmixer",
    "colorcontrast",
    "colorcorrect",
    "colorhold",
    "colorize",
    "colorkey",
    "colorlevels",
    "colorspace",
    "colortemperature",
    "convolution",
    "crop",
    "curves",
    "dctdnoiz",
    "deband",
    "deblock",
    "deflicker",
    "dilation",
    "drawbox",
    "drawgrid",
    "edgedetect",
    "eq",
    "erosion",
    "exposure",
    "fade",
    "fftdnoiz",
    "fillborders",
    "format",
    "gblur",
    "geq",
    "gradfun",
    "hflip",
    "histeq",
    "hqdn3d",
    "hqx",
    "hstack",
    "hsvhold",
    "hsvkey",
    "hue",
    "huesaturation",
    "lagfun",
    "lenscorrection",
    "limiter",
    "lumakey",
    "lut",
    "lut1d",
    "lut3d",
    "lutrgb",
    "lutyuv",
    "median",
    "monochrome",
    "negate",
    "nlmeans",
    "noise",
    "normalize",
    "overlay",
    "pad",
    "perspective",
    "photosensitivity",
    "pixelize",
    "prewitt",
    "pseudocolor",
    "rgbashift",
    "roberts",
    "rotate",
    "sab",
    "scale",
    "selectivecolor",
    "shear",
    "shuffleplanes",
    "smartblur",
    "sobel",
    "split",
    "swapuv",
    "tblend",
    "tile",
    "tmix",
    "tonemap",
    "transpose",
    "unsharp",
    "vaguedenoiser",
    "vflip",
    "vibrance",
    "vignette",
    "vstack",
    "xbr",
    "zoompan",
];

/// Whether a chain may name this filter.
pub fn allowed(name: &str) -> bool {
    ALLOWED.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_free_of_repeats() {
        for pair in ALLOWED.windows(2) {
            assert!(pair[0] < pair[1], "`{}` before `{}`", pair[0], pair[1]);
        }
    }

    #[test]
    fn the_frame_is_the_only_thing_a_filter_may_touch() {
        for name in ["hue", "eq", "lut3d", "split", "blend", "volume", "atempo"] {
            assert!(allowed(name), "{name}");
        }
        for name in [
            "movie",
            "amovie",
            "drawtext",
            "frei0r",
            "ladspa",
            "lv2",
            "sendcmd",
            "asendcmd",
            "zmq",
            "azmq",
            "vidstabdetect",
            "vidstabtransform",
            "signature",
            "metadata",
            "ametadata",
            "arnndn",
            "subtitles",
            "ass",
            "ocr",
            "sr",
            "dnn_processing",
            "",
            "Hue",
            "hue ",
        ] {
            assert!(!allowed(name), "{name}");
        }
    }
}
