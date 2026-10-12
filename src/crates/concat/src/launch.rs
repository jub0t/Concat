// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Desktop launch arguments, kept separate from the window and engine.

use std::ffi::OsString;
use std::path::PathBuf;

/// No arguments preserves the launch screen; a project names its directory.
pub fn project(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>, String> {
    let mut args = args.into_iter();
    let Some(option) = args.next() else {
        return Ok(None);
    };
    if option != "--project" {
        return Err("usage: concat [--project <PROJECT_DIRECTORY>]".into());
    }
    let path = args
        .next()
        .filter(|path| !path.is_empty() && path != "--project")
        .ok_or_else(|| "--project requires a project directory".to_string())?;
    if args.next().is_some() {
        return Err("usage: concat [--project <PROJECT_DIRECTORY>]".into());
    }
    Ok(Some(PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<PathBuf>, String> {
        project(args.iter().map(|arg| OsString::from(*arg)))
    }

    #[test]
    fn no_arguments_keeps_the_launch_screen() {
        assert_eq!(parse(&[]).unwrap(), None);
    }

    #[test]
    fn accepts_a_directory_with_spaces_and_unicode() {
        assert_eq!(
            parse(&["--project", "podcasts/Café com amigos"]).unwrap(),
            Some(PathBuf::from("podcasts/Café com amigos"))
        );
    }

    #[test]
    fn rejects_missing_empty_duplicate_and_unknown_arguments() {
        for args in [
            vec!["--project"],
            vec!["--project", ""],
            vec!["--project", "--project"],
            vec!["--project", "one", "--project", "two"],
            vec!["--unknown"],
            vec!["one"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_unicode_paths_for_the_open_error_handler() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'/', 0xff]);
        assert_eq!(
            project([OsString::from("--project"), path.clone()]).unwrap(),
            Some(PathBuf::from(path))
        );
    }
}
