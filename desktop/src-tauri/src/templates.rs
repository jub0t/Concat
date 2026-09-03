// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Template bundles on disk.
//!
//! A template is a folder in the app config directory's `templates/`:
//!
//! - `template.json` - an ordinary wolfcut document whose placeholder media
//!   (engine `placeholder: true`) have blank paths, and whose bundled media
//!   and fonts - the music, overlays and faces that are part of the design -
//!   have paths relative to the bundle (`assets/...`).
//! - `assets/` - those bundled files, copied in at save time.
//! - `poster.jpg` - a frame of the edit, for the gallery card.
//!
//! Saving packs the open project into a bundle; instantiating unpacks one
//! into a fresh project folder and fills every slot with the user's media
//! *through the engine* (`Command::FillSlot` in a batch), so what a fill
//! means is defined in exactly one place - `concat-project` - and the editor
//! never opens on a slot with a dead path.

use std::path::{Path, PathBuf};

use concat_project::{Command, DocumentSettings, Editor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projects;

const MANIFEST: &str = "template.json";
const ASSETS: &str = "assets";
const POSTER: &str = "poster.jpg";

/// One template, as the gallery sees it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// The bundle folder.
    pub path: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rate_num: i64,
    pub rate_den: i64,
    /// The slots to fill, in the order they first appear on the timeline.
    pub slots: Vec<SlotInfo>,
    pub has_poster: bool,
}

/// One placeholder the user's media will replace.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub media_id: String,
    pub name: String,
    /// "video", "audio" or "image".
    pub kind: String,
    /// Timeline seconds this slot covers, summed over its clips.
    pub seconds: f64,
}

/// The user's media for one slot, straight from a probe.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlotFill {
    pub media_id: String,
    pub item: concat_project::commands::NewMedia,
}

pub fn templates_dir(config: &Path) -> PathBuf {
    config.join("templates")
}

/// Packs the given document into a new template bundle.
///
/// `project_path` is where the document's media paths currently resolve; it
/// also donates the poster frame. Refuses a name that already exists rather
/// than overwriting someone's work.
pub fn save(
    config: &Path,
    document: &Value,
    settings: &concat_project::DocumentSettings,
    project_path: &str,
    name: &str,
) -> Result<TemplateInfo, String> {
    let root = templates_dir(config).join(projects::folder_name(name));
    if root.join(MANIFEST).exists() {
        return Err(format!("a template named {name:?} already exists"));
    }
    let assets = root.join(ASSETS);
    std::fs::create_dir_all(&assets)
        .map_err(|error| format!("could not create {}: {error}", assets.display()))?;

    // Typed through the engine's reader and writer, not hand-edited JSON:
    // the manifest is written by the same code path every save uses, and a
    // schema change breaks this at compile time.
    let Some(mut project) = concat_project::from_document(document) else {
        return Err("the open project is not a document this build can read".to_owned());
    };

    // Placeholder media loses its path - it was the creator's own footage,
    // standing in. Everything else is part of the design and ships in the
    // bundle, referenced relative to it.
    for item in &mut project.media {
        if item.placeholder {
            item.path = String::new();
            continue;
        }
        item.path = bundle_file(&assets, &item.id.clone(), &item.path.clone())?;
    }
    for (index, font) in project.fonts.iter_mut().enumerate() {
        font.path = bundle_file(&assets, &format!("font{index}"), &font.path.clone())?;
    }

    let mut settings = settings.clone();
    settings.name = name.to_owned();
    let document = concat_project::to_document(&settings, &project);

    let encoded = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("could not encode the template: {error}"))?;
    std::fs::write(root.join(MANIFEST), encoded)
        .map_err(|error| format!("could not write the template: {error}"))?;

    // Best effort: a template without a poster is still a template.
    if let Ok(bytes) = crate::poster_frame(project_path) {
        let _ = std::fs::write(root.join(POSTER), bytes);
    }

    read_info(&root)
}

