// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Where the app keeps what belongs to this machine rather than to a
//! project: the recents list, remembered settings, downloaded models.
//!
//! Two directories, named by the app identifier and following each
//! platform's convention for configuration and data. On macOS they are the
//! same folder. On an iPhone the settings are in the sandbox's
//! `Application Support` and the rest in its `Caches`. On Windows the
//! settings roam (`%APPDATA%`) and the models and caches stay on the
//! machine (`%LOCALAPPDATA%`), since gigabytes of model have no business
//! following a domain profile to every PC it logs into. On Linux they
//! follow the XDG split.
//!
//! Or one directory, chosen by the person running the app: a folder named
//! `portable` beside the executable holds both, and nothing is written
//! anywhere else on the machine. That is what makes the zip a portable
//! build - unpack it on a stick, make the folder, and the settings, recents
//! and models travel with it and leave with it.

use std::path::{Path, PathBuf};

/// The app identifier, which is also the folder name everywhere.
pub const IDENTIFIER: &str = "org.standalones.concat";

/// The identifier every build before 2026-10-26 used, whose folders
/// [`adopt`] moves to [`IDENTIFIER`]'s on first start.
const FORMER_IDENTIFIER: &str = "app.concat.editor";

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
        let convention = convention(std::env::consts::OS);
        if convention == Convention::Apple {
            let dirs = apple(&home()?, cfg!(target_os = "ios"));
            adopt(&dirs.config);
            adopt(&dirs.data);
            Ok(dirs)
        } else if convention == Convention::Windows {
            let config = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .ok_or_else(|| "APPDATA is not set".to_owned())?
                .join(IDENTIFIER);
            let data = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map_or_else(|| config.clone(), |base| base.join(IDENTIFIER));
            adopt(&config);
            adopt(&data);
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
            let dirs = AppDirs {
                config: config_base.join(IDENTIFIER),
                data: data_base.join(IDENTIFIER),
            };
            adopt(&dirs.config);
            adopt(&dirs.data);
            Ok(dirs)
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

    /// Where what the user made and cannot download again is kept - the
    /// Audio page's own sounds, the voices they cloned: `data` everywhere
    /// but an iPhone, whose `data` is `Caches`, which the phone may empty
    /// when it runs short of space and does not back up. There it is
    /// `config`, `Application Support`, which it keeps.
    pub fn library(&self) -> PathBuf {
        if cfg!(target_os = "ios") {
            self.config.clone()
        } else {
            self.data.clone()
        }
    }
}

/// The folders under `data`, each named where it is made: models, caches
/// and logs. What is not listed is the settings', and stays in `config`.
const DATA_FOLDERS: [&str; 10] = [
    "cutout-models",  // concat-vision, models.rs
    "whisper-models", // concat-speech, transcribe.rs
    "tts-models",     // concat-speech, tts.rs
    "cards",          // concat, studio.rs
    "titles",         // titles.rs
    "updates",        // updates.rs
    "logs",           // logs.rs
    "audio",          // sounds.rs: the Audio page's own sounds; see `library`
    "voices",         // voices.rs: saved cloned voices; see `library`
    "sounds",         // sounds.rs: Concat's sounds, downloaded on first use
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

/// Moves the folder the former identifier named, beside `folder`, to
/// `folder`, once: the settings, recents, models and the user's own sounds
/// and voices of every build before the rename, so an update does not
/// open on an empty app and fetch every model again. Nothing happens when
/// `folder` exists already or there is nothing to move.
///
/// A link is left where the old folder was, to the new one: a project
/// that took a sound from the Audio page or a saved voice holds its path,
/// under the old name, and finds it through the link. A symbolic link on
/// a Mac and on Linux, a directory junction on Windows, which needs no
/// rights a symbolic link would. A folder that will not move - another
/// volume, something holding it open - is left where it is, and the app
/// starts afresh beside it rather than not at all.
fn adopt(folder: &Path) {
    let (Some(parent), Some(name)) = (folder.parent(), folder.file_name()) else {
        return;
    };
    if name != IDENTIFIER || folder.exists() {
        return;
    }
    let former = parent.join(FORMER_IDENTIFIER);
    if !former.is_dir() || former.is_symlink() {
        return;
    }
    if let Err(error) = std::fs::rename(&former, folder) {
        log::warn!(
            "could not move {} to {}: {error}",
            former.display(),
            folder.display()
        );
        return;
    }
    if let Err(error) = link(folder, &former) {
        log::warn!("could not leave a link at {}: {error}", former.display());
    }
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(windows)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    /// No console window flashing up behind the app's for it.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(at)
        .arg(target)
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("mklink /J answered {status}")))
    }
}

