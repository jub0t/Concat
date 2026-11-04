// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A CapCut draft read into a Concat project (#477).
//!
//! CapCut keeps each draft as a folder of JSON: the edit of each timeline
//! in a `draft_info.json` (macOS, and every build since the desktop app
//! moved to it) or a `draft_content.json` (Windows and the older builds),
//! the media referenced by path, and its times in microseconds. This is
//! the pure half of an import: the drafts' JSON in, a [`Project`] out,
//! nothing read or written. The host finds the files and writes the new
//! project's folder; the draft itself is never touched.
//!
//! What comes across is the cut: every timeline, every lane, each clip's
//! timing, in-point, speed and speed curve, volume, fades, crop, place, size, turn,
//! opacity and flips, their keyframes, titles with their words and their
//! look, and each transition as a transition clip over its cut. What does
//! not - CapCut's own filters, effects, stickers, animations, masks, a
//! clip played backwards - is said, once per kind, in
//! [`Imported::notes`], so nobody is left wondering where it went.
//!
//! The draft's format is CapCut's and changes between its builds. Every
//! field is read as optional: what a build does not write falls back to
//! what a plain clip has.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;

use crate::commands::{self, Command, IdMint};
use crate::doc::DocumentSettings;
use crate::model::ranges::{
    MAX_OFFSET, MAX_SCALE, MAX_SPEED, MAX_STRETCH, MIN_CLIP_DURATION, MIN_SCALE, MIN_SPEED,
    MIN_STRETCH, MIN_TRANSITION, wrap_rotation,
};
use crate::model::{
    Background, Clip, ClipKey, ClipKind, Crop, KeyEase, KeyProperty, MediaItem, MediaKind, Project,
    SpeedPoint, TextAlign, TextStyle, Timeline, Track, VideoSettings,
};
use crate::placement::TOUCH;

/// The file a draft folder's edit is in, newest builds first.
pub const DRAFT_FILES: [&str; 2] = ["draft_info.json", "draft_content.json"];

/// What a draft folder starts every media path with when the file was
/// copied into the draft: `##_draftpath_placeholder_<id>_##/...`.
const PLACEHOLDER: &str = "##_draftpath_placeholder_";

/// CapCut's times are microseconds.
const MICROS: f64 = 1_000_000.0;

/// A CapCut title's size over Concat's: CapCut's default title, 15, as
/// Concat's default title, nine hundredths of the frame's height.
/// Approximate; a title can be resized after.
const TEXT_SIZE: f64 = 0.09 / 15.0;

/// One timeline of a draft: its name and its edit.
pub struct DraftTimeline {
    /// What CapCut calls it, "Timeline 01"; empty for a draft of one.
    pub name: String,
    /// The `draft_info.json` (or `draft_content.json`) of it.
    pub edit: Value,
}

/// A draft as the host read it.
pub struct Draft {
    /// What CapCut calls the draft (`draft_meta_info.json`'s
    /// `draft_name`), or the folder's name.
    pub name: String,
    /// The draft's folder, which a copied file's placeholder path stands
    /// for.
    pub folder: String,
    /// Its timelines, the one CapCut opens first first.
    pub timelines: Vec<DraftTimeline>,
}

/// A draft read into a project.
pub struct Imported {
    /// The project: the draft's media, and a timeline for each of its own.
    pub project: Project,
    /// The settings the document is written with: the first timeline's
    /// frame and rate, and the draft's name.
    pub settings: DocumentSettings,
    /// What did not come across, or came across changed, once per kind.
    pub notes: Vec<String>,
}

/// Why a draft could not be read at all.
#[derive(Debug, PartialEq, Eq)]
pub enum CapcutError {
    /// No timeline of the draft has a clip this can read.
    Empty,
}

impl std::fmt::Display for CapcutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapcutError::Empty => write!(f, "the CapCut draft has no clip Concat can read"),
        }
    }
}

impl std::error::Error for CapcutError {}

/// What did not come across, counted by what it was, said once each.
#[derive(Default)]
struct Notes(BTreeMap<String, usize>);

impl Notes {
    fn note(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_insert(0) += 1;
    }

    fn lines(self) -> Vec<String> {
        self.0
            .into_iter()
            .map(|(what, count)| {
                if count == 1 {
                    what
                } else {
                    format!("{what} (×{count})")
                }
            })
            .collect()
    }
}

