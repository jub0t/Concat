// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Adopting a CapCut draft folder as a Concat project.
//!
//! A CapCut draft is a folder holding `draft_content.json`; a Concat project
//! is a folder holding `concat.json`. [`adopt`] is called when a folder is
//! opened: a folder with a draft and no Concat manifest is converted, once,
//! by writing `concat.json` beside the draft. No CapCut file is read for
//! anything but its content, and none is changed or removed, so the draft
//! still opens in CapCut.
//!
//! The conversion builds a [`Project`] from the engine's own types and
//! writes it with `to_document`, then reads the document back through
//! `from_document` - the load a session does - and refuses to leave a file
//! behind that would not open. The mapping rules are in `CAPCUT_IMPORT.md`;
//! `scripts/capcut_to_concat.py` follows the same rules and is the oracle
//! this module's tests were written from.
//!
//! What is carried: media, video / image / audio / text segments, timing,
//! speed, transform, opacity, flips, the Blur effect, and transitions (as a
//! dissolve of the same length). What is not: keyframes, masks, filters,
//! colour adjustment, animations, stickers, audio fades. Each thing skipped
//! is named in the log.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use concat_project::model::ranges::{
    MAX_OFFSET, MAX_SCALE, MAX_SPEED, MAX_STRETCH, MIN_CLIP_DURATION, MIN_SCALE, MIN_SPEED,
    MIN_STRETCH, MIN_TRANSITION, wrap_rotation,
};
use concat_project::model::{
    AppliedFilter, Clip, ClipKind, MediaItem, MediaKind, Project, TextStyle, Timeline, Track,
    Transition, VideoSettings,
};
use concat_project::placement::TOUCH;
use concat_project::{DocumentSettings, from_document, to_document};
use serde_json::Value;

/// The file that makes a folder a CapCut draft.
pub const DRAFT_CONTENT: &str = "draft_content.json";
const DRAFT_META: &str = "draft_meta_info.json";
const MANIFEST: &str = "concat.json";

/// CapCut writes times in microseconds; Concat's are seconds.
const US: f64 = 1_000_000.0;
/// A CapCut transition Concat has no package for becomes a dissolve of the
/// same length. Timing is what matters to the cut; the look is approximate.
const FALLBACK_TRANSITION: &str = "concat.dissolve";
/// CapCut's text size unit is unverified. 15 is CapCut's default title size
/// and 0.09 of the frame height is Concat's, so sizes scale linearly around
/// that pair.
const TEXT_SIZE_UNIT: f64 = 0.09 / 15.0;

/// What a conversion made, and what it had to leave behind.
#[derive(Debug)]
pub struct Converted {
    /// The `concat.json` document.
    pub document: Value,
    /// One line for everything skipped, clamped or approximated.
    pub warnings: Vec<String>,
    /// Items in the media bin.
    pub media: usize,
    /// Lanes on the timeline.
    pub lanes: usize,
    /// Clips on the timeline.
    pub clips: usize,
}

/// Whether `root` holds a CapCut draft.
pub fn is_draft(root: &Path) -> bool {
    root.join(DRAFT_CONTENT).is_file()
}

/// Converts the draft in `root` into a Concat project in place, when `root`
/// is one: a draft and no Concat manifest. `Ok(None)` for any other folder,
/// which is the common case and costs two file checks.
pub fn adopt(root: &Path) -> Result<Option<Converted>, String> {
    if root.join(MANIFEST).is_file() || !is_draft(root) {
        return Ok(None);
    }
    log::info!("detect capcut project try to convert...");
    let converted = convert_folder(root)?;
    log::info!(
        "capcut project converted: {} media, {} lane(s), {} clip(s), {} warning(s) -> {}",
        converted.media,
        converted.lanes,
        converted.clips,
        converted.warnings.len(),
        root.join(MANIFEST).display()
    );
    Ok(Some(converted))
}

