// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Template bundles on disk.
//!
//! A template is a folder in the app config directory's `templates/`:
//!
//! - `template.json` - the bundle's name and frame, as a project's manifest
//!   holds them.
//! - `template.concat` - an ordinary project document whose placeholder
//!   media have blank paths, and whose bundled media and fonts - the music,
//!   overlays and faces that are part of the design - have paths relative
//!   to the bundle (`assets/...`).
//! - `assets/` - those bundled files, copied in at save time.
//! - `poster.jpg` - a frame of the edit, for the gallery card.
//!
//! Saving packs the open project into a bundle; instantiating unpacks one
//! into a fresh project folder and fills every slot with the user's media
//! through the document's own command (`FillSlot` in a batch), so what a
//! fill means is defined in one place, and the editor never opens on a
//! slot with a dead path.

use std::path::{Path, PathBuf};

use concat_doc::commands::{self, Command};
use concat_doc::model::{Frame, Media, MediaId, Model};
use concat_doc::{Document, Origin};
use serde::{Deserialize, Serialize};

use crate::{media, projects};

const MANIFEST: &str = "template.json";
const DOCUMENT: &str = "template.concat";
const ASSETS: &str = "assets";
const POSTER: &str = "poster.jpg";

/// One template, as the gallery sees it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// The bundle folder.
    pub path: String,
    /// The template's name.
    pub name: String,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub rate_num: i64,
    /// Frame rate denominator.
    pub rate_den: i64,
    /// The slots to fill, in the order they first appear on the timeline.
    pub slots: Vec<SlotInfo>,
    /// Whether the bundle carries a poster.
    pub has_poster: bool,
}

/// One placeholder the user's media will replace.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    /// The placeholder file's id in the template document.
    pub media_id: String,
    /// The placeholder's name, as the creator labelled it.
    pub name: String,
    /// What kind of file the slot wants.
    pub kind: media::FileKind,
    /// Timeline seconds this slot covers, summed over its clips.
    pub seconds: f64,
}

/// The user's file for one slot, straight from a probe.
#[derive(Clone, Debug)]
pub struct SlotFill {
    /// Which slot.
    pub media_id: MediaId,
    /// What fills it.
    pub item: Media,
}

/// A bundle's name and frame.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    name: String,
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
}

/// Where the library lives under the config directory.
pub fn templates_dir(config: &Path) -> PathBuf {
    config.join("templates")
}

/// Packs the given document into a new template bundle, in `frame`.
///
/// `project_path` is where the document's media paths currently resolve; it
/// also donates the poster frame. Refuses a name that already exists rather
/// than overwriting someone's work.
pub fn save(
    config: &Path,
    document: &Document<Model>,
    frame: &Frame,
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

    // Placeholder media loses its path - it was the creator's own footage,
    // standing in. Everything else is part of the design and ships in the
    // bundle, referenced relative to it.
    let mut draft = document.begin("Pack template", Origin(0));
    let ids: Vec<MediaId> = draft.media.ids().collect();
    for id in ids {
        let Some(item) = draft.media.get(id).cloned() else {
            continue;
        };
        let path = if item.placeholder {
            String::new()
        } else {
            bundle_file(&assets, &id.to_string(), &item.path)?
        };
        draft
            .media
            .update(id, |item| item.path = path)
            .map_err(|error| error.to_string())?;
    }
    let mut fonts = draft.fonts.clone();
    for (index, font) in fonts.iter_mut().enumerate() {
        font.path = bundle_file(&assets, &format!("font{index}"), &font.path)?;
    }
    draft.fonts = fonts;
    let packed = draft
        .commit()
        .map_err(|error| format!("the template could not be packed: {error}"))?
        .into_document();

    write_manifest(
        &root,
        &Manifest {
            name: name.to_owned(),
            width: frame.width.get(),
            height: frame.height.get(),
            rate_num: i64::from(frame.rate.num()),
            rate_den: i64::from(frame.rate.den()),
        },
    )?;
    concat_doc::save::save_document(&root.join(DOCUMENT), &packed)
        .map_err(|error| format!("could not write the template: {error}"))?;

    // Best effort: a template without a poster is still a template.
    if let Ok(bytes) = media::poster_frame(project_path) {
        let _ = std::fs::write(root.join(POSTER), bytes);
    }

    read_info(&root)
}

fn write_manifest(root: &Path, manifest: &Manifest) -> Result<(), String> {
    let encoded = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("could not encode the template: {error}"))?;
    std::fs::write(root.join(MANIFEST), encoded)
        .map_err(|error| format!("could not write the template: {error}"))
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