/// Reads `draft` into a project.
pub fn import(draft: &Draft) -> Result<Imported, CapcutError> {
    let mut mint = IdMint::default();
    let mut notes = Notes::default();
    // No starter timeline: the draft's are the project's.
    let mut project = Project {
        timelines: Vec::new(),
        ..Project::default()
    };
    let mut media = MediaBin::default();
    let mut first: Option<DocumentSettings> = None;

    for (index, part) in draft.timelines.iter().enumerate() {
        let mut timeline = read_timeline(part, draft, &mut media, &mut mint, &mut notes);
        if timeline.clips.is_empty() {
            continue;
        }
        timeline.id = mint.next("TL");
        if timeline.name.is_empty() {
            timeline.name = format!("Timeline {}", index + 1);
        }
        let video = &timeline.video;
        first.get_or_insert_with(|| DocumentSettings {
            name: draft.name.clone(),
            width: video.width,
            height: video.height,
            rate_num: video.rate_num,
            rate_den: video.rate_den,
            canvas: video.canvas,
        });
        let transitions = std::mem::take(&mut media.transitions);
        let id = timeline.id.clone();
        project.timelines.push(Arc::new(timeline));
        project.active_timeline_id = id;
        // Each through the editor's own command, so it lands on a lane the
        // way one dragged on would.
        for (start, duration, package, name) in transitions {
            let placed = commands::apply(
                &mut project,
                &mut mint,
                Command::AddTransitionClip {
                    track_id: None,
                    start,
                    duration: Some(duration),
                    transition_id: package.to_owned(),
                    name,
                },
            );
            if placed.is_err() {
                notes.note("a transition that would not fit on the timeline");
            }
        }
    }
    let Some(settings) = first else {
        return Err(CapcutError::Empty);
    };
    project.active_timeline_id = project.timelines[0].id.clone();
    project.media = media.items;
    Ok(Imported {
        project,
        settings,
        notes: notes.lines(),
    })
}

/// The project's media, one item a file across every timeline, and the
/// transitions the timeline being read has found.
#[derive(Default)]
struct MediaBin {
    items: Vec<MediaItem>,
    by_path: HashMap<String, String>,
    /// `(start, duration, package, name)` of each transition on the
    /// timeline being read.
    transitions: Vec<(f64, f64, &'static str, String)>,
}

impl MediaBin {
    /// The item for `material`'s file, made the first time it is met.
    fn item_for(
        &mut self,
        material: &Value,
        kind: MediaKind,
        folder: &str,
        mint: &mut IdMint,
    ) -> String {
        let path = resolve(&text(material, "path"), folder);
        if let Some(id) = self.by_path.get(&path) {
            return id.clone();
        }
        let id = mint.next("m");
        let name = [text(material, "material_name"), text(material, "name")]
            .into_iter()
            .find(|name| !name.is_empty())
            .unwrap_or_else(|| file_name(&path));
        let side = |key: &str| {
            material
                .get(key)
                .and_then(Value::as_u64)
                .filter(|side| *side > 0)
                .and_then(|side| u32::try_from(side).ok())
        };
        let duration = micros(material.get("duration"));
        self.items.push(MediaItem {
            id: id.clone(),
            path: path.clone(),
            name,
            kind,
            duration: (kind != MediaKind::Image && duration > 0.0).then_some(duration),
            width: (kind != MediaKind::Audio).then(|| side("width")).flatten(),
            height: (kind != MediaKind::Audio).then(|| side("height")).flatten(),
            has_audio: material
                .get("has_audio")
                .and_then(Value::as_bool)
                .unwrap_or(kind == MediaKind::Audio),
            ..MediaItem::default()
        });
        self.by_path.insert(path, id.clone());
        id
    }
}

/// A material's path with a copied file's placeholder replaced by the
/// draft's folder.
fn resolve(path: &str, folder: &str) -> String {
    match path.strip_prefix(PLACEHOLDER) {
        Some(rest) => match rest.split_once("_##") {
            Some((_, tail)) => format!("{}{tail}", folder.trim_end_matches(['/', '\\'])),
            None => path.to_owned(),
        },
        None => path.to_owned(),
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

/// A string field, empty when absent.
fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// A number field.
fn number(value: Option<&Value>, key: &str) -> Option<f64> {
    value.and_then(|value| value.get(key)).and_then(Value::as_f64)
}

/// CapCut microseconds as seconds.
fn micros(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0) / MICROS
}

/// A frame rate as the exact fraction the engine works in.
fn rate_of(fps: f64) -> (i64, i64) {
    for num in [24_000_i64, 30_000, 60_000] {
        if (fps - num as f64 / 1001.0).abs() < 0.005 {
            return (num, 1001);
        }
    }
    if fps > 0.0 && (fps - fps.round()).abs() < 1e-3 {
        return (fps.round() as i64, 1);
    }
    if fps > 0.0 {
        return ((fps * 1000.0).round() as i64, 1000);
    }
    (30, 1)
}

/// One of the draft's timelines, its media into `bin`, its transitions
/// into `bin.transitions` for the caller to place.
fn read_timeline(
    part: &DraftTimeline,
    draft: &Draft,
    bin: &mut MediaBin,
    mint: &mut IdMint,
    notes: &mut Notes,
) -> Timeline {
    let edit = &part.edit;
    let canvas = edit.get("canvas_config");
    let side = |key: &str, fallback: u32| {
        number(canvas, key)
            .filter(|side| *side >= 2.0)
            .map_or(fallback, |side| (side as u32) & !1)
    };
    let (rate_num, rate_den) = rate_of(edit.get("fps").and_then(Value::as_f64).unwrap_or(30.0));
    let mut timeline = Timeline {
        name: part.name.clone(),
        video: VideoSettings {
            width: side("width", 1920),
            height: side("height", 1080),
            rate_num,
            rate_den,
            ..VideoSettings::default()
        },
        // As a new project's: no fill behind the picture, and a clip's
        // sound moving with its picture.
        background: Background::None,
        linkage: true,
        ..Timeline::default()
    };

    // Every material by its id, with the list it is in.
    let mut materials: HashMap<&str, (&str, &Value)> = HashMap::new();
    if let Some(lists) = edit.get("materials").and_then(Value::as_object) {
        for (list, entries) in lists {
            for entry in entries.as_array().into_iter().flatten() {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    materials.insert(id, (list.as_str(), entry));
                }
            }
        }
    }
    let reader = Reader {
        materials: &materials,
        folder: &draft.folder,
    };

    let tracks = edit.get("tracks").and_then(Value::as_array);
    for track in tracks.into_iter().flatten() {
        let kind = track.get("type").and_then(Value::as_str).unwrap_or("");
        let mut segments: Vec<&Value> = track
            .get("segments")
            .and_then(Value::as_array)
            .map(|segments| segments.iter().collect())
            .unwrap_or_default();
        if !matches!(kind, "video" | "audio" | "text") {
            if !segments.is_empty() {
                notes.note(format!("CapCut {kind} lane, not carried"));
            }
            continue;
        }
        segments.sort_by(|a, b| start_of(a).total_cmp(&start_of(b)));
        let mut clips: Vec<(Clip, Option<(f64, String)>)> = segments
            .into_iter()
            .filter_map(|segment| reader.clip(segment, kind, bin, mint, notes))
            .collect();
        bin.transitions.extend(cuts(&mut clips));
        for lane in lanes(clips.into_iter().map(|(clip, _)| clip).collect()) {
            let track_id = mint.next("T");
            let muted = track.get("attribute").and_then(Value::as_u64).unwrap_or(0) & 1 != 0;
            timeline.tracks.push(Track {
                id: track_id.clone(),
                muted,
                ..Track::default()
            });
            for mut clip in lane {
                clip.id = mint.next("c");
                clip.track_id = track_id.clone();
                timeline.clips.push(Arc::new(clip));
            }
        }
    }
    timeline
}

fn start_of(segment: &Value) -> f64 {
    number(segment.get("target_timerange"), "start").unwrap_or(0.0)
}

/// What a segment's references point at.
struct Reader<'a> {
    materials: &'a HashMap<&'a str, (&'a str, &'a Value)>,
    folder: &'a str,
}