/// Reads the draft in `root`, converts it, and writes `concat.json` beside
/// it. Refuses a folder that already has one.
pub fn convert_folder(root: &Path) -> Result<Converted, String> {
    let draft_path = root.join(DRAFT_CONTENT);
    let bytes = std::fs::read(&draft_path)
        .map_err(|error| format!("could not read {}: {error}", draft_path.display()))?;
    // Some CapCut builds encrypt the draft; that is not readable from here.
    let draft: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{} is not readable JSON ({error}); it may be encrypted by this CapCut version",
            draft_path.display()
        )
    })?;
    let meta: Option<Value> = std::fs::read(root.join(DRAFT_META))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let fallback = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "CapCut project".to_owned());

    let converted = convert(&draft, meta.as_ref(), &fallback)?;

    let target = root.join(MANIFEST);
    let temporary = root.join(".concat-capcut.tmp");
    let encoded = serde_json::to_vec_pretty(&converted.document)
        .map_err(|error| format!("could not encode the project: {error}"))?;
    std::fs::write(&temporary, encoded)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    if target.exists() {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{} already exists", target.display()));
    }
    std::fs::rename(&temporary, &target).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("could not write {}: {error}", target.display())
    })?;
    Ok(converted)
}

/// Warnings, in the log as they happen and in the report at the end.
struct Notes(Vec<String>);

impl Notes {
    fn warn(&mut self, message: String) {
        log::warn!("{message}");
        self.0.push(message);
    }
}

/// A clip on its way to a lane, with the transition CapCut hung on the
/// segment it leaves (Concat hangs it on the clip it enters).
struct Pending {
    clip: Clip,
    transition: Option<(f64, String)>,
}