/// Copies one source file into `assets/` and returns its bundle-relative
/// path. The bundle must be self-contained, so a missing source is an error,
/// not a warning - a template that cannot find its own music is not one.
fn bundle_file(assets: &Path, id: &str, source: &str) -> Result<String, String> {
    if source.is_empty() {
        return Err(
            "a media item has no file behind it; fill or remove it before saving a template"
                .to_owned(),
        );
    }
    let base = Path::new(source)
        .file_name()
        .and_then(|name| name.to_str())
        .map(projects::folder_name)
        .unwrap_or_else(|| "file".to_owned());
    let name = format!("{}-{base}", projects::folder_name(id));
    let destination = assets.join(&name);
    std::fs::copy(source, &destination)
        .map_err(|error| format!("could not bundle {source}: {error}"))?;
    Ok(format!("{ASSETS}/{name}"))
}

/// Every template in the library, in name order.
///
/// A folder that does not parse is skipped rather than sinking the whole
/// gallery - the same grace the recents list extends to vanished projects.
pub fn list(config: &Path) -> Vec<TemplateInfo> {
    let Ok(entries) = std::fs::read_dir(templates_dir(config)) else {
        return Vec::new();
    };
    let mut templates: Vec<TemplateInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| read_info(&entry.path()).ok())
        .collect();
    templates.sort_by(|left, right| left.name.cmp(&right.name));
    templates
}

/// Reads one bundle's manifest into gallery form.
fn read_info(root: &Path) -> Result<TemplateInfo, String> {
    let manifest = root.join(MANIFEST);
    let bytes = std::fs::read(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a template: {error}", manifest.display()))?;

    let video = document.get("video").ok_or("the template names no output format")?;
    let dimension = |field: &str| {
        video
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("the template has no usable {field}"))
    };
    let rate = |field: &str| {
        video
            .get(field)
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("the template has no usable {field}"))
    };

    Ok(TemplateInfo {
        path: root.to_string_lossy().into_owned(),
        name: document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Template")
            .to_owned(),
        width: dimension("width")?,
        height: dimension("height")?,
        rate_num: rate("rateNum")?,
        rate_den: rate("rateDen")?,
        slots: slots_of(&document),
        has_poster: root.join(POSTER).exists(),
    })
}

/// The placeholder media of a document, ordered by when each first appears
/// on the active timeline - the order a user fills them in.
///
/// Typed through the engine's own reader: a schema change breaks this at
/// compile time, not as a gallery that silently shows no slots.
fn slots_of(document: &Value) -> Vec<SlotInfo> {
    let Some(project) = concat_project::from_document(document) else {
        return Vec::new();
    };
    let clips = project
        .timelines
        .iter()
        .find(|timeline| timeline.id == project.active_timeline_id)
        .or_else(|| project.timelines.first())
        .map(|timeline| timeline.clips.as_slice())
        .unwrap_or(&[]);

    let mut slots: Vec<(f64, SlotInfo)> = Vec::new();
    for item in &project.media {
        if !item.placeholder {
            continue;
        }

        let mut first = f64::MAX;
        let mut seconds = 0.0;
        for clip in clips.iter().filter(|clip| clip.media_id == item.id) {
            first = first.min(clip.start);
            seconds += clip.duration;
        }

        slots.push((
            first,
            SlotInfo {
                media_id: item.id.clone(),
                name: item.name.clone(),
                kind: match item.kind {
                    concat_project::model::MediaKind::Video => "video",
                    concat_project::model::MediaKind::Audio => "audio",
                    concat_project::model::MediaKind::Image => "image",
                }
                .to_owned(),
                seconds,
            },
        ));
    }
    slots.sort_by(|(left, _), (right, _)| left.total_cmp(right));
    slots.into_iter().map(|(_, slot)| slot).collect()
}