impl Reader<'_> {
    /// The materials `segment` refers to beside its own, by the list each
    /// is in.
    fn extras<'v>(&'v self, segment: &'v Value) -> impl Iterator<Item = (&'v str, &'v Value)> {
        segment
            .get("extra_material_refs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|id| self.materials.get(id).copied())
    }

    /// One segment as a clip, with the transition CapCut hung on it - the
    /// one into the next clip - as its length and name. `None` when it
    /// cannot be a clip.
    fn clip(
        &self,
        segment: &Value,
        track: &str,
        bin: &mut MediaBin,
        mint: &mut IdMint,
        notes: &mut Notes,
    ) -> Option<(Clip, Option<(f64, String)>)> {
        let id = segment.get("material_id").and_then(Value::as_str)?;
        let Some(&(list, material)) = self.materials.get(id) else {
            notes.note("a clip whose media is not in the draft");
            return None;
        };
        let target = segment.get("target_timerange");
        let (start, duration) = (
            micros(target.and_then(|range| range.get("start"))).max(0.0),
            micros(target.and_then(|range| range.get("duration"))),
        );
        if duration < MIN_CLIP_DURATION {
            return None;
        }
        if segment.get("visible").and_then(Value::as_bool) == Some(false) {
            notes.note("a clip hidden in CapCut, left out");
            return None;
        }
        let kind = match (track, list) {
            ("text", _) | (_, "texts") => ClipKind::Text,
            ("audio", _) | (_, "audios") => ClipKind::Audio,
            _ if material.get("type").and_then(Value::as_str) == Some("photo") => ClipKind::Image,
            _ => ClipKind::Video,
        };
        let name = [text(material, "material_name"), text(material, "name")]
            .into_iter()
            .find(|name| !name.is_empty())
            .unwrap_or_default();
        let mut clip = Clip::blank("", "", kind, name, start, duration);
        clip.source_start = micros(
            segment
                .get("source_timerange")
                .and_then(|range| range.get("start")),
        )
        .max(0.0);
        clip.volume = segment
            .get("volume")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 10.0);
        self.speed(segment, &mut clip, notes);
        self.place(segment, &mut clip);
        self.keys(segment, &mut clip, notes);
        if segment.get("reverse").and_then(Value::as_bool) == Some(true) {
            notes.note("a clip played backwards, imported playing forwards");
        }

        let mut transition = None;
        for (list, extra) in self.extras(segment) {
            match list {
                "audio_fades" => {
                    clip.fade_in = micros(extra.get("fade_in_duration")).min(duration / 2.0);
                    clip.fade_out = micros(extra.get("fade_out_duration")).min(duration / 2.0);
                }
                "transitions" => {
                    let length = micros(extra.get("duration"));
                    transition = Some((
                        if length > 0.0 { length } else { 0.5 },
                        text(extra, "name"),
                    ));
                }
                "speeds" | "canvases" | "placeholder_infos" | "sound_channel_mappings"
                | "material_colors" | "vocal_separations" | "beats" | "loudnesses" => {}
                "material_animations" => {
                    if extra
                        .get("animations")
                        .and_then(Value::as_array)
                        .is_some_and(|animations| !animations.is_empty())
                    {
                        notes.note("CapCut in/out animation, not carried");
                    }
                }
                other => notes.note(format!("CapCut {} on a clip, not carried", plain(other))),
            }
        }

        match kind {
            ClipKind::Video | ClipKind::Image | ClipKind::Audio => {
                let media = match kind {
                    ClipKind::Audio => MediaKind::Audio,
                    ClipKind::Image => MediaKind::Image,
                    _ => MediaKind::Video,
                };
                clip.media_id = bin.item_for(material, media, self.folder, mint);
                if kind == ClipKind::Image {
                    clip.source_start = 0.0;
                    clip.speed = 1.0;
                }
                if kind != ClipKind::Audio {
                    clip.crop = crop_of(material);
                }
            }
            ClipKind::Text => {
                let style = title_of(material)?;
                clip.name = style.content.lines().next().unwrap_or("").chars().take(40).collect();
                clip.text = Some(style);
            }
            _ => return None,
        }
        Some((clip, transition))
    }

    /// The clip's speed: the segment's, or its speed material's - a curve
    /// when CapCut's is one, its points over the clip's length as
    /// Concat's are, joined by straight lines.
    fn speed(&self, segment: &Value, clip: &mut Clip, notes: &mut Notes) {
        let mut speed = segment.get("speed").and_then(Value::as_f64).unwrap_or(1.0);
        if let Some((_, material)) = self.extras(segment).find(|(list, _)| *list == "speeds") {
            speed = material.get("speed").and_then(Value::as_f64).unwrap_or(speed);
            if material.get("mode").and_then(Value::as_i64).unwrap_or(0) != 0 {
                let points: Vec<SpeedPoint> = material
                    .pointer("/curve_speed/speed_points")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|point| {
                        Some(SpeedPoint {
                            at: point.get("x")?.as_f64()?.clamp(0.0, 1.0),
                            speed: point.get("y")?.as_f64()?.clamp(MIN_SPEED, MAX_SPEED),
                            slope: None,
                        })
                    })
                    .collect();
                if crate::speed::curve_of(&points).is_some() {
                    clip.speed = crate::speed::mean_of(&points);
                    clip.speed_curve = Some(points);
                    return;
                }
                notes.note("a CapCut speed curve Concat could not read, imported as its average");
            }
        }
        // What the clip covers of its source says the speed best when the
        // two disagree: it is what CapCut plays.
        let source = micros(
            segment
                .get("source_timerange")
                .and_then(|range| range.get("duration")),
        );
        if source > 0.0 && clip.duration > 0.0 && (source / clip.duration - speed).abs() > 0.01 {
            speed = source / clip.duration;
        }
        if !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
            notes.note(format!(
                "a speed past Concat's {MIN_SPEED}-{MAX_SPEED}x, held to it"
            ));
        }
        clip.speed = speed.clamp(MIN_SPEED, MAX_SPEED);
    }

    /// Where the picture sits: CapCut's place is in half the canvas, y
    /// up; Concat's in the whole frame, y down. Both measure the size over
    /// the picture fitted inside the frame, and turn clockwise.
    fn place(&self, segment: &Value, clip: &mut Clip) {
        let placed = segment.get("clip");
        let scale = placed.and_then(|placed| placed.get("scale"));
        let moved = placed.and_then(|placed| placed.get("transform"));
        let flip = placed.and_then(|placed| placed.get("flip"));
        let x = number(scale, "x").unwrap_or(1.0).abs().max(MIN_SCALE);
        let y = number(scale, "y").unwrap_or(x).abs().max(MIN_SCALE);
        clip.scale = x.clamp(MIN_SCALE, MAX_SCALE);
        if (y - x).abs() > 1e-6 {
            clip.stretch_y = (y / x).clamp(MIN_STRETCH, MAX_STRETCH);
        }
        clip.offset_x = (number(moved, "x").unwrap_or(0.0) / 2.0).clamp(-MAX_OFFSET, MAX_OFFSET);
        clip.offset_y = (-number(moved, "y").unwrap_or(0.0) / 2.0).clamp(-MAX_OFFSET, MAX_OFFSET);
        clip.rotation = wrap_rotation(number(placed, "rotation").unwrap_or(0.0));
        clip.opacity = number(placed, "alpha").unwrap_or(1.0).clamp(0.0, 1.0);
        let flipped = |key: &str| {
            flip.and_then(|flip| flip.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        clip.flip_h = flipped("horizontal");
        clip.flip_v = flipped("vertical");
    }

    /// The keyframes Concat has a property for, eased straight from one
    /// to the next as CapCut's default is.
    fn keys(&self, segment: &Value, clip: &mut Clip, notes: &mut Notes) {
        let length = clip.duration * MICROS;
        let lists = segment.get("common_keyframes").and_then(Value::as_array);
        for list in lists.into_iter().flatten() {
            let kind = list.get("property_type").and_then(Value::as_str).unwrap_or("");
            let (property, value): (KeyProperty, fn(f64) -> f64) = match kind {
                "KFTypePositionX" => (KeyProperty::OffsetX, |x| x / 2.0),
                "KFTypePositionY" => (KeyProperty::OffsetY, |y| -y / 2.0),
                "KFTypeScaleX" | "UNIFORM_SCALE" => (KeyProperty::Scale, |s| s),
                "KFTypeRotation" => (KeyProperty::Rotation, |r| r),
                "KFTypeAlpha" => (KeyProperty::Opacity, |a| a.clamp(0.0, 1.0)),
                "KFTypeVolume" => (KeyProperty::Volume, |v| v.max(0.0)),
                "KFTypeScaleY" => continue,
                _ => {
                    notes.note("a CapCut keyframe Concat has no property for, not carried");
                    continue;
                }
            };
            for key in list
                .get("keyframe_list")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(raw) = key
                    .get("values")
                    .and_then(Value::as_array)
                    .and_then(|values| values.first())
                    .and_then(Value::as_f64)
                else {
                    continue;
                };
                let at = key.get("time_offset").and_then(Value::as_f64).unwrap_or(0.0);
                clip.keys.push(ClipKey {
                    property,
                    at: if length > 0.0 { (at / length).clamp(0.0, 1.0) } else { 0.0 },
                    value: value(raw),
                    ease: KeyEase::LINEAR,
                });
            }
        }
    }
}