/// CapCut draft JSON to a Concat document. Pure: nothing is read or written.
pub fn convert(draft: &Value, meta: Option<&Value>, fallback_name: &str) -> Result<Converted, String> {
    let mut notes = Notes(Vec::new());

    // Every material by id, with the list it came from.
    let mut by_id: HashMap<&str, (&str, &Value)> = HashMap::new();
    if let Some(materials) = draft.get("materials").and_then(Value::as_object) {
        for (kind, entries) in materials {
            for entry in entries.as_array().map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    by_id.insert(id, (kind.as_str(), entry));
                }
            }
        }
    }

    let canvas = draft.get("canvas_config");
    let dimension = |key: &str, fallback: u32| {
        canvas
            .and_then(|canvas| canvas.get(key))
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .map_or(fallback, |value| value as u32)
    };
    let (width, height) = (dimension("width", 1920), dimension("height", 1080));
    let (rate_num, rate_den) = fps_to_rate(draft.get("fps").and_then(Value::as_f64).unwrap_or(30.0));
    let name = meta
        .and_then(|meta| meta.get("draft_name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| draft.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()))
        .unwrap_or(fallback_name)
        .to_owned();

    // The media bin: one item per distinct file, as Concat's own duplicate
    // check has it.
    let mut media: Vec<MediaItem> = Vec::new();
    let mut media_of: HashMap<String, String> = HashMap::new();
    let mut by_path: HashMap<String, String> = HashMap::new();
    for (list, audio) in [("videos", false), ("audios", true)] {
        let entries = draft
            .get("materials")
            .and_then(|materials| materials.get(list))
            .and_then(Value::as_array);
        for material in entries.map(Vec::as_slice).unwrap_or(&[]) {
            let Some(id) = material.get("id").and_then(Value::as_str) else {
                continue;
            };
            let kind = if audio {
                MediaKind::Audio
            } else if material.get("type").and_then(Value::as_str) == Some("photo") {
                MediaKind::Image
            } else {
                MediaKind::Video
            };
            let path = text(material, "path").replace('\\', "/");
            let key = if path.is_empty() { id.to_owned() } else { path.clone() };
            if !by_path.contains_key(&key) {
                let label = [text(material, "material_name"), text(material, "name")]
                    .into_iter()
                    .find(|label| !label.is_empty())
                    .or_else(|| path.rsplit('/').next().map(str::to_owned))
                    .unwrap_or_default();
                let mut item = MediaItem {
                    id: format!("m{}", media.len() + 1),
                    path: path.clone(),
                    name: label,
                    kind,
                    has_audio: material.get("has_audio").and_then(Value::as_bool).unwrap_or(audio),
                    ..MediaItem::default()
                };
                let duration = material.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
                if kind != MediaKind::Image && duration > 0.0 {
                    item.duration = Some(duration / US);
                }
                if kind != MediaKind::Audio {
                    let side = |key: &str| {
                        material
                            .get(key)
                            .and_then(Value::as_u64)
                            .filter(|value| *value > 0)
                            .map(|value| value as u32)
                    };
                    item.width = side("width");
                    item.height = side("height");
                }
                if path.is_empty() {
                    notes.warn(format!("media '{}' has no file path; relink it in Concat", item.name));
                }
                by_path.insert(key.clone(), item.id.clone());
                media.push(item);
            }
            media_of.insert(id.to_owned(), by_path[&key].clone());
        }
    }

    // Tracks become lanes. CapCut's first track is the bottom of the stack
    // and so is Concat's first lane (`Timeline::background_lane`): order kept.
    let mut lanes: Vec<Vec<Clip>> = Vec::new();
    let tracks = draft.get("tracks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    for (index, track) in tracks.iter().enumerate() {
        let kind = track.get("type").and_then(Value::as_str).unwrap_or("");
        let mut segments: Vec<&Value> = track
            .get("segments")
            .and_then(Value::as_array)
            .map(|segments| segments.iter().collect())
            .unwrap_or_default();
        if !matches!(kind, "video" | "audio" | "text" | "effect") {
            notes.warn(format!(
                "track {} of type '{kind}' is not supported; skipped ({} segment(s))",
                index + 1,
                segments.len()
            ));
            continue;
        }
        let start_of = |segment: &Value| {
            segment
                .get("target_timerange")
                .and_then(|range| range.get("start"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        segments.sort_by(|a, b| start_of(*a).total_cmp(&start_of(*b)));
        let mut pending: Vec<Pending> = segments
            .into_iter()
            .filter_map(|segment| segment_to_clip(segment, kind, &by_id, &media_of, &mut notes))
            .collect();
        apply_transitions(&mut pending, &mut notes);
        lanes.extend(place_without_overlap(
            pending.into_iter().map(|pending| pending.clip).collect(),
            &mut notes,
        ));
    }
    if lanes.is_empty() {
        return Err("the draft has no track this importer can read".to_owned());
    }

    // Ids last, once the order is final.
    let mut timeline = Timeline {
        id: "TL1".to_owned(),
        name: "Timeline 1".to_owned(),
        video: VideoSettings {
            width,
            height,
            rate_num,
            rate_den,
            ..VideoSettings::default()
        },
        ..Timeline::default()
    };
    let mut count = 0;
    for (number, lane) in lanes.into_iter().enumerate() {
        let track_id = format!("T{}", number + 1);
        timeline.tracks.push(Track {
            id: track_id.clone(),
            ..Track::default()
        });
        for mut clip in lane {
            count += 1;
            clip.id = format!("c{count}");
            clip.track_id = track_id.clone();
            timeline.clips.push(Arc::new(clip));
        }
    }
    let lane_count = timeline.tracks.len();
    let settings = DocumentSettings {
        name,
        width,
        height,
        rate_num,
        rate_den,
    };
    let media_count = media.len();
    let project = Project {
        media,
        timelines: vec![Arc::new(timeline)],
        active_timeline_id: "TL1".to_owned(),
        ..Project::default()
    };
    let document = to_document(&settings, &project);

    // The check a session's open makes. A file that would not open is worse
    // than no file.
    match from_document(&document) {
        Some(read) if read.active().clips.len() == count => {}
        Some(read) => {
            return Err(format!(
                "converted project lost clips on load ({} of {count} kept)",
                read.active().clips.len()
            ));
        }
        None => return Err("converted project does not load".to_owned()),
    }

    Ok(Converted {
        document,
        warnings: notes.0,
        media: media_count,
        lanes: lane_count,
        clips: count,
    })
}

/// A string field, empty when absent.
fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// CapCut microseconds as seconds.
fn seconds(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0) / US
}

/// A frame rate as the exact fraction the engine works in. NTSC rates are
/// fractions, not decimals.
fn fps_to_rate(fps: f64) -> (i64, i64) {
    for num in [24_000_i64, 30_000, 60_000] {
        if (fps - num as f64 / 1001.0).abs() < 0.01 {
            return (num, 1001);
        }
    }
    if (fps - fps.round()).abs() < 1e-6 {
        return (fps.round() as i64, 1);
    }
    ((fps * 1000.0).round() as i64, 1000)
}

/// The words of a CapCut text material: `<size=15><font ...>[words]</font>
/// </size>` in CapCut 1.5.x, a JSON object with a `text` field in other
/// builds.
fn words_of(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with('{')
        && let Ok(object) = serde_json::from_str::<Value>(trimmed)
        && let Some(words) = object.get("text").and_then(Value::as_str)
    {
        return words.trim().to_owned();
    }
    let mut plain = String::new();
    let mut in_tag = false;
    for character in content.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => plain.push(character),
            _ => {}
        }
    }
    if let (Some(open), Some(close)) = (plain.find('['), plain.rfind(']'))
        && open < close
    {
        plain = plain[open + 1..close].to_owned();
    }
    plain.trim().to_owned()
}

/// One CapCut segment as a clip, or `None` when it cannot be one.
fn segment_to_clip(
    segment: &Value,
    track_kind: &str,
    by_id: &HashMap<&str, (&str, &Value)>,
    media_of: &HashMap<String, String>,
    notes: &mut Notes,
) -> Option<Pending> {
    let material_id = segment.get("material_id").and_then(Value::as_str)?;
    let Some((_, material)) = by_id.get(material_id) else {
        notes.warn("a segment points at a material that is not in the draft; skipped".to_owned());
        return None;
    };
    let target = segment.get("target_timerange")?;
    let label = [text(material, "material_name"), text(material, "name")]
        .into_iter()
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| "clip".to_owned());
    let kind = match track_kind {
        "audio" => ClipKind::Audio,
        "text" => ClipKind::Text,
        "effect" => ClipKind::Layer,
        _ if material.get("type").and_then(Value::as_str) == Some("photo") => ClipKind::Image,
        _ => ClipKind::Video,
    };
    let mut clip = Clip::blank(
        "",
        "",
        kind,
        label.clone(),
        seconds(target.get("start")),
        seconds(target.get("duration")),
    );

    clip.source_start = seconds(segment.get("source_timerange").and_then(|range| range.get("start"))).max(0.0);
    let mut speed = segment.get("speed").and_then(Value::as_f64).unwrap_or(1.0);
    if !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
        let clamped = speed.clamp(MIN_SPEED, MAX_SPEED);
        notes.warn(format!(
            "speed {speed}x is outside Concat's {MIN_SPEED}-{MAX_SPEED}x range; using {clamped}x"
        ));
        speed = clamped;
    }
    clip.speed = speed;
    clip.volume = segment.get("volume").and_then(Value::as_f64).unwrap_or(1.0).max(0.0);

    let placed = segment.get("clip");
    let number = |from: Option<&Value>, key: &str, default: f64| {
        from.and_then(|from| from.get(key)).and_then(Value::as_f64).unwrap_or(default)
    };
    let scale = placed.and_then(|placed| placed.get("scale"));
    let transform = placed.and_then(|placed| placed.get("transform"));
    let flip = placed.and_then(|placed| placed.get("flip"));
    let scale_x = number(scale, "x", 1.0).clamp(MIN_SCALE, MAX_SCALE);
    let scale_y = number(scale, "y", 1.0).clamp(MIN_SCALE, MAX_SCALE);
    clip.scale = scale_x;
    if (scale_y - scale_x).abs() > 1e-6 {
        clip.stretch_y = (scale_y / scale_x).clamp(MIN_STRETCH, MAX_STRETCH);
    }
    // CapCut: units of half the canvas, y up. Concat: frame fractions, y down.
    clip.offset_x = (number(transform, "x", 0.0) / 2.0).clamp(-MAX_OFFSET, MAX_OFFSET);
    clip.offset_y = (-number(transform, "y", 0.0) / 2.0).clamp(-MAX_OFFSET, MAX_OFFSET);
    clip.rotation = wrap_rotation(number(placed, "rotation", 0.0));
    clip.opacity = number(placed, "alpha", 1.0).clamp(0.0, 1.0);
    clip.flip_h = flip.and_then(|flip| flip.get("horizontal")).and_then(Value::as_bool).unwrap_or(false);
    clip.flip_v = flip.and_then(|flip| flip.get("vertical")).and_then(Value::as_bool).unwrap_or(false);

    match kind {
        ClipKind::Video | ClipKind::Audio | ClipKind::Image => {
            let Some(media_id) = media_of.get(material_id) else {
                notes.warn(format!("segment of '{label}' has no media; skipped"));
                return None;
            };
            clip.media_id = media_id.clone();
            if kind == ClipKind::Image {
                clip.source_start = 0.0;
                clip.speed = 1.0;
            }
        }
        ClipKind::Text => {
            let words = words_of(&text(material, "content"));
            if words.is_empty() {
                notes.warn("a text segment has no readable text; skipped".to_owned());
                return None;
            }
            let mut style = TextStyle {
                content: words.clone(),
                ..TextStyle::default()
            };
            let color = text(material, "text_color");
            if !color.is_empty() {
                style.color = color.to_lowercase();
            }
            if let Some(size) = material.get("font_size").and_then(Value::as_f64) {
                style.font_size = size * TEXT_SIZE_UNIT;
            }
            clip.name = words.lines().next().unwrap_or("").chars().take(40).collect();
            clip.text = Some(style);
        }
        ClipKind::Layer => {
            // Only effects checked against the catalogue are mapped.
            let effect = match text(material, "name").to_lowercase().as_str() {
                "blur" => "concat.gaussian-blur",
                other => {
                    notes.warn(format!("effect '{other}' has no Concat equivalent; skipped"));
                    return None;
                }
            };
            clip.speed = 1.0;
            clip.video_effects.push(AppliedFilter {
                id: effect.to_owned(),
                params: BTreeMap::new(),
                enabled: true,
                keys: BTreeMap::new(),
            });
        }
        ClipKind::Shape => return None,
    }

    let transition = segment
        .get("extra_material_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|reference| by_id.get(reference))
        .find(|(list, _)| *list == "transitions")
        .map(|(_, found)| {
            let length = seconds(found.get("duration"));
            (if length > 0.0 { length } else { 1.0 }, text(found, "name"))
        });
    Some(Pending { clip, transition })
}

