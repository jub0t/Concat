// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Importing a CapCut draft as a new Concat project (#477).
//!
//! Where CapCut keeps its drafts, what makes a folder one, reading a
//! draft's timelines, what they become as a Concat document, and writing
//! that into a project folder of its own. The draft is only ever read. A CapCut folder may be synced to CapCut's cloud and is
//! CapCut's to manage, so nothing of Concat's is written into it.

use std::path::{Path, PathBuf};

use concat_doc::commands::{self, Applied, Command};
use concat_doc::model::{
    Animated, Content, Frame, Item, ItemId, LaneId, Media, MediaCut, MediaId, MediaKind, Model,
    SequenceId, TimeMap, Title,
};
use concat_doc::{Document, Tick, TickLen};
use serde_json::Value;

use crate::projects::{self, ProjectInfo};

/// The files a CapCut draft keeps its edit in, newest name first.
pub const DRAFT_FILES: &[&str] = &["draft_content.json", "draft_info.json"];

/// One timeline of a draft: its name and its edit, as CapCut wrote it.
#[derive(Clone, Debug)]
pub struct DraftTimeline {
    /// The name CapCut shows; empty for a draft of one.
    pub name: String,
    /// The edit.
    pub edit: Value,
}

/// A CapCut draft, read.
#[derive(Clone, Debug)]
pub struct Draft {
    /// The draft's name.
    pub name: String,
    /// The draft's folder, which a file's path in it may be relative to.
    pub folder: String,
    /// Its timelines, the one CapCut opens first first.
    pub timelines: Vec<DraftTimeline>,
}

/// What a draft became: the document, its name and frame, and a line for
/// each kind of thing left out.
struct Imported {
    document: Document<Model>,
    name: String,
    frame: Frame,
    notes: Vec<String>,
}

/// Microseconds, CapCut's unit of time, as ticks.
fn micros(value: Option<&Value>) -> Option<i64> {
    let us = value?
        .as_i64()
        .or_else(|| value?.as_f64().map(|v| v as i64))?;
    concat_doc::rescale(us, concat_doc::TICKS_PER_SECOND, 1_000_000)
}

/// A number of the draft's.
fn number(value: &Value, path: &[&str]) -> Option<f64> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_f64()
}

/// A file's path in a draft: CapCut writes the draft's own folder as a
/// placeholder in front of what it keeps inside it.
fn file_path(path: &str, folder: &str) -> String {
    match path.find("##_draftpath_placeholder_") {
        Some(at) => {
            let rest = &path[at..];
            let tail = rest.find("##").and_then(|first| {
                rest[first + 2..]
                    .find("##")
                    .map(|second| &rest[first + 2 + second + 2..])
            });
            match tail {
                Some(tail) => format!("{folder}{tail}"),
                None => path.to_owned(),
            }
        }
        None => path.to_owned(),
    }
}

