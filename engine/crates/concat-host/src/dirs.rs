// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where the app keeps what belongs to this machine rather than to a
//! project: the recents list, remembered settings, downloaded models.
//!
//! Two directories, named by the app identifier and following each
//! platform's convention for configuration and data. On macOS and Windows
//! they are the same folder; on Linux they follow the XDG split.

use std::path::{Path, PathBuf};

/// The app identifier, which is also the folder name everywhere.
pub const IDENTIFIER: &str = "app.concat.editor";

/// The app's own directories on this machine.
#[derive(Clone, Debug)]
pub struct AppDirs {
    /// Small state: recents, remembered settings, the template library.
    pub config: PathBuf,
    /// Large state: downloaded models.
    pub data: PathBuf,
}

impl AppDirs {
    /// The platform's directories for this app, created lazily by whoever
    /// writes into them.
    pub fn locate() -> Result<AppDirs, String> {
        if cfg!(target_os = "macos") {
            let dir = home()?
                .join("Library")
                .join("Application Support")
                .join(IDENTIFIER);
            Ok(AppDirs {
                config: dir.clone(),
                data: dir,
            })
        } else if cfg!(windows) {
            let dir = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .ok_or_else(|| "APPDATA is not set".to_owned())?
                .join(IDENTIFIER);
            Ok(AppDirs {
                config: dir.clone(),
                data: dir,
            })
        } else {
            // Linux follows the XDG split. Android arrives here too: the
            // activity names both bases before the window starts, since an
            // app process there has no home directory to derive them from.
            let base = |variable: &str| {
                std::env::var_os(variable)
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
            };
            let config_base = match base("XDG_CONFIG_HOME") {
                Some(path) => path,
                None => home()?.join(".config"),
            };
            let data_base = match base("XDG_DATA_HOME") {
                Some(path) => path,
                None => home()?.join(".local").join("share"),
            };
            Ok(AppDirs {
                config: config_base.join(IDENTIFIER),
                data: data_base.join(IDENTIFIER),
            })
        }
    }

    /// Both directories under one root. For tests, and for anyone running
    /// a portable build out of a folder.
    pub fn under(root: &Path) -> AppDirs {
        AppDirs {
            config: root.to_path_buf(),
            data: root.to_path_buf(),
        }
    }
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "could not locate the home directory".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directories_are_named_by_the_identifier() {
        let dirs = AppDirs::locate().expect("locates");
        assert!(dirs.config.ends_with(IDENTIFIER));
        assert!(dirs.data.ends_with(IDENTIFIER));
        assert!(dirs.config.is_absolute());
    }
}
