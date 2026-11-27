// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The bin: each file and what its probe found.

use std::collections::{BTreeSet, HashMap};

use concat_doc::model::{
    BeatMarks, ColorRange, ColorSpace, Media, MediaId, MediaKind, MediaOrigin, Size, SoundStream,
};
use concat_doc::{Rate, TickLen};
use serde_json::Value;

use crate::ImportError;
use crate::build::Build;
use crate::read::{Fields, Kept, tick};

/// A frame rate as the earlier file wrote it: an exact fraction, or a
/// decimal. Either way a spelling of a slowed rate - 23.976, 29.97, 59.94
/// - becomes its exact fraction over 1001, as [`Rate::from_decimal`] has
///   it; any other fraction is taken at its word.
pub(crate) fn rate(num: u64, den: u64) -> Option<Rate> {
    let (num, den) = (u32::try_from(num).ok()?, u32::try_from(den).ok()?);
    let exact = Rate::new(num, den).ok()?;
    if den == 1 || den == 1001 {
        return Some(exact);
    }
    match Rate::from_decimal(f64::from(num) / f64::from(den)) {
        Ok(slowed) if slowed.den() == 1001 => Some(slowed),
        _ => Some(exact),
    }
}

fn rate_of_fraction(text: &str) -> Option<Rate> {
    let (num, den) = text.trim().split_once('/')?;
    rate(num.trim().parse().ok()?, den.trim().parse().ok()?)
}

/// The colour a picture is in, as the file names it.
pub(crate) fn color_space(build: &mut Build, name: Option<&str>) -> ColorSpace {
    match name {
        None | Some("sdr") => ColorSpace::Sdr,
        Some("hlg") => ColorSpace::Hlg,
        Some("pq") => ColorSpace::Pq,
        Some(other) => ColorSpace::Unknown(build.raw(&Value::String(other.to_owned()))),
    }
}

/// Every file of the bin, each under a new id, and the old ids that name
/// them.
pub(crate) fn bin(
    build: &mut Build,
    list: &[Value],
) -> Result<HashMap<String, MediaId>, ImportError> {
    let mut ids: HashMap<String, MediaId> = HashMap::with_capacity(list.len());
    for value in list {
        let mut f = Fields::of(value);
        let old = f.string_or("id", "");
        let name = f.string_or("name", "");
        let place = format!("file \"{name}\" ({old})");
        let mut kept = Kept::default();
        let media = one(build, &place, &mut f, name, &mut kept);
        kept.rest("", &f);
        if !kept.is_empty() {
            let names = kept.names().join(", ");
            build.note(
                &place,
                format!("kept {names}, which this version has no place for"),
            );
        }
        let media = Media {
            residual: kept.residual(&mut build.words),
            ..media
        };
        let id: MediaId = build.mint()?;
        build
            .model
            .media
            .insert(id, media)
            .map_err(|error| ImportError::Rejected(error.to_string()))?;
        match ids.entry(old) {
            std::collections::hash_map::Entry::Occupied(_) => build.note(
                &place,
                "its id is another file's too; the clips that name it play the first",
            ),
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert(id);
            }
        }
    }
    Ok(ids)
}

fn one(build: &mut Build, place: &str, f: &mut Fields<'_>, name: String, kept: &mut Kept) -> Media {
    let path = f.string_or("path", "");
    let kind = match f.str("kind") {
        None | Some("video") => MediaKind::Video,
        Some("audio") => MediaKind::Audio,
        Some("image") => MediaKind::Image,
        Some(other) => MediaKind::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    // A still has no length of its own.
    let length = match f.f64("duration") {
        Some(seconds) if kind != MediaKind::Image => {
            tick(seconds).and_then(|t| TickLen::new(t.get()))
        }
        _ => None,
    };
    let rate = match f.str("frameRateFraction").and_then(rate_of_fraction) {
        Some(rate) => {
            f.skip(&["frameRate"]);
            Some(rate)
        }
        None => f
            .f64("frameRate")
            .and_then(|fps| Rate::from_decimal(fps).ok()),
    };
    let size = match (f.u32("width"), f.u32("height")) {
        (Some(width), Some(height)) => Size::new(width, height),
        _ => None,
    };
    let mut has_sound = f.bool_or("hasAudio", false);
    let mut streams: Vec<SoundStream> = Vec::new();
    let mut seen = BTreeSet::new();
    for (at, stream) in f.list("audioTracks").iter().enumerate() {
        let mut s = Fields::of(stream);
        let Some(index) = s.u32("index") else {
            kept.keep("audioTracks", &at.to_string(), stream.clone());
            build.note(place, "a sound stream with no index is kept as it was");
            continue;
        };
        if !seen.insert(index) {
            kept.keep("audioTracks", &at.to_string(), stream.clone());
            build.note(
                place,
                format!("its sound stream {index} is listed twice; the first is used"),
            );
            continue;
        }
        streams.push(SoundStream {
            index,
            channels: s.u32("channels").unwrap_or(0),
            sample_rate: s.u32("sampleRate").unwrap_or(0),
            codec: s.string_or("codec", ""),
            language: s.string_or("language", ""),
            title: s.string_or("title", ""),
        });
        kept.rest(&format!("audioTracks/{at}"), &s);
    }
    if !has_sound && !streams.is_empty() {
        // A file whose probe listed its sound has sound.
        has_sound = true;
    }
    let video_codec = f.str("videoCodec").map(str::to_owned);
    let audio_codec = f.str("audioCodec").map(str::to_owned);
    let color = color_space(build, f.str("colorSpace"));
    let range = f.str("colorRange").map(|range| match range {
        "limited" => ColorRange::Limited,
        "full" => ColorRange::Full,
        other => ColorRange::Unknown(build.raw(&Value::String(other.to_owned()))),
    });
    let beats = f.str("beats").map(|beats| match beats {
        "zoom" => BeatMarks::Zoom,
        "every" => BeatMarks::Every,
        "second" => BeatMarks::Second,
        "bar" => BeatMarks::Bar,
        other => BeatMarks::Unknown(build.raw(&Value::String(other.to_owned()))),
    });
    let origin = f.str("origin").map(|origin| match origin {
        "speech" => MediaOrigin::Speech,
        "processed" => MediaOrigin::Processed,
        other => MediaOrigin::Unknown(build.raw(&Value::String(other.to_owned()))),
    });
    let placeholder = f.bool_or("placeholder", false);
    Media {
        path,
        name,
        kind,
        length,
        rate,
        size,
        has_sound,
        streams,
        video_codec,
        audio_codec,
        color,
        range,
        beats,
        origin,
        placeholder,
        residual: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slowed_rate_is_its_exact_fraction_however_it_was_written() {
        let ntsc = Rate::new(30_000, 1001).ok();
        assert_eq!(rate(30_000, 1001), ntsc);
        assert_eq!(rate(2997, 100), ntsc);
        assert_eq!(rate(23_976, 1000), Rate::new(24_000, 1001).ok());
        assert_eq!(rate(30, 1), Rate::new(30, 1).ok());
        assert_eq!(rate(25, 2), Rate::new(25, 2).ok());
        assert_eq!(rate(0, 1), None);
        assert_eq!(rate_of_fraction("30000/1001"), ntsc);
        assert_eq!(Rate::from_decimal(29.97).ok(), ntsc);
    }
}