/// A bundle's document.
fn read_document(root: &Path) -> Result<Document<Model>, String> {
    let file = root.join(DOCUMENT);
    let bytes = std::fs::read(&file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    concat_doc::file::read(&bytes)
        .map(|opened| opened.document)
        .map_err(|error| format!("{} is not a template: {error}", file.display()))
}

/// Reads one bundle's manifest into gallery form.
fn read_info(root: &Path) -> Result<TemplateInfo, String> {
    let manifest = root.join(MANIFEST);
    let bytes = std::fs::read(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a template: {error}", manifest.display()))?;
    if manifest.width == 0
        || manifest.height == 0
        || manifest.rate_num <= 0
        || manifest.rate_den <= 0
    {
        return Err("the template names no usable output format".to_owned());
    }
    let document = read_document(root)?;
    Ok(TemplateInfo {
        path: root.to_string_lossy().into_owned(),
        name: manifest.name,
        width: manifest.width,
        height: manifest.height,
        rate_num: manifest.rate_num,
        rate_den: manifest.rate_den,
        slots: slots_of(&document),
        has_poster: root.join(POSTER).exists(),
    })
}

/// The placeholder media of a document, ordered by when each first appears
/// on the sequence showing - the order a user fills them in.
fn slots_of(document: &Model) -> Vec<SlotInfo> {
    let showing = document.view.showing;
    let mut slots: Vec<(concat_doc::Tick, SlotInfo)> = Vec::new();
    for (id, item) in document.media.iter() {
        if !item.placeholder {
            continue;
        }
        let mut first = concat_doc::Tick::MAX;
        let mut seconds = 0.0;
        for clip in document.items.index().using_media(id) {
            let Some(clip) = document.items.get(clip) else {
                continue;
            };
            let on_showing = document
                .lanes
                .get(clip.lane)
                .is_some_and(|lane| Some(lane.sequence) == showing);
            if !on_showing {
                continue;
            }
            first = first.min(clip.start);
            seconds += concat_doc::TickDur::from(clip.len).display_seconds();
        }
        let kind = match item.kind {
            concat_doc::model::MediaKind::Audio => media::FileKind::Audio,
            concat_doc::model::MediaKind::Image => media::FileKind::Image,
            _ => media::FileKind::Video,
        };
        slots.push((
            first,
            SlotInfo {
                media_id: id.to_string(),
                name: item.name.clone(),
                kind,
                seconds,
            },
        ));
    }
    slots.sort_by_key(|(first, _)| *first);
    slots.into_iter().map(|(_, slot)| slot).collect()
}

/// Unpacks a template into a fresh project with every slot filled.
///
/// The fills are one `Batch` of `FillSlot` commands, so the meaning lives
/// with the document and a half-fillable set of media leaves no half-made
/// project behind.
pub fn instantiate(
    template: &str,
    location: &str,
    name: &str,
    fills: Vec<SlotFill>,
) -> Result<projects::ProjectInfo, String> {
    let bundle = PathBuf::from(template);
    let info = read_info(&bundle)?;
    let document = read_document(&bundle)?;

    let unfilled = info
        .slots
        .iter()
        .filter(|slot| {
            !fills
                .iter()
                .any(|fill| fill.media_id.to_string() == slot.media_id)
        })
        .count();
    if unfilled > 0 {
        return Err(format!("{unfilled} slot(s) still need a clip"));
    }

    let project = projects::create(
        location,
        name,
        info.width,
        info.height,
        info.rate_num,
        info.rate_den,
        [0, 0, 0, 255],
    )?;
    let root = PathBuf::from(&project.path);

    // The bundle's assets move into the project so it stands alone; the
    // document's relative paths become project-local absolute ones.
    copy_assets(&bundle.join(ASSETS), &root.join(ASSETS))?;
    let local = |path: &str| -> Option<String> {
        let relative = path.strip_prefix(&format!("{ASSETS}/"))?;
        Some(
            root.join(ASSETS)
                .join(relative)
                .to_string_lossy()
                .into_owned(),
        )
    };
    let mut draft = document.begin("Unpack template", Origin(0));
    let ids: Vec<MediaId> = draft.media.ids().collect();
    for id in ids {
        let moved = draft.media.get(id).and_then(|item| local(&item.path));
        if let Some(path) = moved {
            draft
                .media
                .update(id, |item| item.path = path)
                .map_err(|error| error.to_string())?;
        }
    }
    let mut fonts = draft.fonts.clone();
    for font in &mut fonts {
        if let Some(path) = local(&font.path) {
            font.path = path;
        }
    }
    draft.fonts = fonts;
    let mut document = draft
        .commit()
        .map_err(|error| format!("the template could not be unpacked: {error}"))?
        .into_document();
    // A template saved before an effect was retired opens the way a
    // project does: its links read as links to the stand-ins.
    document = crate::session::upgrade_links(document).0;
    if !fills.is_empty() {
        let fill = Command::Batch {
            commands: fills
                .into_iter()
                .map(|fill| Command::FillSlot {
                    media: fill.media_id,
                    file: fill.item,
                })
                .collect(),
        };
        document = commands::made(&document, &fill)
            .map_err(|error| format!("could not fill the template: {error}"))?
            .0;
    }
    concat_doc::save::save_document(&crate::session::document_path(&root), &document)
        .map_err(|error| format!("could not save the new project: {error}"))?;

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
            std::fs::copy(&source, &destination)
                .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
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

/// The poster bytes for one bundle, or an error the caller treats as "no art".
pub fn poster(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(Path::new(path).join(POSTER))
        .map_err(|error| format!("no poster for {path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doc::{Doc, at, video};
    use concat_doc::model::{ItemId, MediaKind};

    /// A document with two slots: the A-roll at 0 for 4 s, the B-roll after
    /// it, the B-roll added to the bin first.
    fn template_document() -> (Doc, MediaId, MediaId) {
        let mut doc = Doc::new();
        let slot = |doc: &mut Doc, name: &str, seconds: f64| {
            let mut file = video("", seconds);
            file.name = name.to_owned();
            file.placeholder = true;
            doc.media(file)
        };
        let b = slot(&mut doc, "B-roll", 2.0);
        let a = slot(&mut doc, "A-roll", 4.0);
        doc.apply(Command::AddClipAtFirstFree {
            media: a,
            start: at(0.0),
        });
        doc.apply(Command::AddClipAtFirstFree {
            media: b,
            start: at(4.0),
        });
        (doc, a, b)
    }

    fn fill(media_id: MediaId, path: &str, seconds: f64) -> SlotFill {
        SlotFill {
            media_id,
            item: video(path, seconds),
        }
    }

    fn hd() -> Frame {
        Frame::hd()
    }

    #[test]
    fn slots_come_back_in_timeline_order() {
        let (doc, a, b) = template_document();
        let slots = slots_of(doc.model());
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].media_id, a.to_string());
        assert_eq!(slots[0].seconds, 4.0);
        assert_eq!(slots[1].media_id, b.to_string());
    }

    #[test]
    fn saving_packs_a_bundle_and_instantiating_fills_it() {
        let scratch = tempfile::tempdir().expect("scratch");
        let config = scratch.path().join("config");
        let project = scratch.path().join("project");
        std::fs::create_dir_all(&project).expect("scratch dirs");
        // A real file for the bundled music; the placeholder needs none.
        let music = project.join("music.mp3");
        std::fs::write(&music, b"not really an mp3").expect("writes");

        let (mut doc, a, b) = template_document();
        doc.apply(Command::SetMediaPlaceholder {
            media: b,
            placeholder: false,
        });
        doc.apply(Command::UpdateMediaPath {
            media: b,
            path: music.to_string_lossy().into_owned(),
        });

        let info = save(
            &config,
            &doc.0,
            &hd(),
            &project.to_string_lossy(),
            "Beat Intro",
        )
        .expect("saves");
        assert_eq!(info.name, "Beat Intro");
        assert_eq!(info.slots.len(), 1, "only the placeholder is a slot");
        assert_eq!(info.slots[0].media_id, a.to_string());
        assert!(
            save(
                &config,
                &doc.0,
                &hd(),
                &project.to_string_lossy(),
                "Beat Intro"
            )
            .is_err(),
            "the same name again is refused, not overwritten"
        );
        let bundle = PathBuf::from(&info.path);
        let packed = read_document(&bundle).expect("reads");
        assert_eq!(packed.media.get(a).expect("a").path, "", "blanked");
        let bundled = packed.media.get(b).expect("b").path.clone();
        assert!(bundled.starts_with("assets/") && bundle.join(&bundled).is_file());
        assert_eq!(list(&config).len(), 1, "the library sees it");

        let location = scratch.path().join("projects");
        std::fs::create_dir_all(&location).expect("scratch dir");
        let refused = instantiate(&info.path, &location.to_string_lossy(), "None", Vec::new());
        assert!(refused.unwrap_err().contains("slot"));
        let made = instantiate(
            &info.path,
            &location.to_string_lossy(),
            "From bundle",
            vec![fill(a, "/mine/clip.mp4", 6.0)],
        )
        .expect("instantiates");
        let opened = crate::session::read_document(Path::new(&made.path))
            .expect("reads back")
            .expect("a document");
        let sound = &opened.media.get(b).expect("b").path;
        assert!(
            Path::new(sound).is_absolute() && Path::new(sound).is_file(),
            "{sound}"
        );
        let filled = opened.media.get(a).expect("a");
        assert!(!filled.placeholder && filled.path == "/mine/clip.mp4");
        assert_eq!(filled.kind, MediaKind::Video);
        // The slot's clip kept its place and length.
        let clip: ItemId = opened.items.index().using_media(a).next().expect("a clip");
        let clip = opened.items.get(clip).expect("there");
        assert_eq!(clip.start, at(0.0));
    }
}