/// What a material list holds, in words: "hsl" as "HSL adjustment".
fn plain(list: &str) -> String {
    match list {
        "effects" | "video_effects" => "effect".to_owned(),
        "hsl" => "HSL adjustment".to_owned(),
        "chromas" => "chroma key".to_owned(),
        "smart_relights" => "relighting".to_owned(),
        "material_colors" | "color_curves" | "adjusts" => "colour adjustment".to_owned(),
        other => other.trim_end_matches('s').replace('_', " "),
    }
}

/// The crop a video material carries, as its four corners; `None` for the
/// whole picture or a corner pin Concat's square crop cannot hold.
fn crop_of(material: &Value) -> Option<Crop> {
    let corners = material.get("crop")?;
    let at = |key: &str| number(Some(corners), key);
    let (left, top) = (at("upper_left_x")?, at("upper_left_y")?);
    let (right, bottom) = (at("lower_right_x")?, at("lower_right_y")?);
    let square = (at("upper_right_x")? - right).abs() < 1e-4
        && (at("upper_right_y")? - top).abs() < 1e-4
        && (at("lower_left_x")? - left).abs() < 1e-4
        && (at("lower_left_y")? - bottom).abs() < 1e-4;
    let crop = Crop {
        left: left.clamp(0.0, 1.0),
        top: top.clamp(0.0, 1.0),
        right: (1.0 - right).clamp(0.0, 1.0),
        bottom: (1.0 - bottom).clamp(0.0, 1.0),
    };
    (square && !crop.is_none()).then_some(crop)
}

