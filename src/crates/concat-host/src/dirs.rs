// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where the app keeps what belongs to this machine rather than to a
//! project: the recents list, remembered settings, downloaded models.
//!
//! Two directories, named by the app identifier and following each
//! platform's convention for configuration and data. On macOS they are the
//! same folder. On Windows the settings roam (`%APPDATA%`) and the models
//! and caches stay on the machine (`%LOCALAPPDATA%`), since gigabytes of
//! model have no business following a domain profile to every PC it logs
//! into. On Linux they follow the XDG split.
//!
//! Or one directory, chosen by the person running the app: a folder named
//! `portable` beside the executable holds both, and nothing is written
//! anywhere else on the machine. That is what makes the zip a portable
//! build - unpack it on a stick, make the folder, and the settings, recents
//! and models travel with it and leave with it.

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
    /// writes into them - or the `portable` folder beside the executable,
    /// when there is one.
    pub fn locate() -> Result<AppDirs, String> {
        if let Some(portable) = portable_root() {
            return Ok(AppDirs::under(&portable));
        }
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
            let config = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .ok_or_else(|| "APPDATA is not set".to_owned())?
                .join(IDENTIFIER);
            let data = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map_or_else(|| config.clone(), |base| base.join(IDENTIFIER));
            move_data(&config, &data);
            Ok(AppDirs { config, data })
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

/// The folders under `data`, each named where it is made: models, caches
/// and logs. What is not listed is the settings', and stays in `config`.
const DATA_FOLDERS: [&str; 7] = [
    "cutout-models",   // concat-vision, models.rs
    "whisper-models",  // concat-speech, transcribe.rs
    "tts-models",      // concat-speech, tts.rs
    "cards",           // concat, studio.rs
    "titles",          // titles.rs
    "updates",         // updates.rs
    "logs",            // logs.rs
];

/// Moves what an older build kept under `from` - Windows kept models and
/// settings in one roaming folder - into `to`, once: a folder already at
/// `to` is left alone. A rename, so nothing is copied when both are on one
/// volume, as %APPDATA% and %LOCALAPPDATA% are; a folder that will not move
/// stays behind, and is downloaded or rebuilt again where it is now looked
/// for.
fn move_data(from: &Path, to: &Path) {
    if from == to {
        return;
    }
    for name in DATA_FOLDERS {
        let (old, new) = (from.join(name), to.join(name));
        if old.is_dir() && !new.exists() {
            let _ = std::fs::create_dir_all(to);
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// The `portable` folder beside the executable, when someone has made one.
/// A folder rather than a marker file, because it *is* where everything
/// then goes, and its presence is the whole of the switch: no flag, no
/// setting, nothing to remember. A phone's executable has no such
/// neighbour, and the desktop's never has one unless a person put it there.
fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.join("portable");
    folder.is_dir().then_some(folder)
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

    /// An older layout's models move to the new home once, and the settings
    /// beside them stay where they were.
    #[test]
    fn the_models_move_and_the_settings_stay() {
        let root = std::env::temp_dir().join(format!("concat-dirs-{}", std::process::id()));
        let (roaming, local) = (root.join("roaming"), root.join("local"));
        std::fs::create_dir_all(roaming.join("whisper-models")).unwrap();
        std::fs::write(roaming.join("whisper-models").join("base.bin"), b"model").unwrap();
        std::fs::write(roaming.join("settings.json"), b"{}").unwrap();

        move_data(&roaming, &local);
        assert!(local.join("whisper-models").join("base.bin").is_file());
        assert!(!roaming.join("whisper-models").exists());
        assert!(roaming.join("settings.json").is_file());
        assert!(!local.join("settings.json").exists());

        // A second run finds them moved and changes nothing.
        move_data(&roaming, &local);
        assert!(local.join("whisper-models").join("base.bin").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