/// A draft as a Concat document: every timeline a sequence in the draft's
/// frame, every file it uses in the bin, and each clip and title of its
/// tracks on a lane of its own track, the bottom track the main lane.
/// What has no counterpart yet - effects, filters, transitions, stickers,
/// keyframes, masks - is left out and named in the notes.
fn import_draft(draft: &Draft) -> Result<Imported, String> {
    let first = &draft.timelines[0].edit;
    let width = number(first, &["canvas_config", "width"]).unwrap_or(1920.0) as u32;
    let height = number(first, &["canvas_config", "height"]).unwrap_or(1080.0) as u32;
    let fps = first.get("fps").and_then(Value::as_f64).unwrap_or(30.0);
    let frame = Frame {
        width: std::num::NonZeroU32::new(width).unwrap_or(std::num::NonZeroU32::MIN),
        height: std::num::NonZeroU32::new(height).unwrap_or(std::num::NonZeroU32::MIN),
        rate: concat_doc::Rate::from_decimal(fps).unwrap_or(concat_doc::Rate::FPS_30),
        ..Frame::hd()
    };
    let mut document = Document::<Model>::new();
    let mut notes: Vec<String> = Vec::new();
    let mut note = |what: &str| {
        if !notes.iter().any(|kept| kept == what) {
            notes.push(what.to_owned());
        }
    };
    let made = |document: &mut Document<Model>, command: Command| -> Result<Applied, String> {
        let (after, applied) = commands::made(document, &command).map_err(|e| e.to_string())?;
        *document = after;
        Ok(applied)
    };
    let mut files: std::collections::HashMap<String, MediaId> = std::collections::HashMap::new();
    for (index, timeline) in draft.timelines.iter().enumerate() {
        let applied = made(&mut document, Command::AddTimeline)?;
        let sequence: SequenceId = applied
            .created
            .and_then(|id| id.typed())
            .ok_or("no timeline was made")?;
        made(
            &mut document,
            Command::SetTimelineVideo {
                sequence,
                frame: frame.clone(),
            },
        )?;
        if !timeline.name.trim().is_empty() {
            made(
                &mut document,
                Command::RenameTimeline {
                    sequence,
                    name: timeline.name.clone(),
                },
            )?;
        }
        if index == 0 {
            made(&mut document, Command::SelectTimeline { sequence })?;
        }
        let edit = &timeline.edit;
        // The files, by the draft's own ids.
        let materials = edit.get("materials");
        let list = |kind: &str| -> Vec<&Value> {
            materials
                .and_then(|materials| materials.get(kind))
                .and_then(Value::as_array)
                .map(|list| list.iter().collect())
                .unwrap_or_default()
        };
        for (kind, material) in list("videos")
            .into_iter()
            .map(|m| ("videos", m))
            .chain(list("audios").into_iter().map(|m| ("audios", m)))
        {
            let Some(id) = material.get("id").and_then(Value::as_str) else {
                continue;
            };
            let path = material.get("path").and_then(Value::as_str).unwrap_or("");
            if path.is_empty() {
                note("files CapCut keeps for itself");
                continue;
            }
            let photo = material.get("type").and_then(Value::as_str) == Some("photo");
            let media_kind = match (kind, photo) {
                ("audios", _) => MediaKind::Audio,
                (_, true) => MediaKind::Image,
                _ => MediaKind::Video,
            };
            let path = file_path(path, &draft.folder);
            let size = concat_doc::model::Size::new(
                number(material, &["width"]).unwrap_or(0.0) as u32,
                number(material, &["height"]).unwrap_or(0.0) as u32,
            );
            let media = Media {
                name: Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone()),
                path,
                length: match media_kind {
                    MediaKind::Image => None,
                    _ => micros(material.get("duration")).and_then(|t| TickLen::try_from(t).ok()),
                },
                has_sound: media_kind != MediaKind::Image,
                size: if media_kind == MediaKind::Audio {
                    None
                } else {
                    size
                },
                kind: media_kind,
                ..Media::default()
            };
            let applied = made(&mut document, Command::AddMedia { media })?;
            if let Some(media) = applied.created.and_then(|id| id.typed()) {
                files.insert(id.to_owned(), media);
            }
        }
        let texts: std::collections::HashMap<&str, String> = list("texts")
            .into_iter()
            .filter_map(|text| {
                let id = text.get("id").and_then(Value::as_str)?;
                let content = text.get("content").and_then(Value::as_str).unwrap_or("");
                // The words are a JSON text of their own, with their looks.
                let words = serde_json::from_str::<Value>(content)
                    .ok()
                    .and_then(|inner| inner.get("text").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_else(|| content.to_owned());
                Some((id, words))
            })
            .collect();
        for kind in [
            "effects",
            "video_effects",
            "transitions",
            "stickers",
            "filters",
        ] {
            if !list(kind).is_empty() {
                note(&format!("CapCut's {}", kind.replace('_', " ")));
            }
        }

        let tracks = edit.get("tracks").and_then(Value::as_array);
        for (place, track) in tracks.into_iter().flatten().enumerate() {
            let lanes: Vec<LaneId> = document
                .lanes
                .index()
                .of(sequence)
                .map(|(id, _)| id)
                .collect();
            let lane = match lanes.get(place) {
                Some(lane) => *lane,
                None => {
                    made(&mut document, Command::AddTrack { bottom: false })?;
                    let Some((lane, _)) = document.lanes.index().of(sequence).last() else {
                        continue;
                    };
                    lane
                }
            };
            let segments = track.get("segments").and_then(Value::as_array);
            for segment in segments.into_iter().flatten() {
                let material = segment
                    .get("material_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let (Some(start), Some(len)) = (
                    micros(segment.pointer("/target_timerange/start")),
                    micros(segment.pointer("/target_timerange/duration"))
                        .and_then(|t| TickLen::try_from(t).ok()),
                ) else {
                    continue;
                };
                let content = if let Some(media) = files.get(material) {
                    let source = micros(segment.pointer("/source_timerange/start")).unwrap_or(0);
                    let speed = segment
                        .get("speed")
                        .and_then(Value::as_f64)
                        .and_then(commands::speed_from_decimal)
                        .unwrap_or(concat_doc::model::Speed::NORMAL);
                    let mut cut = MediaCut::whole(*media);
                    cut.time = TimeMap::Linear {
                        src_in: Tick::new(source),
                        speed,
                    };
                    Content::Media(cut)
                } else if let Some(words) = texts.get(material) {
                    Content::Title(Title {
                        content: words.clone(),
                        ..Title::default()
                    })
                } else {
                    note("clips of things Concat does not import yet");
                    continue;
                };
                let mut item = Item::new(lane, Tick::new(start), len, content);
                if let Some(volume) = segment.get("volume").and_then(Value::as_f64) {
                    item.sound.volume = Animated::fixed(volume.max(0.0));
                }
                if let Some(clip) = segment.get("clip") {
                    let p = &mut item.picture;
                    if let Some(scale) = number(clip, &["scale", "x"]).filter(|s| *s > 0.0) {
                        p.scale = Animated::fixed(scale);
                    }
                    // CapCut places a picture by its centre from the
                    // frame's, a whole frame to a side being 2, up being
                    // positive.
                    if let Some(x) = number(clip, &["transform", "x"]) {
                        p.offset_x = Animated::fixed(x / 2.0);
                    }
                    if let Some(y) = number(clip, &["transform", "y"]) {
                        p.offset_y = Animated::fixed(-y / 2.0);
                    }
                    if let Some(turn) = number(clip, &["rotation"]) {
                        p.rotation = Animated::fixed(turn);
                    }
                    if let Some(alpha) = number(clip, &["alpha"]) {
                        p.opacity = Animated::fixed(alpha.clamp(0.0, 1.0));
                    }
                }
                if segment
                    .get("common_keyframes")
                    .and_then(Value::as_array)
                    .is_some_and(|keys| !keys.is_empty())
                {
                    note("keyframes");
                }
                let mut draft_doc = document.begin("Import", concat_doc::Origin(0));
                let Ok(id) = draft_doc.mint::<ItemId>() else {
                    continue;
                };
                if draft_doc.items.insert(id, item).is_err() {
                    continue;
                }
                match draft_doc.commit() {
                    Ok(commit) => document = commit.into_document(),
                    Err(_) => note("clips that do not fit where CapCut had them"),
                }
            }
        }
    }
    Ok(Imported {
        document,
        name: draft.name.clone(),
        frame,
        notes,
    })
}