/// A text material as a title: its words, and the look of its first run.
/// CapCut has written the words three ways - a JSON object whose `text`
/// and `styles` say it all, `<size><font>[words]</font></size>` markup,
/// and plain text - and every one is read. `None` for a title with no
/// words.
fn title_of(material: &Value) -> Option<TextStyle> {
    let content = text(material, "content");
    let parsed: Option<Value> = content
        .trim()
        .starts_with('{')
        .then(|| serde_json::from_str(content.trim()).ok())
        .flatten();
    let words = match parsed.as_ref().and_then(|parsed| parsed.get("text")) {
        Some(words) => words.as_str().unwrap_or("").to_owned(),
        None => unmarked(&content),
    };
    let words = words.trim().to_owned();
    if words.is_empty() {
        return None;
    }
    let mut style = TextStyle {
        content: words,
        ..TextStyle::default()
    };
    let run = parsed
        .as_ref()
        .and_then(|parsed| parsed.get("styles"))
        .and_then(Value::as_array)
        .and_then(|styles| styles.first());
    let colour = run
        .and_then(|run| run.pointer("/fill/content/solid/color"))
        .and_then(Value::as_array)
        .and_then(|rgb| hex(rgb))
        .or_else(|| Some(text(material, "text_color")).filter(|colour| colour.starts_with('#')));
    if let Some(colour) = colour {
        style.color = colour.to_lowercase();
    }
    let size = number(run, "size").or_else(|| number(Some(material), "font_size"));
    if let Some(size) = size.filter(|size| *size > 0.0) {
        style.font_size = size * TEXT_SIZE;
    }
    let flag = |key: &str| run.and_then(|run| run.get(key)).and_then(Value::as_bool);
    style.italic = flag("italic").unwrap_or(style.italic);
    style.underline = flag("underline").unwrap_or(style.underline);
    // CapCut's titles are regular unless made bold; Concat's start bold.
    if run.is_some() {
        style.font_weight = if flag("bold") == Some(true) { 700.0 } else { 400.0 };
    }
    style.align = match material.get("alignment").and_then(Value::as_u64) {
        Some(0) => TextAlign::Left,
        Some(2) => TextAlign::Right,
        _ => TextAlign::Center,
    };
    if let Some(alpha) = number(Some(material), "global_alpha") {
        style.opacity = alpha.clamp(0.0, 1.0);
    }
    Some(style)
}