/// Unpacks a template into a fresh project with every slot filled.
///
/// The fills go through the engine as one `Batch` of `FillSlot` commands, so
/// the semantics live in `concat-project` and a half-fillable set of media
/// leaves no half-made project behind.
pub fn instantiate(
    template: &str,
    location: &str,
    name: &str,
    fills: Vec<SlotFill>,
) -> Result<projects::ProjectInfo, String> {
    let bundle = PathBuf::from(template);
    let manifest = bundle.join(MANIFEST);
    let bytes = std::fs::read(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let mut document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a template: {error}", manifest.display()))?;
    let info = read_info(&bundle)?;

    let unfilled: Vec<&SlotInfo> = info
        .slots
        .iter()
        .filter(|slot| !fills.iter().any(|fill| fill.media_id == slot.media_id))
        .collect();
    if !unfilled.is_empty() {
        return Err(format!("{} slot(s) still need a clip", unfilled.len()));
    }

    let project =
        projects::create(location, name, info.width, info.height, info.rate_num, info.rate_den)?;
    let root = PathBuf::from(&project.path);

    // The bundle's assets move into the project so it stands alone; the
    // document's relative paths become project-local absolute ones.
    copy_assets(&bundle.join(ASSETS), &root.join(ASSETS))?;
    document["name"] = Value::String(project.name.clone());
    for section in ["media", "fonts"] {
        if let Some(items) = document.get_mut(section).and_then(Value::as_array_mut) {
            for item in items {
                let Some(path) = item.get("path").and_then(Value::as_str) else { continue };
                if let Some(relative) = path.strip_prefix(&format!("{ASSETS}/")) {
                    let absolute = root.join(ASSETS).join(relative);
                    item["path"] = Value::String(absolute.to_string_lossy().into_owned());
                }
            }
        }
    }

    let mut editor = Editor::from_document(&document)
        .ok_or("this template holds a document this build cannot read")?;
    if !fills.is_empty() {
        editor
            .apply(Command::Batch {
                commands: fills
                    .into_iter()
                    .map(|fill| Command::FillSlot { media_id: fill.media_id, item: fill.item })
                    .collect(),
            })
            .map_err(|error| format!("could not fill the template: {error}"))?;
    }

    let settings = DocumentSettings {
        name: project.name.clone(),
        width: info.width,
        height: info.height,
        rate_num: info.rate_num,
        rate_den: info.rate_den,
    };
    projects::save(&project.path, &editor.to_document(&settings))?;

    // After the save, so the cached poster reads as fresh.
    if bundle.join(POSTER).is_file() {
        let cache = root.join("cache");
        let _ = std::fs::create_dir_all(&cache);
        let _ = std::fs::copy(bundle.join(POSTER), cache.join("preview.jpg"));
    }

    Ok(project)
}

fn copy_assets(from: &Path, to: &Path) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(()); // a template with no bundled assets
    };
    std::fs::create_dir_all(to)
        .map_err(|error| format!("could not create {}: {error}", to.display()))?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let source = entry.path();
        if source.is_file() {
            let destination = to.join(entry.file_name());
            std::fs::copy(&source, &destination).map_err(|error| {
                format!("could not copy {}: {error}", source.display())
            })?;
        }
    }
    Ok(())
}

/// Removes one template bundle for good.
///
/// Only ever a folder inside the templates directory that actually holds a
/// manifest - this function deletes recursively, and a stray path here would
/// be a catastrophe, so it is checked rather than trusted.
pub fn delete(config: &Path, path: &str) -> Result<(), String> {
    let target = PathBuf::from(path);
    let library = templates_dir(config);
    let (Ok(target), Ok(library)) = (target.canonicalize(), library.canonicalize()) else {
        return Err(format!("{path} is not a template"));
    };
    if !target.starts_with(&library) || target == library || !target.join(MANIFEST).is_file() {
        return Err(format!("{path} is not a template"));
    }
    std::fs::remove_dir_all(&target)
        .map_err(|error| format!("could not delete {}: {error}", target.display()))
}

