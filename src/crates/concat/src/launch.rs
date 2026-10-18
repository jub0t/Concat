// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Desktop launch arguments, kept separate from the window and engine.

use std::ffi::OsString;
use std::path::PathBuf;

const USAGE: &str = "usage: concat [--project <PROJECT_DIRECTORY> | --mcp [auto|window|headless] | --version | --smoke | --whisper-cpu]";

/// What one run of the binary was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// The window, on the launch screen or on the project it names.
    Normal(Option<PathBuf>),
    /// Serve the Concat API to an AI agent as an MCP server on standard
    /// input and output, with no window: what an agent's configuration
    /// names (docs/MCP.md). The mode, when given: through the window's
    /// server, on its own, or whichever is there; `CONCAT_MCP_MODE`
    /// otherwise, and `auto` without that.
    Mcp(Option<concat_mcp::Mode>),
    /// Print the version and exit, before logging, the GPU or a window: a
    /// run that gets this far has proved the executable and every library
    /// it links were found and loaded, which on Windows fails before `main`.
    Version,
    /// Start the way a desktop launch does, through the first publish of
    /// the window's models, and exit instead of entering the event loop.
    /// For CI, which has no one to close a window.
    Smoke,
    /// Load whisper's CPU kernels the way the first transcription does,
    /// print the CPU they run on and exit, before logging, the GPU or a
    /// window. For CI on an emulated older CPU: where the kernels are one
    /// module per CPU generation, ggml opens every one of them to ask it
    /// whether it suits, and a module whose loading alone used an
    /// instruction the CPU lacks would crash here, not at somebody's first
    /// transcription (#388).
    WhisperCpu,
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
    } else if option == "--whisper-cpu" {
        Launch::WhisperCpu
    } else if option == "--mcp" {
        match args.next() {
            None => Launch::Mcp(None),
            Some(word) => Launch::Mcp(Some(
                word.to_str()
                    .and_then(concat_mcp::Mode::parse)
                    .ok_or_else(|| format!("--mcp takes auto, window or headless\n{USAGE}"))?,
            )),
        }
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
        assert_eq!(parse_strs(&["--whisper-cpu"]).unwrap(), Launch::WhisperCpu);
        for args in [
            vec!["--version", "--smoke"],
            vec!["--whisper-cpu", "--version"],
            vec!["--smoke", "--project", "one"],
            vec!["--version", "extra"],
            vec!["--project", "one", "--smoke"],
        ] {
            assert!(parse_strs(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn mcp_takes_an_optional_mode_and_nothing_else() {
        use concat_mcp::Mode;
        assert_eq!(parse_strs(&["--mcp"]).unwrap(), Launch::Mcp(None));
        assert_eq!(
            parse_strs(&["--mcp", "headless"]).unwrap(),
            Launch::Mcp(Some(Mode::Headless))
        );
        assert_eq!(
            parse_strs(&["--mcp", "window"]).unwrap(),
            Launch::Mcp(Some(Mode::Window))
        );
        assert_eq!(
            parse_strs(&["--mcp", "auto"]).unwrap(),
            Launch::Mcp(Some(Mode::Auto))
        );
        for args in [
            vec!["--mcp", "sometimes"],
            vec!["--mcp", "headless", "extra"],
            vec!["--mcp", "--project"],
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