/// Markup's words: what is outside the tags, and inside its brackets when
/// it has them.
fn unmarked(content: &str) -> String {
    let mut plain = String::new();
    let mut tag = false;
    for character in content.chars() {
        match character {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => plain.push(character),
            _ => {}
        }
    }
    match (plain.find('['), plain.rfind(']')) {
        (Some(open), Some(close)) if open < close => plain[open + 1..close].to_owned(),
        _ => plain,
    }
}

/// `[r, g, b]` in `0..1` as `#rrggbb`.
fn hex(rgb: &[Value]) -> Option<String> {
    let channel = |index: usize| {
        rgb.get(index)
            .and_then(Value::as_f64)
            .map(|level| (level.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channel(0)?,
        channel(1)?,
        channel(2)?
    ))
}

/// The cuts each clip's transition goes over, as `(start, length, package,
/// name)`. CapCut draws a transition by overlapping the two clips by its
/// length; Concat keeps a lane's clips end to end and lays the transition
/// over the cut. So where two overlap, the cut goes in the middle of the
/// overlap - the incoming clip starts later by half of it, and its in-point
/// moves by the source that covers - and the transition spans the overlap
/// it was drawn over. Where they only touch, it is centred on the cut.
fn cuts(clips: &mut [(Clip, Option<(f64, String)>)]) -> Vec<(f64, f64, &'static str, String)> {
    let mut placed = Vec::new();
    for index in 0..clips.len().saturating_sub(1) {
        let Some((length, name)) = clips[index].1.take() else {
            continue;
        };
        let (head, tail) = clips.split_at_mut(index + 1);
        let (left, right) = (&mut head[index].0, &mut tail[0].0);
        let end = left.start + left.duration;
        if right.start - end > TOUCH {
            // A gap between them: CapCut does not draw one there.
            continue;
        }
        let overlap = end - right.start;
        let cut = if overlap > TOUCH {
            let cut = right.start + overlap / 2.0;
            let shift = cut - right.start;
            left.duration = (cut - left.start).max(MIN_CLIP_DURATION);
            right.start = cut;
            right.duration = (right.duration - shift).max(MIN_CLIP_DURATION);
            right.source_start += shift * right.speed;
            cut
        } else {
            right.start
        };
        let span = if overlap > TOUCH { overlap } else { length };
        let longest = 2.0 * left.duration.min(right.duration);
        let span = span.min(longest).max(MIN_TRANSITION);
        let package = package_for(&name);
        let label = if name.is_empty() { "Transition".to_owned() } else { name };
        placed.push(((cut - span / 2.0).max(0.0), span, package, label));
    }
    placed
}

/// The Concat package nearest a CapCut transition, by the plain words in
/// its name; a dissolve for anything else, which keeps the cut's timing.
fn package_for(name: &str) -> &'static str {
    let name = name.to_lowercase();
    let has = |word: &str| name.contains(word);
    if has("black") {
        "concat.fade-black"
    } else if has("white") || has("flash") {
        "concat.fade-white"
    } else if has("blur") {
        "concat.blur-dissolve"
    } else if has("zoom") {
        "concat.zoom-transition"
    } else if has("spin") || has("rotate") {
        "concat.spin"
    } else if has("slide") {
        "concat.slide"
    } else if has("push") || has("pull") {
        "concat.push"
    } else if has("wipe") {
        "concat.linear-wipe"
    } else if has("glitch") {
        "concat.glitch-transition"
    } else {
        "concat.dissolve"
    }
}

