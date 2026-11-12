// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Files stored alongside a project and their undoable path updates.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use concat_project::{Command, model::Project};

#[derive(Clone, Debug, Default)]
pub(crate) struct Collection {
    pub commands: Vec<Command>,
    pub missing_media: Vec<String>,
    pub missing_fonts: Vec<String>,
    pub copy_errors: Vec<String>,
}

pub(crate) fn copy_into_project_media(source: &Path, project_dir: &Path) -> io::Result<PathBuf> {
    let root = project_dir.canonicalize()?;
    let source_path = source.canonicalize()?;
    if source_path.starts_with(&root) {
        return Ok(source.to_path_buf());
    }

    // Keep the caller's path spelling in the returned path. On Windows,
    // canonicalize adds the `\\?\` prefix, which is useful for containment
    // checks but leaks into paths stored in the project document.
    let media_dir = project_dir.join("media");
    std::fs::create_dir_all(&media_dir)?;
    let name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "media path has no file name")
    })?;
    let stem = source_path.file_stem().unwrap_or(name).to_string_lossy();
    let extension = source_path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let mut source_file = std::fs::File::open(&source_path)?;

    for suffix in 1_u32.. {
        let candidate_name = if suffix == 1 {
            name.to_os_string()
        } else {
            format!("{stem} ({suffix}){extension}").into()
        };
        let destination = media_dir.join(candidate_name);
        let mut target = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(target) => target,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        if let Err(error) = io::copy(&mut source_file, &mut target) {
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
        return Ok(PathBuf::from(concat_host::media_paths::local_path_string(
            &destination,
        )));
    }

    unreachable!("the numbered media name space is larger than the filesystem")
}

pub(crate) fn collect(project: &Project, project_dir: &Path) -> Collection {
    let mut result = Collection::default();
    for item in &project.media {
        match collect_path(&item.path, project_dir) {
            Ok(Some(path)) => result.commands.push(Command::UpdateMediaPath {
                media_id: item.id.clone(),
                new_path: path.to_string_lossy().into_owned(),
            }),
            Ok(None) => {}
            Err(CollectError::Missing) => result.missing_media.push(item.name.clone()),
            Err(CollectError::Copy(error)) => {
                result.copy_errors.push(format!("{}: {error}", item.name));
            }
        }
    }
    for font in &project.fonts {
        match collect_path(&font.path, project_dir) {
            Ok(Some(path)) => result.commands.push(Command::UpdateFontPath {
                font_path: font.path.clone(),
                new_path: path.to_string_lossy().into_owned(),
            }),
            Ok(None) => {}
            Err(CollectError::Missing) => result.missing_fonts.push(font.family.clone()),
            Err(CollectError::Copy(error)) => {
                result.copy_errors.push(format!("{}: {error}", font.family));
            }
        }
    }
    result
}

enum CollectError {
    Missing,
    Copy(io::Error),
}

fn collect_path(path: &str, project_dir: &Path) -> Result<Option<PathBuf>, CollectError> {
    let source = Path::new(path);
    match copy_into_project_media(source, project_dir) {
        Ok(destination) if destination == source => Ok(None),
        Ok(destination) => Ok(Some(destination)),
        Err(_) if !source.is_file() => Err(CollectError::Missing),
        Err(error) => Err(CollectError::Copy(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "concat_{label}_{}_{}",
                std::process::id(),
                unique
            ));
            std::fs::create_dir_all(&path).expect("temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copying_an_outside_file_uses_a_numbered_name_on_collision() {
        let root = TestDirectory::new("project");
        let source_dir = TestDirectory::new("source");
        let source = source_dir.path().join("a.mp4");
        std::fs::write(&source, b"new video").expect("source file");
        let media_dir = root.path().join("media");
        std::fs::create_dir_all(&media_dir).expect("media folder");
        std::fs::write(media_dir.join("a.mp4"), b"existing video").expect("existing file");

        let imported = copy_into_project_media(&source, root.path()).expect("copy succeeds");

        assert_eq!(imported, media_dir.join("a (2).mp4"));
        assert_eq!(std::fs::read(imported).expect("copied file"), b"new video");
    }

    #[test]
    fn collecting_copies_fonts_and_reports_missing_media_and_fonts() {
        use concat_project::model::{CustomFont, MediaItem};

        let root = TestDirectory::new("project");
        let external = TestDirectory::new("external");
        let media = external.path().join("a.mp4");
        let font = external.path().join("font.ttf");
        std::fs::write(&media, b"video").expect("media file");
        std::fs::write(&font, b"font").expect("font file");
        let project = Project {
            media: vec![
                MediaItem {
                    id: "m1".into(),
                    path: media.to_string_lossy().into_owned(),
                    name: "a.mp4".into(),
                    ..MediaItem::default()
                },
                MediaItem {
                    id: "m2".into(),
                    path: external
                        .path()
                        .join("missing.mp4")
                        .to_string_lossy()
                        .into_owned(),
                    name: "missing.mp4".into(),
                    ..MediaItem::default()
                },
            ],
            fonts: vec![
                CustomFont {
                    family: "Test Font".into(),
                    path: font.to_string_lossy().into_owned(),
                },
                CustomFont {
                    family: "Missing Font".into(),
                    path: external
                        .path()
                        .join("missing.ttf")
                        .to_string_lossy()
                        .into_owned(),
                },
            ],
            ..Project::default()
        };

        let result = collect(&project, root.path());

        assert_eq!(result.commands.len(), 2);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            Command::UpdateFontPath { font_path, .. } if font_path == font.to_string_lossy().as_ref()
        )));
        assert_eq!(result.missing_media, ["missing.mp4"]);
        assert_eq!(result.missing_fonts, ["Missing Font"]);
        assert!(root.path().join("media/a.mp4").is_file());
        assert!(root.path().join("media/font.ttf").is_file());
    }
}