/// Moves each CapCut transition onto the clip it enters and settles the cut.
///
/// CapCut overlaps the two segments by the transition's length. Concat keeps
/// clips touching and centres the transition on the cut, so the cut goes in
/// the middle of CapCut's overlap and both blend windows coincide. The
/// incoming clip loses the front half of the overlap and its in-point moves
/// by the same amount of source, so what plays at every instant outside the
/// blend is unchanged.
fn apply_transitions(pending: &mut [Pending], notes: &mut Notes) {
    for index in 0..pending.len().saturating_sub(1) {
        let Some((requested, name)) = pending[index].transition.take() else {
            continue;
        };
        let (head, tail) = pending.split_at_mut(index + 1);
        let left = &mut head[index].clip;
        let right = &mut tail[0].clip;
        let overlap = left.start + left.duration - right.start;
        if overlap > TOUCH {
            let cut = right.start + overlap / 2.0;
            let shift = cut - right.start;
            left.duration = (cut - left.start).max(MIN_CLIP_DURATION);
            right.start = cut;
            right.duration = (right.duration - shift).max(MIN_CLIP_DURATION);
            right.source_start += shift * right.speed;
        }
        let label = if name.is_empty() { "transition" } else { name.as_str() };
        notes.warn(format!("transition '{label}' has no Concat equivalent; using a dissolve"));
        let longest = left.duration.min(right.duration).max(MIN_TRANSITION);
        right.transition_in = Some(Transition {
            id: FALLBACK_TRANSITION.to_owned(),
            duration: requested.clamp(MIN_TRANSITION, longest),
            params: BTreeMap::new(),
        });
    }
}

