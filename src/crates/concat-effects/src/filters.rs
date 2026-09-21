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
//! and sound filters an author is likely to reach for next, sound and
//! picture in one alphabet, since the search is binary and needs the
//! whole list in order. Adding one is adding a line in its place; the
//! test keeps the order.

/// Every filter a chain may use, in one sorted run for [`allowed`]'s search.
pub const ALLOWED: &[&str] = &[
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
    "avgblur",
    "bandpass",
    "bandreject",
    "bass",
    "bilateral",
    "biquad",
    "blend",
    "boxblur",
    "bs2b",
    "bwdif",
    "cas",
    "chorus",
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
    "compand",
    "compensationdelay",
    "convolution",
    "crop",
    "crossfeed",
    "crystalizer",
    "curves",
    "dcshift",
    "dctdnoiz",
    "deband",
    "deblock",
    "deesser",
    "deflicker",
    "dilation",
    "drawbox",
    "drawgrid",
    "dynaudnorm",
    "earwax",
    "edgedetect",
    "eq",
    "equalizer",
    "erosion",
    "exposure",
    "extrastereo",
    "fade",
    "fftdnoiz",
    "fillborders",
    "firequalizer",
    "flanger",
    "format",
    "gblur",
    "geq",
    "gradfun",
    "haas",
    "hflip",
    "highpass",
    "highshelf",
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
    "loudnorm",
    "lowpass",
    "lowshelf",
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
    "pan",
    "perspective",
    "photosensitivity",
    "pixelize",
    "prewitt",
    "pseudocolor",
    "rgbashift",
    "roberts",
    "rotate",
    "rubberband",
    "sab",
    "scale",
    "selectivecolor",
    "shear",
    "shuffleplanes",
    "smartblur",
    "sobel",
    "speechnorm",
    "split",
    "stereotools",
    "stereowiden",
    "superequalizer",
    "surround",
    "swapuv",
    "tblend",
    "tile",
    "tmix",
    "tonemap",
    "transpose",
    "treble",
    "tremolo",
    "unsharp",
    "vaguedenoiser",
    "vflip",
    "vibrance",
    "vibrato",
    "vignette",
    "virtualbass",
    "volume",
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
