// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Desktop launch arguments, kept separate from the window and engine.

use std::ffi::OsString;
use std::path::PathBuf;

const USAGE: &str = "usage: concat [--project <PROJECT_DIRECTORY> | --version | --smoke]";

/// What one run of the binary was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// The window, on the launch screen or on the project it names.
    Normal(Option<PathBuf>),
    /// Print the version and exit, before logging, the GPU or a window: a
    /// run that gets this far has proved the executable and every library
    /// it links were found and loaded, which on Windows fails before `main`.
    Version,
    /// Start the way a desktop launch does, through the first publish of
    /// the window's models, and exit instead of entering the event loop.
    /// For CI, which has no one to close a window.
    Smoke,
}

/// No arguments preserves the launch screen; a project names its directory.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Launch, String> {
    let mut args = args.into_iter();
    let Some(option) = args.next() else {
        return Ok(Launch::Normal(None));
    };
    let launch = if option == "--version" {
        Launch::Version
    } else if option == "--smoke" {
        Launch::Smoke
    } else if option == "--project" {
        let path = args
            .next()
            .filter(|path| !path.is_empty() && path != "--project")
            .ok_or_else(|| "--project requires a project directory".to_string())?;
        Launch::Normal(Some(PathBuf::from(path)))
    } else {
        return Err(USAGE.into());
    };
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    Ok(launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Launch, String> {
        parse(args.iter().map(|arg| OsString::from(*arg)))
    }

    #[test]
    fn no_arguments_keeps_the_launch_screen() {
        assert_eq!(parse_strs(&[]).unwrap(), Launch::Normal(None));
    }

    #[test]
    fn accepts_a_directory_with_spaces_and_unicode() {
        assert_eq!(
            parse_strs(&["--project", "podcasts/Café com amigos"]).unwrap(),
            Launch::Normal(Some(PathBuf::from("podcasts/Café com amigos")))
        );
    }

    #[test]
    fn version_and_smoke_take_no_arguments() {
        assert_eq!(parse_strs(&["--version"]).unwrap(), Launch::Version);
        assert_eq!(parse_strs(&["--smoke"]).unwrap(), Launch::Smoke);
        for args in [
            vec!["--version", "--smoke"],
            vec!["--smoke", "--project", "one"],
            vec!["--version", "extra"],
            vec!["--project", "one", "--smoke"],
        ] {
            assert!(parse_strs(&args).is_err(), "{args:?}");
        }
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
            assert!(parse_strs(&args).is_err(), "{args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_unicode_paths_for_the_open_error_handler() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'/', 0xff]);
        assert_eq!(
            parse([OsString::from("--project"), path.clone()]).unwrap(),
            Launch::Normal(Some(PathBuf::from(path)))
        );
    }
}