/// The poster bytes for one bundle, or an error the UI treats as "no art".
pub fn poster(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(Path::new(path).join(POSTER))
        .map_err(|error| format!("no poster for {path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn template_document() -> Value {
        json!({
            "wolfcut": "0.1.0", "version": 1, "name": "Beat Intro",
            "video": { "width": 1920, "height": 1080, "rateNum": 30, "rateDen": 1 },
            "media": [
                { "id": "m1", "path": "", "name": "A-roll", "duration": 4.0, "kind": "video",
                  "hasAudio": true, "placeholder": true },
                { "id": "m2", "path": "", "name": "B-roll", "duration": 2.0, "kind": "video",
                  "hasAudio": false, "placeholder": true }
            ],
            "tracks": [{ "id": "T1", "name": "Track 1", "visible": true, "muted": false }],
            "clips": [
                { "id": "c2", "trackId": "T1", "mediaId": "m2", "name": "B-roll",
                  "kind": "video", "start": 4.0, "duration": 2.0, "sourceStart": 0.0,
                  "volume": 1.0, "fadeIn": 0.0, "fadeOut": 0.0, "scale": 1.0,
                  "offsetX": 0.0, "offsetY": 0.0, "rotation": 0.0, "opacity": 1.0,
                  "speed": 1.0, "preservePitch": true, "filters": [], "videoEffects": [] },
                { "id": "c1", "trackId": "T1", "mediaId": "m1", "name": "A-roll",
                  "kind": "video", "start": 0.0, "duration": 4.0, "sourceStart": 1.0,
                  "volume": 1.0, "fadeIn": 0.0, "fadeOut": 0.0, "scale": 1.0,
                  "offsetX": 0.0, "offsetY": 0.0, "rotation": 0.0, "opacity": 1.0,
                  "speed": 1.0, "preservePitch": true, "filters": [], "videoEffects": [] }
            ]
        })
    }

    #[test]
    fn slots_come_back_in_timeline_order() {
        // c2 is listed first but starts later; the fill order is by time.
        let slots = slots_of(&template_document());
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].media_id, "m1");
        assert_eq!(slots[0].seconds, 4.0);
        assert_eq!(slots[1].media_id, "m2");
    }

    #[test]
    fn saving_packs_a_bundle_and_it_round_trips() {
        let scratch = std::env::temp_dir().join("wolfcut-template-save-test");
        let _ = std::fs::remove_dir_all(&scratch);
        let config = scratch.join("config");
        let project = scratch.join("project");
        std::fs::create_dir_all(&project).expect("scratch dirs");

        // A real file for the bundled music; the placeholder needs none.
        let music = project.join("music.mp3");
        std::fs::write(&music, b"not really an mp3").expect("writes");

        let mut document = template_document();
        {
            let media = document["media"].as_array_mut().expect("media");
            // m1 stays a placeholder; m2 becomes ordinary bundled media.
            media[1]["placeholder"] = Value::Bool(false);
            media[1]["path"] = Value::String(music.to_string_lossy().into_owned());
            media[1]["kind"] = Value::String("audio".to_owned());
        }

        let settings = concat_project::DocumentSettings {
            name: "Beat Intro".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
        };
        let info = save(&config, &document, &settings, &project.to_string_lossy(), "Beat Intro")
            .expect("saves");
        assert_eq!(info.name, "Beat Intro");
        assert_eq!(info.slots.len(), 1, "only the placeholder is a slot");
        assert_eq!(info.slots[0].media_id, "m1");

        // The same name again is refused, not overwritten.
        assert!(
            save(&config, &document, &settings, &project.to_string_lossy(), "Beat Intro")
                .is_err()
        );

        let bundle = PathBuf::from(&info.path);
        let packed: Value = serde_json::from_slice(
            &std::fs::read(bundle.join(MANIFEST)).expect("manifest"),
        )
        .expect("parses");
        let media = packed["media"].as_array().expect("media");
        assert_eq!(media[0]["path"], "", "the placeholder's path is blanked");
        let bundled = media[1]["path"].as_str().expect("path");
        assert!(bundled.starts_with("assets/"), "bundled media went relative: {bundled}");
        assert!(bundle.join(bundled).is_file(), "and the file really is in the bundle");

        assert_eq!(list(&config).len(), 1, "the library sees it");

        // And it instantiates: the asset lands in the project, absolute.
        let location = scratch.join("projects");
        std::fs::create_dir_all(&location).expect("scratch dir");
        let made = instantiate(
            &info.path,
            &location.to_string_lossy(),
            "From bundle",
            vec![SlotFill {
                media_id: "m1".to_owned(),
                item: concat_project::commands::NewMedia {
                    path: "/mine/clip.mp4".to_owned(),
                    name: "clip.mp4".to_owned(),
                    duration: Some(6.0),
                    kind: concat_project::model::MediaKind::Video,
                    width: Some(1920),
                    height: Some(1080),
                    frame_rate: Some(30.0),
                    frame_rate_fraction: Some("30/1".to_owned()),
                    video_codec: Some("h264".to_owned()),
                    audio_codec: None,
                    has_audio: false,
                },
            }],
        )
        .expect("instantiates");

        let opened = projects::read_document(&made.path).expect("reads back");
        let media = opened["media"].as_array().expect("media");
        let sound = media[1]["path"].as_str().expect("path");
        assert!(
            Path::new(sound).is_absolute() && Path::new(sound).is_file(),
            "the bundled asset was copied into the project and repointed: {sound}"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn instantiating_fills_through_the_engine() {
        let scratch = std::env::temp_dir().join("wolfcut-template-test");
        let _ = std::fs::remove_dir_all(&scratch);
        let bundle = scratch.join("templates").join("Beat Intro");
        std::fs::create_dir_all(&bundle).expect("scratch dir");
        std::fs::write(
            bundle.join(MANIFEST),
            serde_json::to_vec(&template_document()).expect("encodes"),
        )
        .expect("writes");

        let fill = |media_id: &str, path: &str| SlotFill {
            media_id: media_id.to_owned(),
            item: concat_project::commands::NewMedia {
                path: path.to_owned(),
                name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                duration: Some(9.0),
                kind: concat_project::model::MediaKind::Video,
                width: Some(1920),
                height: Some(1080),
                frame_rate: Some(30.0),
                frame_rate_fraction: Some("30/1".to_owned()),
                video_codec: Some("h264".to_owned()),
                audio_codec: None,
                has_audio: false,
            },
        };

        let projects_dir = scratch.join("projects");
        std::fs::create_dir_all(&projects_dir).expect("scratch dir");
        let location = projects_dir.to_string_lossy().into_owned();

        // Missing a slot is refused before any folder is written.
        let refused = instantiate(
            &bundle.to_string_lossy(),
            &location,
            "Missing",
            vec![fill("m1", "/mine/a.mp4")],
        );
        assert!(refused.unwrap_err().contains("slot"));

        let project = instantiate(
            &bundle.to_string_lossy(),
            &location,
            "My edit",
            vec![fill("m1", "/mine/a.mp4"), fill("m2", "/mine/b.mp4")],
        )
        .expect("instantiates");
        assert_eq!((project.width, project.height), (1920, 1080));

        let document = projects::read_document(&project.path).expect("reads back");
        let media = document["media"].as_array().expect("media");
        assert!(media.iter().all(|item| {
            !item.get("placeholder").and_then(Value::as_bool).unwrap_or(false)
        }));
        assert_eq!(media[0]["path"], "/mine/a.mp4");
        // The slot kept the template's timing; the in-point reset (c1 had 1.0).
        let clips = document["clips"].as_array().expect("clips");
        let c1 = clips.iter().find(|clip| clip["id"] == "c1").expect("c1");
        assert_eq!(c1["start"], 0.0);
        assert_eq!(c1["duration"], 4.0);
        assert_eq!(c1["sourceStart"], 0.0);

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
