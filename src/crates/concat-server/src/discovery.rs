// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! How a program on this machine finds the window's server: a small file
//! in the app's config folder, written while the server runs and removed
//! when it stops.
//!
//! The file is what an MCP adapter started by an agent reads - `concat
//! --mcp`, launched by Claude Code or another client with no way to be
//! told an address or a token - so it can go through the window's server
//! rather than open projects behind the window's back. It holds the token,
//! so it is written readable by its owner alone (0600 on Unix; on Windows
//! the config folder's own per-user permissions), the way the preferences
//! that may already hold a chosen token are. A file left by a window that
//! crashed names a server that is no longer there; a reader finds that
//! out by connecting, and [`Listing::pid`] lets it say why.
//!
//! ```json
//! {"version": 1, "address": "127.0.0.1:7420", "token": "...",
//!  "pid": 4242, "concat": "0.2.7", "apiVersion": "0.2"}
//! ```

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file's name in the config folder.
pub const FILE_NAME: &str = "automation.json";
/// This layout. A reader refuses a file with another.
pub const VERSION: u32 = 1;

/// Where the listing is kept, under the app's config folder.
pub fn path_in(config: &Path) -> PathBuf {
    config.join(FILE_NAME)
}

/// What the file says.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// [`VERSION`].
    pub version: u32,
    /// Where to connect: the JSON-RPC address, made reachable from this
    /// machine - a server bound to every interface is reached on loopback.
    pub address: SocketAddr,
    /// The token to present.
    pub token: String,
    /// The process serving, for a reader that wants to tell a stale file
    /// from a server that is busy.
    pub pid: u32,
    /// The Concat build serving.
    pub concat: String,
    /// The API contract it speaks.
    pub api_version: String,
}

impl Listing {
    /// A listing for a server bound to `bound`, in this process.
    pub fn new(bound: SocketAddr, token: &str) -> Listing {
        Listing {
            version: VERSION,
            address: crate::loopback(bound),
            token: token.to_owned(),
            pid: std::process::id(),
            concat: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: concat_api::API_VERSION.to_owned(),
        }
    }
}

/// Writes `listing` to `path`, readable by its owner alone, replacing what
/// was there in one step: written beside it and renamed over it, so a
/// reader sees the old file or the new one and never half of either.
pub fn write(path: &Path, listing: &Listing) -> Result<(), String> {
    let could_not = |error: &dyn std::fmt::Display| {
        format!("could not write {}: {error}", path.display())
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| could_not(&error))?;
    }
    let text = serde_json::to_string_pretty(listing).map_err(|error| could_not(&error))?;
    let staging = path.with_extension(format!("json.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&staging)
        .and_then(|mut file| {
            // The mode above applies to a file created now; one left from
            // before with wider permissions is narrowed here.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&staging, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&staging);
        return Err(could_not(&error));
    }
    Ok(())
}

/// The listing at `path`: `None` when there is none, `Err` when there is
/// one that cannot be read or is of another layout.
pub fn read(path: &Path) -> Result<Option<Listing>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let listing: Listing = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a server listing: {error}", path.display()))?;
    if listing.version != VERSION {
        return Err(format!(
            "{} is layout {}, and this build reads {VERSION}",
            path.display(),
            listing.version
        ));
    }
    Ok(Some(listing))
}

/// Takes the listing away - but only one this process wrote, so a second
/// window stopping its server does not unlist the first's.
pub fn remove(path: &Path) {
    if let Ok(Some(listing)) = read(path)
        && listing.pid != std::process::id()
    {
        return;
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_round_trips_owner_only_and_is_removed_by_its_writer() {
        let scratch = tempfile::tempdir().expect("scratch");
        let path = path_in(&scratch.path().join("config"));
        assert_eq!(read(&path), Ok(None));
        let listing = Listing::new("0.0.0.0:7420".parse().expect("an address"), "t0ken");
        assert_eq!(listing.address, "127.0.0.1:7420".parse().expect("an address"));
        write(&path, &listing).expect("writes");
        assert_eq!(read(&path), Ok(Some(listing.clone())));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("there").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }
        // Someone else's listing stays.
        let theirs = Listing {
            pid: listing.pid.wrapping_add(1),
            ..listing.clone()
        };
        write(&path, &theirs).expect("writes");
        remove(&path);
        assert!(path.exists());
        write(&path, &listing).expect("writes");
        remove(&path);
        assert!(!path.exists());
    }

    #[test]
    fn a_file_of_another_layout_or_none_is_said_so() {
        let scratch = tempfile::tempdir().expect("scratch");
        let path = scratch.path().join(FILE_NAME);
        std::fs::write(&path, "{").expect("writes");
        assert!(read(&path).is_err());
        let mut listing = Listing::new("[::]:1".parse().expect("an address"), "t");
        assert_eq!(listing.address, "[::1]:1".parse().expect("an address"));
        listing.version = VERSION + 1;
        std::fs::write(&path, serde_json::to_string(&listing).expect("JSON")).expect("writes");
        assert!(read(&path).expect_err("refused").contains("layout"));
    }
}