/// A lane's clips laid so none covers another: one that would goes onto
/// a lane of its own above.
fn lanes(clips: Vec<Clip>) -> Vec<Vec<Clip>> {
    let mut lanes: Vec<Vec<Clip>> = Vec::new();
    for clip in clips {
        let fits = lanes.iter().position(|lane| {
            lane.last()
                .is_none_or(|last| clip.start >= last.start + last.duration - TOUCH)
        });
        match fits {
            Some(index) => lanes[index].push(clip),
            None => lanes.push(vec![clip]),
        }
    }
    lanes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A 9.x-shaped draft: a 9:16 timeline at 30, two cuts of one file
    /// overlapping for a transition, a photo, a title in the JSON form,
    /// music with fades and a keyframed volume, and a sticker lane.
    fn draft() -> Draft {
        let edit = json!({
            "fps": 30.0,
            "canvas_config": { "width": 1080, "height": 1920 },
            "materials": {
                "videos": [
                    { "id": "v", "type": "video", "path": "/m/a.mp4", "material_name": "a.mp4",
                      "duration": 14_000_000, "width": 1920, "height": 1080, "has_audio": true,
                      "crop": { "upper_left_x": 0.1, "upper_left_y": 0.0, "upper_right_x": 0.9,
                                "upper_right_y": 0.0, "lower_left_x": 0.1, "lower_left_y": 1.0,
                                "lower_right_x": 0.9, "lower_right_y": 1.0 } },
                    { "id": "p", "type": "photo", "path": "##_draftpath_placeholder_ABC_##/media/p.jpg",
                      "width": 800, "height": 600 }
                ],
                "audios": [{ "id": "au", "type": "music", "path": "/m/s.mp3", "name": "song",
                             "duration": 60_000_000 }],
                "texts": [{ "id": "t", "alignment": 1, "content": json!({
                    "text": "Hello there",
                    "styles": [{ "fill": { "content": { "solid": { "color": [1.0, 0.5, 0.0] } } },
                                 "range": [0, 11], "size": 15.0, "bold": true, "italic": true }]
                }).to_string() }],
                "transitions": [{ "id": "tr", "name": "Pull in", "duration": 600_000 }],
                "audio_fades": [{ "id": "f", "fade_in_duration": 1_000_000, "fade_out_duration": 2_000_000 }],
                "speeds": [
                    { "id": "s2", "type": "speed", "mode": 0, "speed": 2.0 },
                    { "id": "s3", "type": "speed", "mode": 1, "speed": 1.45, "curve_speed": {
                        "name": "Montage", "speed_points": [
                            { "x": 0.0, "y": 0.9 }, { "x": 0.5, "y": 6.8 }, { "x": 1.0, "y": 1.0 } ] } }
                ],
                "effects": [{ "id": "e", "type": "saturation" }]
            },
            "tracks": [
                { "type": "video", "segments": [
                    { "material_id": "v", "extra_material_refs": ["tr", "e", "s3"],
                      "target_timerange": { "start": 0, "duration": 4_000_000 },
                      "source_timerange": { "start": 1_000_000, "duration": 4_000_000 },
                      "clip": { "scale": { "x": 3.16, "y": 3.16 }, "transform": { "x": 0.5, "y": 0.25 },
                                "rotation": 90.0, "alpha": 0.5, "flip": { "horizontal": true, "vertical": false } } },
                    { "material_id": "v", "extra_material_refs": ["s2"],
                      "target_timerange": { "start": 3_400_000, "duration": 3_000_000 },
                      "source_timerange": { "start": 8_000_000, "duration": 6_000_000 } },
                    { "material_id": "p",
                      "target_timerange": { "start": 6_400_000, "duration": 2_000_000 } } ] },
                { "type": "text", "segments": [
                    { "material_id": "t", "target_timerange": { "start": 500_000, "duration": 2_000_000 } } ] },
                { "type": "audio", "segments": [
                    { "material_id": "au", "volume": 0.8, "extra_material_refs": ["f"],
                      "target_timerange": { "start": 0, "duration": 8_400_000 },
                      "source_timerange": { "start": 0, "duration": 8_400_000 },
                      "common_keyframes": [{ "property_type": "KFTypeVolume", "keyframe_list": [
                          { "time_offset": 0, "values": [0.0] },
                          { "time_offset": 4_200_000, "values": [1.0] } ] }] } ] },
                { "type": "sticker", "segments": [{ "material_id": "x" }] }
            ]
        });
        Draft {
            name: "Reel".into(),
            folder: "/drafts/0816".into(),
            timelines: vec![DraftTimeline {
                name: String::new(),
                edit,
            }],
        }
    }

    fn imported() -> Imported {
        import(&draft()).expect("imports")
    }

    fn clips_of(imported: &Imported, kind: ClipKind) -> Vec<Arc<Clip>> {
        imported.project.timelines[0]
            .clips
            .iter()
            .filter(|clip| clip.kind == kind)
            .cloned()
            .collect()
    }

    #[test]
    fn the_frame_rate_and_name_come_across() {
        let imported = imported();
        let settings = &imported.settings;
        assert_eq!((settings.width, settings.height), (1080, 1920));
        assert_eq!((settings.rate_num, settings.rate_den), (30, 1));
        assert_eq!(settings.name, "Reel");
        assert_eq!(rate_of(29.97), (30_000, 1001));
    }

    #[test]
    fn a_transition_overlap_becomes_a_cut_and_a_transition_clip() {
        let imported = imported();
        let videos = clips_of(&imported, ClipKind::Video);
        let (left, right) = (&videos[0], &videos[1]);
        // Overlapping 3.4s to 4.0s: the cut is at 3.7s.
        assert!((left.start + left.duration - 3.7).abs() < 1e-9);
        assert!((right.start - 3.7).abs() < 1e-9);
        // 0.3s more of the incoming clip's source is skipped, at 2x.
        assert!((right.source_start - (8.0 + 0.3 * 2.0)).abs() < 1e-9);
        assert_eq!(left.track_id, right.track_id, "end to end on one lane");
        let transitions = clips_of(&imported, ClipKind::Transition);
        assert_eq!(transitions.len(), 1);
        let transition = &transitions[0];
        assert!((transition.start - 3.4).abs() < 1e-9);
        assert!((transition.duration - 0.6).abs() < 1e-9);
        assert_ne!(transition.track_id, left.track_id, "over the cut, not on it");
    }

    #[test]
    fn place_size_turn_crop_and_speed_come_across() {
        let imported = imported();
        let videos = clips_of(&imported, ClipKind::Video);
        let first = &videos[0];
        assert!((first.scale - 3.16).abs() < 1e-9);
        assert!((first.offset_x - 0.25).abs() < 1e-9);
        assert!((first.offset_y + 0.125).abs() < 1e-9, "CapCut's y is up");
        assert!((first.rotation - 90.0).abs() < 1e-9);
        assert!((first.opacity - 0.5).abs() < 1e-9);
        assert!(first.flip_h && !first.flip_v);
        let crop = first.crop.expect("a crop");
        assert!((crop.left - 0.1).abs() < 1e-9 && (crop.right - 0.1).abs() < 1e-9);
        assert!((videos[1].speed - 2.0).abs() < 1e-9);
        let curve = first.speed_curve.as_ref().expect("CapCut's curve");
        assert_eq!(curve.len(), 3);
        assert!((curve[1].speed - 6.8).abs() < 1e-9 && (curve[1].at - 0.5).abs() < 1e-9);
        assert!((first.speed - crate::speed::mean_of(curve)).abs() < 1e-9);
        assert_eq!(imported.project.media.len(), 3, "one file, two cuts: one item");
    }

    #[test]
    fn a_copied_file_is_found_in_the_draft() {
        let imported = imported();
        let photo = clips_of(&imported, ClipKind::Image).remove(0);
        let media = imported.project.media_by_id(&photo.media_id).expect("media");
        assert_eq!(media.path, "/drafts/0816/media/p.jpg");
        assert_eq!(media.kind, MediaKind::Image);
    }

    #[test]
    fn titles_keep_their_words_and_look() {
        let imported = imported();
        let title = clips_of(&imported, ClipKind::Text).remove(0);
        let style = title.text.as_ref().expect("a title");
        assert_eq!(style.content, "Hello there");
        assert_eq!(style.color, "#ff8000");
        assert!(style.italic);
        assert!((style.font_weight - 700.0).abs() < 1e-9);
        assert!((style.font_size - 0.09).abs() < 1e-9);
        assert_eq!(unmarked("<size=15><font id=\"\">[hi you]</font></size>"), "hi you");
    }

    #[test]
    fn sound_keeps_its_fades_volume_and_keys() {
        let imported = imported();
        let music = clips_of(&imported, ClipKind::Audio).remove(0);
        assert!((music.fade_in - 1.0).abs() < 1e-9 && (music.fade_out - 2.0).abs() < 1e-9);
        assert!((music.volume - 0.8).abs() < 1e-9);
        assert_eq!(music.keys.len(), 2);
        assert_eq!(music.keys[1].property, KeyProperty::Volume);
        assert!((music.keys[1].at - 0.5).abs() < 1e-9);
    }

    #[test]
    fn what_does_not_come_across_is_said() {
        let notes = imported().notes;
        assert!(notes.iter().any(|note| note.contains("sticker")), "{notes:?}");
        assert!(notes.iter().any(|note| note.contains("effect")), "{notes:?}");
    }

    #[test]
    fn the_project_saves_and_loads_whole() {
        let imported = imported();
        let document = crate::doc::to_document(&imported.settings, &imported.project);
        let read = crate::doc::from_document(&document).expect("loads");
        assert_eq!(
            read.active().clips.len(),
            imported.project.active().clips.len()
        );
        for timeline in &read.timelines {
            for track in &timeline.tracks {
                let mut lane: Vec<_> = timeline
                    .clips
                    .iter()
                    .filter(|clip| clip.track_id == track.id)
                    .collect();
                lane.sort_by(|a, b| a.start.total_cmp(&b.start));
                for pair in lane.windows(2) {
                    assert!(pair[1].start >= pair[0].start + pair[0].duration - TOUCH);
                }
            }
        }
    }

    #[test]
    fn a_draft_with_nothing_readable_is_refused() {
        let draft = Draft {
            name: String::new(),
            folder: String::new(),
            timelines: vec![DraftTimeline {
                name: String::new(),
                edit: json!({ "tracks": [{ "type": "sticker", "segments": [] }] }),
            }],
        };
        assert_eq!(import(&draft).err(), Some(CapcutError::Empty));
    }
}