/// CapCut's drafts on this machine, where it keeps them: the folder holding
/// one folder a draft. `None` where there is none.
pub fn drafts_folder() -> Option<PathBuf> {
    let under = |base: Option<PathBuf>, tail: &str| base.map(|base| base.join(tail));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidates = [
        // The download from capcut.com, then the App Store's, which is
        // sandboxed into a container of its own.
        under(
            home.clone(),
            "Movies/CapCut/User Data/Projects/com.lveditor.draft",
        ),
        under(
            home,
            "Library/Containers/com.lemon.lvoverseas/Data/Movies/CapCut/User Data/Projects/com.lveditor.draft",
        ),
        under(
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            "CapCut/User Data/Projects/com.lveditor.draft",
        ),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|folder| folder.is_dir())
}

/// Whether `folder` is a CapCut draft.
pub fn is_draft(folder: &Path) -> bool {
    DRAFT_FILES.iter().any(|file| folder.join(file).is_file())
}

/// A JSON file of the draft's, or `None` when it is missing.
fn json(path: &Path) -> Option<Result<Value, String>> {
    let bytes = std::fs::read(path).ok()?;
    Some(serde_json::from_slice(&bytes).map_err(|_| {
        // Not JSON: CapCut encrypts the drafts of some of its builds and
        // every draft made from one of its templates.
        "this CapCut draft is encrypted by CapCut, and cannot be read by anything else; \
         a draft made in an older CapCut, or not from a template, can"
            .to_owned()
    }))
}