/// Concat never lets one clip cover another on a lane: a clip that would
/// goes onto an extra lane above.
fn place_without_overlap(clips: Vec<Clip>, notes: &mut Notes) -> Vec<Vec<Clip>> {
    let mut lanes: Vec<Vec<Clip>> = Vec::new();
    for clip in clips {
        let fits = lanes.iter().position(|lane| {
            lane.last()
                .is_none_or(|last| clip.start >= last.start + last.duration - TOUCH)
        });
        match fits {
            Some(index) => lanes[index].push(clip),
            None => {
                if !lanes.is_empty() {
                    notes.warn(format!(
                        "clips overlap on one CapCut track around {:.2}s; moved to an extra lane",
                        clip.start
                    ));
                }
                lanes.push(vec![clip]);
            }
        }
    }
    lanes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two clips of one file with a transition between them, a title, and a
    /// music clip too fast for Concat - the shapes the real draft this was
    /// written from has. Times in microseconds, as CapCut writes them.
    fn draft() -> Value {
        json!({
            "canvas_config": { "width": 1920, "height": 1080 },
            "fps": 60.0,
            "materials": {
                "videos": [{ "id": "v", "type": "video", "path": "C:/m/a.mp4", "material_name": "a",
                             "duration": 14_400_000, "width": 1920, "height": 1080, "has_audio": false }],
                "audios": [{ "id": "au", "type": "music", "path": "C:/m/s.mp3", "name": "song",
                             "duration": 189_900_000 }],
                "texts": [{ "id": "t", "content": "<size=15><font id=\"\" path=\"x.ttf\">[hello world ]</font></size>",
                            "text_color": "#FFFFFF", "font_size": 15.0 }],
                "transitions": [{ "id": "tr", "name": "Comparison II", "duration": 800_000 }]
            },
            "tracks": [
                { "type": "video", "segments": [
                    { "material_id": "v", "speed": 1.0, "extra_material_refs": ["tr"],
                      "target_timerange": { "start": 2_983_000, "duration": 4_067_000 },
                      "source_timerange": { "start": 2_983_000, "duration": 4_067_000 } },
                    { "material_id": "v", "speed": 1.6,
                      "target_timerange": { "start": 6_250_000, "duration": 2_575_000 },
                      "source_timerange": { "start": 7_050_000, "duration": 4_120_000 } } ] },
                { "type": "text", "segments": [
                    { "material_id": "t",
                      "target_timerange": { "start": 1_883_000, "duration": 3_000_000 } } ] },
                { "type": "audio", "segments": [
                    { "material_id": "au", "speed": 38.0,
                      "target_timerange": { "start": 0, "duration": 5_000_000 },
                      "source_timerange": { "start": 0, "duration": 189_900_000 } } ] }
            ]
        })
    }

    fn converted() -> Converted {
        convert(&draft(), None, "folder").expect("converts")
    }

    fn clips(converted: &Converted) -> Vec<Value> {
        converted.document["clips"].as_array().expect("clips").clone()
    }

    #[test]
    fn the_cut_goes_in_the_middle_of_the_overlap() {
        let converted = converted();
        let clips = clips(&converted);
        let (left, right) = (&clips[0], &clips[1]);
        // CapCut overlaps 6.25s-7.05s; the cut is its middle, 6.65s.
        assert!((right["start"].as_f64().unwrap() - 6.65).abs() < 1e-9);
        assert!((left["start"].as_f64().unwrap() + left["duration"].as_f64().unwrap() - 6.65).abs() < 1e-9);
        // The in-point moved by the 0.4s dropped, at 1.6x.
        assert!((right["sourceStart"].as_f64().unwrap() - (7.05 + 0.4 * 1.6)).abs() < 1e-9);
        assert_eq!(right["transitionIn"]["id"], "concat.dissolve");
        assert!((right["transitionIn"]["duration"].as_f64().unwrap() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn text_speed_and_media_are_read() {
        let converted = converted();
        let clips = clips(&converted);
        let title = clips.iter().find(|clip| clip["kind"] == "text").expect("title");
        assert_eq!(title["text"]["content"], "hello world");
        assert_eq!(title["text"]["color"], "#ffffff");
        let music = clips.iter().find(|clip| clip["kind"] == "audio").expect("music");
        assert_eq!(music["speed"].as_f64().unwrap(), MAX_SPEED);
        assert_eq!(converted.media, 2);
        assert!(converted.warnings.iter().any(|line| line.contains("38")));
    }

    #[test]
    fn no_lane_has_clips_covering_each_other() {
        let converted = converted();
        let read = from_document(&converted.document).expect("loads");
        let timeline = read.active();
        for track in &timeline.tracks {
            let mut on_lane: Vec<_> = timeline.clips.iter().filter(|clip| clip.track_id == track.id).collect();
            on_lane.sort_by(|a, b| a.start.total_cmp(&b.start));
            for pair in on_lane.windows(2) {
                assert!(pair[1].start >= pair[0].start + pair[0].duration - TOUCH);
            }
        }
    }

    #[test]
    fn a_draft_with_nothing_to_read_is_refused() {
        let draft = json!({ "tracks": [{ "type": "sticker", "segments": [] }] });
        assert!(convert(&draft, None, "x").is_err());
    }

    #[test]
    fn ntsc_rates_are_fractions() {
        assert_eq!(fps_to_rate(29.97), (30_000, 1001));
        assert_eq!(fps_to_rate(60.0), (60, 1));
    }
}