#[cfg(not(any(unix, windows)))]
fn link(_target: &Path, _at: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Which platform's rule names the directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Convention {
    /// `~/Library/Application Support`, both directories in one.
    Apple,
    /// `%APPDATA%` for the settings, `%LOCALAPPDATA%` for the rest.
    Windows,
    /// The XDG bases, or their defaults under the home directory.
    Xdg,
}

/// The convention for `os`, as `std::env::consts::OS` names it.
///
/// An iPhone is Apple's, as a Mac is: its `HOME` is the app's sandbox,
/// where `Library` is the app's own and the top of the container is not a
/// place the app may make folders. It used to fall through to the XDG branch, and the
/// settings, the recents and the log went to `~/.config` and
/// `~/.local/share`. https://github.com/jub0t/Concat/issues/299
fn convention(os: &str) -> Convention {
    match os {
        "macos" | "ios" => Convention::Apple,
        "windows" => Convention::Windows,
        _ => Convention::Xdg,
    }
}

/// Apple's directories under `home`. A Mac keeps both in one folder in
/// `Application Support`. An iPhone keeps the settings there and the rest -
/// models, caches, logs, every one of them made again or downloaded again
/// when missing - in `Caches`, which the phone does not back up to iCloud
/// and may empty when it runs short of space.
fn apple(home: &Path, phone: bool) -> AppDirs {
    let library = home.join("Library");
    let config = library.join("Application Support").join(IDENTIFIER);
    let data = if phone {
        library.join("Caches").join(IDENTIFIER)
    } else {
        config.clone()
    };
    AppDirs { config, data }
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

    /// A Mac and an iPhone keep their directories where Apple says to;
    /// Android and Linux take the XDG bases.
    #[test]
    fn an_iphone_follows_apples_convention() {
        assert_eq!(convention("ios"), Convention::Apple);
        assert_eq!(convention("macos"), Convention::Apple);
        assert_eq!(convention("windows"), Convention::Windows);
        assert_eq!(convention("android"), Convention::Xdg);
        assert_eq!(convention("linux"), Convention::Xdg);
    }

    #[test]
    fn a_mac_keeps_one_folder_and_an_iphone_caches_the_rest() {
        let home = Path::new("/home");
        let mac = apple(home, false);
        let support = home
            .join("Library")
            .join("Application Support")
            .join(IDENTIFIER);
        assert_eq!((&mac.config, &mac.data), (&support, &support));
        let phone = apple(home, true);
        assert_eq!(phone.config, support);
        assert_eq!(
            phone.data,
            home.join("Library").join("Caches").join(IDENTIFIER)
        );
    }

    /// A folder under the former identifier moves to the new name with
    /// everything in it, once, and its old path still reaches it.
    #[test]
    fn the_former_identifiers_folder_is_adopted() {
        let root = std::env::temp_dir().join(format!("concat-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let former = root.join(FORMER_IDENTIFIER);
        std::fs::create_dir_all(former.join("voices")).unwrap();
        std::fs::write(former.join("settings.json"), b"{}").unwrap();
        std::fs::write(former.join("voices").join("me.wav"), b"voice").unwrap();

        let folder = root.join(IDENTIFIER);
        adopt(&folder);
        assert!(folder.join("settings.json").is_file());
        assert!(folder.join("voices").join("me.wav").is_file());
        // A project's path under the old name still finds the file.
        assert_eq!(
            std::fs::read(former.join("voices").join("me.wav")).unwrap(),
            b"voice"
        );

        // Run again, nothing changes; and a folder already there is left be.
        adopt(&folder);
        assert!(folder.join("settings.json").is_file());
        let _ = std::fs::remove_dir_all(&root);
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