/// The edit in `folder`: its first draft file there.
fn edit_in(folder: &Path) -> Option<Result<Value, String>> {
    DRAFT_FILES.iter().find_map(|file| json(&folder.join(file)))
}

/// Reads the draft in `folder`: its name and every timeline it has, the
/// one CapCut opens first first.
pub fn read(folder: &Path) -> Result<Draft, String> {
    let main =
        edit_in(folder).ok_or_else(|| format!("{} is not a CapCut draft", folder.display()))??;
    let meta = json(&folder.join("draft_meta_info.json")).and_then(Result::ok);
    let name = meta
        .as_ref()
        .and_then(|meta| meta.get("draft_name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "CapCut draft".to_owned());

    // A draft of several timelines lists them in `Timelines/project.json`,
    // each in a folder of its own; the draft's own file is the main one's.
    let mut timelines = Vec::new();
    let listed = json(&folder.join("Timelines").join("project.json")).and_then(Result::ok);
    if let Some(listed) = listed {
        let main_id = listed
            .get("main_timeline_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut entries: Vec<&Value> = listed
            .get("timelines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("is_marked_delete").and_then(Value::as_bool) != Some(true))
            .collect();
        entries.sort_by_key(|entry| entry.get("id").and_then(Value::as_str) != Some(main_id));
        for entry in entries {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
            let edit = if id == main_id {
                Some(Ok(main.clone()))
            } else {
                edit_in(&folder.join("Timelines").join(id))
            };
            if let Some(Ok(edit)) = edit {
                timelines.push(DraftTimeline {
                    name: entry
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    edit,
                });
            }
        }
    }
    if timelines.is_empty() {
        timelines.push(DraftTimeline {
            name: String::new(),
            edit: main,
        });
    }
    Ok(Draft {
        name,
        folder: folder.to_string_lossy().into_owned(),
        timelines,
    })
}

/// What an import made: the new project, and what of the draft did not
/// come across.
pub struct Import {
    /// The project, ready to open.
    pub project: ProjectInfo,
    /// One line for each kind of thing left out or changed.
    pub notes: Vec<String>,
}

/// Imports the CapCut draft in `folder` as a new project under `location`,
/// in a folder named after the draft - "Reel", then "Reel 2" if that is
/// taken - and writes its whole document there.
pub fn import(folder: &Path, location: &str) -> Result<Import, String> {
    let draft = read(folder)?;
    let imported = import_draft(&draft)?;
    let base = imported.name.trim();
    let base = if base.is_empty() {
        "CapCut draft"
    } else {
        base
    };
    let name = (1..)
        .map(|number| {
            if number == 1 {
                base.to_owned()
            } else {
                format!("{base} {number}")
            }
        })
        .find(|name| {
            !Path::new(location)
                .join(projects::folder_name(name))
                .exists()
        })
        .expect("a free name");
    let frame = &imported.frame;
    let project = projects::create(
        location,
        &name,
        frame.width.get(),
        frame.height.get(),
        i64::from(frame.rate.num()),
        i64::from(frame.rate.den()),
        [0, 0, 0, 255],
    )?;
    concat_doc::save::save_document(
        &crate::session::document_path(Path::new(&project.path)),
        &imported.document,
    )
    .map_err(|error| format!("could not save the imported project: {error}"))?;
    log::info!(
        "capcut: imported {} as {} ({} timeline(s), {} media)",
        folder.display(),
        project.path,
        imported.document.sequences.len(),
        imported.document.media.len()
    );
    for note in &imported.notes {
        log::info!("capcut: not carried: {note}");
    }
    Ok(Import {
        project,
        notes: imported.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every draft CapCut has on this machine imports, into a scratch
    /// folder, and opens. Ignored: it needs CapCut's drafts, and reads
    /// them where CapCut keeps them. `cargo test -p concat-host --lib
    /// capcut -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs CapCut's drafts on this machine"]
    fn the_drafts_on_this_machine_import() {
        let Some(drafts) = drafts_folder() else {
            eprintln!("no CapCut drafts here");
            return;
        };
        let location = std::env::temp_dir().join(format!("concat-capcut-{}", std::process::id()));
        std::fs::create_dir_all(&location).expect("a scratch folder");
        for entry in std::fs::read_dir(&drafts).expect("the drafts").flatten() {
            let folder = entry.path();
            if !is_draft(&folder) {
                continue;
            }
            match import(&folder, &location.to_string_lossy()) {
                Ok(import) => {
                    let session =
                        crate::session::Session::open_info(&import.project).expect("opens");
                    let document = session.document();
                    eprintln!(
                        "{}: {} timeline(s), {} clip(s), {} media; notes: {:?}",
                        folder.display(),
                        document.sequences.len(),
                        document.items.len(),
                        document.media.len(),
                        import.notes
                    );
                }
                Err(error) => eprintln!("{}: {error}", folder.display()),
            }
        }
        let _ = std::fs::remove_dir_all(&location);
    }

    /// A draft of one video clip and one title imports as a timeline in
    /// the draft's frame, the clip on the main lane where CapCut had it.
    #[test]
    fn a_draft_of_a_clip_and_a_title_imports() {
        let edit = serde_json::json!({
            "canvas_config": {"width": 1080, "height": 1920},
            "fps": 30.0,
            "materials": {
                "videos": [{"id": "v1", "path": "/footage/a.mp4", "type": "video",
                            "duration": 10_000_000, "width": 1920, "height": 1080}],
                "texts": [{"id": "t1", "content": "{\"text\":\"Hello\"}"}]
            },
            "tracks": [
                {"type": "video", "segments": [{"material_id": "v1",
                    "target_timerange": {"start": 0, "duration": 4_000_000},
                    "source_timerange": {"start": 2_000_000, "duration": 4_000_000},
                    "speed": 1.0, "volume": 0.5}]},
                {"type": "text", "segments": [{"material_id": "t1",
                    "target_timerange": {"start": 1_000_000, "duration": 2_000_000}}]}
            ]
        });
        let draft = Draft {
            name: "Reel".to_owned(),
            folder: "/drafts/reel".to_owned(),
            timelines: vec![DraftTimeline {
                name: String::new(),
                edit,
            }],
        };
        let imported = import_draft(&draft).expect("imports");
        assert_eq!(imported.frame.width.get(), 1080);
        assert_eq!(imported.document.media.len(), 1);
        assert_eq!(imported.document.items.len(), 2);
        let sequence = imported.document.view.showing.expect("a timeline shows");
        let main = imported
            .document
            .lanes
            .index()
            .of(sequence)
            .next()
            .expect("a lane")
            .0;
        let (clip, start, _) = imported
            .document
            .items
            .index()
            .on_lane(main)
            .next()
            .expect("the clip is on the main lane");
        assert_eq!(start, Tick::ZERO);
        let clip = imported.document.items.get(clip).expect("there");
        assert_eq!(clip.sound.volume.value, 0.5);
        assert!(imported.document.items.iter().any(|(_, item)| matches!(
            &item.content,
            Content::Title(title) if title.content == "Hello"
        )));
    }

    #[test]
    fn a_folder_without_a_draft_is_said_to_be_one() {
        let error = read(Path::new("/nonexistent/folder")).expect_err("refused");
        assert!(error.contains("not a CapCut draft"), "{error}");
    }
}
