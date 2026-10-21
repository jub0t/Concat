// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Whether this machine can run ONNX Runtime at all.
//!
//! Everywhere but an Intel Mac the runtime is linked into the app, or
//! shipped beside it and loaded at launch, and it runs wherever the app
//! does. An Intel Mac gets Microsoft's 1.23.2, the last build there is for
//! it, and that one is built for macOS 13.4 while the app runs on 12: it
//! calls libc++'s floating-point `to_chars`, which macOS has from 13.3, and
//! linked at load time it kept the app from starting at all on 12 (#333).
//! So there it is loaded at first use, from the bundle's Frameworks, and
//! only on a macOS at least as new as the one its own header names. The
//! minimum is read from the file rather than written down here, so a
//! different runtime in the bundle brings its own.
//!
//! This part does no inference and needs no runtime: Settings asks it
//! too, to say up front what the AI features need.

// Elsewhere only the tests read the header and the version: the answer
// there is always that nothing is known against the runtime.
#![cfg_attr(
    not(all(target_os = "macos", target_arch = "x86_64")),
    allow(dead_code)
)]

use std::fmt;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::path::PathBuf;

/// Why the AI features - background removal, the smart brushes, Enhance
/// and Chatterbox - cannot run on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// The macOS the runtime needs, as `13.4`.
    pub needs: String,
    /// The macOS this Mac runs.
    pub runs: String,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the AI features need macOS {} or later, and this Mac runs {}",
            self.needs, self.runs
        )
    }
}

/// Why the AI features cannot run here, or `None` when nothing is known
/// against them. Worked out once: neither the runtime in the bundle nor
/// the running system changes under a process.
pub fn unavailable() -> Option<&'static Unavailable> {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        static ANSWER: std::sync::OnceLock<Option<Unavailable>> = std::sync::OnceLock::new();
        ANSWER
            .get_or_init(|| {
                let needs = minimum_macos(&header(&library())?)?;
                let plist =
                    std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
                        .ok()?;
                let runs = product_version(&plist)?;
                (packed(runs)? < needs).then(|| Unavailable {
                    needs: dotted(needs),
                    runs: runs.to_owned(),
                })
            })
            .as_ref()
    }
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    None
}

/// Where an Intel Mac loads the runtime from: `ORT_DYLIB_PATH` when it is
/// set, which is how a build run from the source tree finds one; the
/// bundle's Frameworks, where the release puts it; beside the executable;
/// and failing those, the bare name, for dyld's own search.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub fn library() -> PathBuf {
    const NAME: &str = "libonnxruntime.dylib";
    if let Some(path) = std::env::var_os("ORT_DYLIB_PATH").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    if let Some(dir) = beside {
        for candidate in [dir.join("../Frameworks").join(NAME), dir.join(NAME)] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(NAME)
}

/// The Mach-O header and load commands at the front of `path`: enough to
/// read its minimum, without reading the rest of a library that is tens of
/// megabytes.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn header(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 32];
    file.read_exact(&mut head).ok()?;
    let commands = u32::from_le_bytes(head[20..24].try_into().ok()?) as usize;
    let mut rest = vec![0u8; commands.min(1 << 20)];
    file.read_exact(&mut rest).ok()?;
    head.extend_from_slice(&rest);
    Some(head)
}

/// The minimum macOS a 64-bit Mach-O file names for itself, packed the
/// way Mach-O packs it (`0xMMMMmmpp`): its `LC_BUILD_VERSION`'s `minos`,
/// or an older file's `LC_VERSION_MIN_MACOSX`. `None` for anything else,
/// a universal file among them.
fn minimum_macos(bytes: &[u8]) -> Option<u32> {
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const LC_VERSION_MIN_MACOSX: u32 = 0x24;
    const LC_BUILD_VERSION: u32 = 0x32;
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if word(0)? != MH_MAGIC_64 {
        return None;
    }
    let mut at = 32;
    for _ in 0..word(16)? {
        let (command, size) = (word(at)?, word(at + 4)? as usize);
        match command {
            LC_BUILD_VERSION => return word(at + 12),
            LC_VERSION_MIN_MACOSX => return word(at + 8),
            _ => {}
        }
        if size < 8 {
            return None;
        }
        at += size;
    }
    None
}

/// `ProductVersion` from SystemVersion.plist's XML.
fn product_version(plist: &str) -> Option<&str> {
    let after = &plist[plist.find("<key>ProductVersion</key>")?..];
    let start = after.find("<string>")? + "<string>".len();
    let end = start + after[start..].find("</string>")?;
    Some(after[start..end].trim())
}

/// `12.7.6` packed as Mach-O packs a version, to compare with one.
fn packed(version: &str) -> Option<u32> {
    let mut parts = version.split('.').map(str::parse::<u32>);
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major << 16) | (minor.min(0xff) << 8) | patch.min(0xff))
}

/// A packed version as people write it: `13.4`, or `13.4.1`.
fn dotted(version: u32) -> String {
    let (major, minor, patch) = (version >> 16, (version >> 8) & 0xff, version & 0xff);
    if patch == 0 {
        format!("{major}.{minor}")
    } else {
        format!("{major}.{minor}.{patch}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mach-O header with one unrelated command ahead of the one asked
    /// about, the way a real file has a dozen.
    fn file(command: u32, version_at: usize, version: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let words = |bytes: &mut Vec<u8>, words: &[u32]| {
            for word in words {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        };
        // magic, cputype, cpusubtype, filetype, ncmds, sizeofcmds, flags, reserved
        words(&mut bytes, &[0xfeed_facf, 0x0100_0007, 3, 6, 2, 40, 0, 0]);
        // LC_UUID: cmd, cmdsize, sixteen bytes.
        words(&mut bytes, &[0x1b, 24, 0, 0, 0, 0]);
        let mut found = vec![command, 16, 1, 0];
        found[version_at / 4] = version;
        words(&mut bytes, &found);
        bytes
    }

    #[test]
    fn the_minimum_is_read_from_either_command() {
        assert_eq!(
            minimum_macos(&file(0x32, 12, 0x000d_0400)),
            Some(0x000d_0400)
        );
        assert_eq!(
            minimum_macos(&file(0x24, 8, 0x000a_0e00)),
            Some(0x000a_0e00)
        );
        assert_eq!(minimum_macos(&file(0x1d, 8, 1)), None);
        assert_eq!(minimum_macos(b"\xca\xfe\xba\xbe"), None);
        assert_eq!(minimum_macos(&[]), None);
    }

    #[test]
    fn versions_compare_the_way_macos_numbers_them() {
        assert_eq!(packed("13.4"), Some(0x000d_0400));
        assert!(packed("12.7.6") < packed("13.4"));
        assert!(packed("13.4.1") >= packed("13.4"));
        assert!(packed("26.0") > packed("13.4"));
        assert_eq!(packed("twelve"), None);
        assert_eq!(dotted(0x000d_0400), "13.4");
        assert_eq!(dotted(0x000c_0706), "12.7.6");
    }

    #[test]
    fn the_running_version_is_read_from_the_plist() {
        let plist = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t\
                     <key>ProductVersion</key>\n\t<string>12.7.6</string>\n</dict>";
        assert_eq!(product_version(plist), Some("12.7.6"));
        assert_eq!(product_version("<dict></dict>"), None);
    }

    /// The test binary itself, on a Mac: a real file's header parses.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_real_binary_names_its_minimum() {
        let bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let minimum = minimum_macos(&bytes).expect("a minimum");
        assert!(minimum >= 0x000a_0000, "{}", dotted(minimum));
    }
}
