// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Convert project media and font paths between local paths and portable
//! paths stored relative to the project folder.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// Store paths inside the project relative to its folder, with `/` separators.
/// Absolute paths outside the project remain unchanged.
pub fn to_portable(root: &Path, document: &mut Value) {
    walk_paths(root, document, |root, path| {
        if let Some(relative) = relative_to(root, path) {
            return relative.replace('\\', "/");
        }
        if !is_absolute(path) {
            return path.replace('\\', "/");
        }
        path.to_owned()
    });
}

/// Resolve project-relative paths for local use and recover copied media when
/// an older project still names its former absolute location.
pub fn to_local(root: &Path, document: &mut Value) {
    walk_paths(root, document, |root, path| {
        if is_absolute(path) {
            if Path::new(path).exists() {
                return local_path_string(Path::new(path));
            }
            return fallback_media(root, path)
                .map(|fallback| local_path_string(&fallback))
                .unwrap_or_else(|| path.to_owned());
        }

        let local_path = if cfg!(windows) || is_windows_path(&root.to_string_lossy()) {
            path.replace('/', "\\")
        } else {
            path.to_owned()
        };
        local_path_string(&root.join(local_path))
    });
}

/// Return an ordinary local spelling for paths stored in the project model.
/// Windows accepts verbatim paths and mixed separators, but they make saved
/// model values differ from paths produced by imports and edits.
pub fn local_path_string(path: &Path) -> String {
    let mut value = path.to_string_lossy().into_owned();
    if cfg!(windows) || value.starts_with(r"\\?\") || is_windows_path(&value) {
        value = value.replace('/', "\\");
    }
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        value = format!(r"\\{unc}");
    } else if let Some(path) = value.strip_prefix(r"\\?\") {
        value = path.to_owned();
    }
    value
}

fn walk_paths(root: &Path, document: &mut Value, mut convert: impl FnMut(&Path, &str) -> String) {
    for section in ["media", "fonts"] {
        let Some(items) = document.get_mut(section).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items {
            let Some(path) = item.get("path").and_then(Value::as_str) else {
                continue;
            };
            item["path"] = Value::String(convert(root, path));
        }
    }
}

fn relative_to(root: &Path, path: &str) -> Option<String> {
    let candidate = Path::new(path);
    let relative = candidate
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| normalized_relative(root, path))?;
    if relative
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return None;
    }
    Some(relative.to_string_lossy().into_owned())
}

fn normalized_relative(root: &Path, path: &str) -> Option<PathBuf> {
    let root = root.to_string_lossy().replace('\\', "/");
    let windows_root = is_windows_path(&root);
    let path = path.replace('\\', "/");
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return path.strip_prefix('/').map(PathBuf::from);
    }
    let prefix = format!("{root}/");
    let remainder = if windows_root {
        path.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(&prefix))
            .and_then(|_| path.get(prefix.len()..))
    } else {
        path.strip_prefix(&prefix)
    }?;
    Some(PathBuf::from(remainder))
}

fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || is_windows_path(path)
}

fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn fallback_media(root: &Path, original: &str) -> Option<PathBuf> {
    let basename = original.rsplit(['/', '\\']).next()?;
    if basename.is_empty() {
        return None;
    }
    let fallback = root.join("media").join(basename);
    fallback.is_file().then_some(fallback)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::{to_local, to_portable};

    fn path<'a>(document: &'a Value, section: &str, index: usize) -> &'a str {
        document[section][index]["path"].as_str().expect("path")
    }

    #[test]
    fn windows_style_path_inside_root_round_trips() {
        let root = tempfile::tempdir().expect("project folder");
        let media = root.path().join("media").join("a.mp4");
        std::fs::create_dir_all(media.parent().expect("media folder")).expect("creates media");
        std::fs::write(&media, b"video").expect("writes media");
        let mut document = json!({"media": [{"path": media.to_string_lossy()}], "fonts": []});

        to_portable(root.path(), &mut document);
        assert_eq!(path(&document, "media", 0), "media/a.mp4");

        to_local(root.path(), &mut document);
        assert_eq!(
            path(&document, "media", 0),
            media.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn outside_absolute_path_stays_absolute() {
        let root = tempfile::tempdir().expect("project folder");
        let outside = tempfile::NamedTempFile::new().expect("outside media");
        let original = outside.path().to_string_lossy().into_owned();
        let mut document = json!({"media": [{"path": original}], "fonts": []});

        to_portable(root.path(), &mut document);

        assert_eq!(path(&document, "media", 0), original);
    }

    #[test]
    fn relative_forward_slash_path_resolves_under_root() {
        let root = tempfile::tempdir().expect("project folder");
        let mut document = json!({"media": [{"path": "media/a.mp4"}], "fonts": []});

        to_local(root.path(), &mut document);

        assert_eq!(
            PathBuf::from(path(&document, "media", 0)),
            root.path().join("media/a.mp4")
        );
    }

    #[test]
    fn missing_absolute_path_falls_back_to_project_media_by_basename() {
        let root = tempfile::tempdir().expect("project folder");
        let fallback = root.path().join("media").join("a.mp4");
        std::fs::create_dir_all(fallback.parent().expect("media folder")).expect("creates media");
        std::fs::write(&fallback, b"video").expect("writes media");

        for missing in [r"D:\x\a.mp4", "/x/a.mp4"] {
            let mut document = json!({"media": [{"path": missing}], "fonts": []});
            to_local(root.path(), &mut document);
            assert_eq!(Path::new(path(&document, "media", 0)), fallback);
        }
    }

    #[test]
    fn walks_media_and_custom_font_paths() {
        let root = tempfile::tempdir().expect("project folder");
        let media = root.path().join("media").join("a.mp4");
        let font = root.path().join("media").join("a.ttf");
        std::fs::create_dir_all(media.parent().expect("media folder")).expect("creates media");
        let mut document = json!({
            "media": [{"path": media.to_string_lossy()}],
            "fonts": [{"path": font.to_string_lossy()}]
        });

        to_portable(root.path(), &mut document);

        assert_eq!(path(&document, "media", 0), "media/a.mp4");
        assert_eq!(path(&document, "fonts", 0), "media/a.ttf");
    }
}
