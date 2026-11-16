// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! `.concatpkg`: a gzip'd tar whose first member is `extra.toml`.
//!
//! Reading one is where a hostile file would do its damage, so everything
//! is checked before it is believed, and nothing is unpacked anywhere but
//! a fresh folder of the caller's:
//!
//! * the first member is the manifest, and it validates ([`Manifest::validate`]);
//! * every later member is a plain file or a folder - no links of either
//!   kind, no devices, no FIFOs, no sparse files;
//! * every member's path is one [`crate::manifest::safe_path`] accepts, and
//!   every file is one the manifest lists, at the size it lists, hashing to
//!   the digest it lists, listed once and present;
//! * the decompressed stream may not run past what the manifest says it
//!   holds (plus the tar's own headers), so a small archive cannot inflate
//!   into a full disk;
//! * files are created with `create_new`, so nothing in the archive can
//!   overwrite anything, including another member.
//!
//! Writing one ([`write`]) is deterministic: members in path order, no
//! timestamps, no owners, so packing the same folder twice gives the same
//! bytes and the same digest.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::Error;
use crate::manifest::{self, MANIFEST_FILE, Manifest};

/// The file extension.
pub const EXTENSION: &str = "concatpkg";

/// A reader that refuses to give more than its budget, which can be raised
/// once the manifest says how much there should be.
struct Budget<R> {
    inner: R,
    read: Rc<Cell<u64>>,
    limit: Rc<Cell<u64>>,
}

impl<R: Read> Read for Budget<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let total = self.read.get() + n as u64;
        self.read.set(total);
        if total > self.limit.get() {
            return Err(std::io::Error::other(
                "the archive unpacks to more than its manifest says it holds",
            ));
        }
        Ok(n)
    }
}

/// What every member costs in tar beyond its bytes: a header, the padding
/// to a block, and room for a long name's or a pax record's own header.
const PER_MEMBER: u64 = 512 * 4;

/// An open archive, with the counters of its budget: bytes read, and the limit.
type Opened = (
    tar::Archive<Budget<flate2::read::GzDecoder<std::fs::File>>>,
    Rc<Cell<u64>>,
    Rc<Cell<u64>>,
);

fn open(path: &Path) -> Result<Opened, Error> {
    let file = std::fs::File::open(path).map_err(|error| Error::io(path, error))?;
    let read = Rc::new(Cell::new(0));
    let limit = Rc::new(Cell::new(manifest::MAX_MANIFEST_SIZE + PER_MEMBER * 4));
    let budget = Budget {
        inner: flate2::read::GzDecoder::new(file),
        read: read.clone(),
        limit: limit.clone(),
    };
    Ok((tar::Archive::new(budget), read, limit))
}

/// The path a member names, checked; None for a member tar uses for its
/// own bookkeeping.
fn member_path(entry: &tar::Entry<'_, impl Read>) -> Result<Option<(String, bool)>, Error> {
    use tar::EntryType;
    let kind = entry.header().entry_type();
    let directory = match kind {
        EntryType::Regular | EntryType::Continuous => false,
        EntryType::Directory => true,
        // Bookkeeping the tar crate applies itself, or that names nothing.
        EntryType::XGlobalHeader | EntryType::XHeader | EntryType::GNULongName => {
            return Ok(None);
        }
        other => {
            return Err(Error::Invalid(format!(
                "the archive holds a {}, and an extra may only hold plain files and folders",
                match other {
                    EntryType::Symlink => "symbolic link".to_owned(),
                    EntryType::Link | EntryType::GNULongLink => "hard link".to_owned(),
                    EntryType::Char | EntryType::Block => "device".to_owned(),
                    EntryType::Fifo => "pipe".to_owned(),
                    EntryType::GNUSparse => "sparse file".to_owned(),
                    other => format!("member of type {other:?}"),
                }
            )));
        }
    };
    let bytes = entry.path_bytes();
    let path = std::str::from_utf8(&bytes).map_err(|_| {
        Error::Invalid("the archive names a file in something other than UTF-8".to_owned())
    })?;
    let path = path.strip_prefix("./").unwrap_or(path);
    let path = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    if directory && path.is_empty() {
        return Ok(None);
    }
    manifest::safe_path(path)
        .map_err(|why| Error::Invalid(format!("the archive's member `{path}` {why}")))?;
    Ok(Some((path.to_owned(), directory)))
}

fn read_manifest_member(entry: &mut tar::Entry<'_, impl Read>) -> Result<Manifest, Error> {
    let size = entry.header().size().unwrap_or(u64::MAX);
    if size > manifest::MAX_MANIFEST_SIZE {
        return Err(Error::Invalid(format!("{MANIFEST_FILE} is too large")));
    }
    let mut text = String::new();
    entry
        .take(manifest::MAX_MANIFEST_SIZE)
        .read_to_string(&mut text)
        .map_err(|error| Error::Invalid(format!("{MANIFEST_FILE} could not be read: {error}")))?;
    Manifest::parse(&text)
}

/// The manifest of an archive, read from its first member and validated -
/// which says what the archive claims to be, and nothing about whether to
/// believe it: only [`extract`] checks the rest.
pub fn read_manifest(path: &Path) -> Result<Manifest, Error> {
    let (mut archive, _, _) = open(path)?;
    let mut entries = archive
        .entries()
        .map_err(|error| Error::Invalid(format!("not an extra archive: {error}")))?;
    loop {
        let mut entry = match entries.next() {
            Some(Ok(entry)) => entry,
            Some(Err(error)) => {
                return Err(Error::Invalid(format!("not an extra archive: {error}")));
            }
            None => {
                return Err(Error::Invalid(format!(
                    "the archive holds no {MANIFEST_FILE}"
                )));
            }
        };
        match member_path(&entry)? {
            None => continue,
            Some((path, false)) if path == MANIFEST_FILE => {
                return read_manifest_member(&mut entry);
            }
            Some(_) => {
                return Err(Error::Invalid(format!(
                    "the archive's first member must be {MANIFEST_FILE}"
                )));
            }
        }
    }
}

/// Unpacks the archive at `path` into `dest`, which must not exist yet,
/// checking everything the module says. On any failure `dest` is removed
/// again, so nothing half unpacked is ever left for something to find.
/// `progress` is called with (bytes unpacked, bytes in all).
pub fn extract(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Manifest, Error> {
    if dest.exists() {
        return Err(Error::io(dest, "the staging folder is already there"));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    std::fs::create_dir(dest).map_err(|error| Error::io(dest, error))?;
    let result = extract_into(path, dest, cancel, progress);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(dest);
    }
    result
}

fn extract_into(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Manifest, Error> {
    let (mut archive, read, limit) = open(path)?;
    let corrupt =
        |error: std::io::Error| Error::Invalid(format!("the archive is damaged: {error}"));
    let mut entries = archive.entries().map_err(corrupt)?;

    // The manifest, first.
    let manifest = loop {
        let mut entry = match entries.next() {
            Some(entry) => entry.map_err(corrupt)?,
            None => {
                return Err(Error::Invalid(format!(
                    "the archive holds no {MANIFEST_FILE}"
                )));
            }
        };
        match member_path(&entry)? {
            None => continue,
            Some((name, false)) if name == MANIFEST_FILE => {
                break read_manifest_member(&mut entry)?;
            }
            Some(_) => {
                return Err(Error::Invalid(format!(
                    "the archive's first member must be {MANIFEST_FILE}"
                )));
            }
        }
    };
    let total = manifest.total_size();
    limit.set(read.get() + total + PER_MEMBER * (manifest.files.len() as u64 * 2 + 16) + 64 * 1024);
    let mut wanted: HashMap<&str, &manifest::FileEntry> = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let mut done: u64 = 0;
    progress(0, total);

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut entry = entry.map_err(corrupt)?;
        let Some((name, directory)) = member_path(&entry)? else {
            continue;
        };
        if directory {
            // A folder is only made for a file that needs it; one the
            // manifest has no file under is clutter, and refused.
            if !manifest
                .files
                .iter()
                .any(|file| manifest::nested(&name, &file.path) && file.path != name)
            {
                return Err(Error::Invalid(format!(
                    "the archive holds a folder `{name}` with nothing the manifest lists in it"
                )));
            }
            continue;
        }
        let Some(file) = wanted.remove(name.as_str()) else {
            return Err(Error::Invalid(if name == MANIFEST_FILE {
                format!("the archive holds {MANIFEST_FILE} twice")
            } else if manifest.files.iter().any(|file| file.path == name) {
                format!("the archive holds `{name}` twice")
            } else {
                format!("the archive holds `{name}`, which its manifest does not list")
            }));
        };
        let size = entry.header().size().map_err(corrupt)?;
        if size != file.size {
            return Err(Error::Integrity(format!(
                "`{name}` is {size} bytes in the archive; its manifest says {}",
                file.size
            )));
        }
        let target = join(dest, &name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| Error::io(&target, error))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut written: u64 = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = entry.read(&mut buffer).map_err(corrupt)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > file.size {
                return Err(Error::Integrity(format!(
                    "`{name}` is longer than its manifest says"
                )));
            }
            hasher.update(&buffer[..n]);
            out.write_all(&buffer[..n])
                .map_err(|error| Error::io(&target, error))?;
            done += n as u64;
            progress(done, total);
        }
        out.flush().map_err(|error| Error::io(&target, error))?;
        drop(out);
        if written != file.size {
            return Err(Error::Integrity(format!(
                "`{name}` is shorter than its manifest says"
            )));
        }
        let digest = crate::hex(&hasher.finalize());
        if digest != file.sha256 {
            return Err(Error::Integrity(format!(
                "`{name}` is not the file its manifest vouches for: its SHA-256 is {digest}"
            )));
        }
    }
    if let Some(name) = wanted.keys().next() {
        return Err(Error::Invalid(format!(
            "the archive is missing `{name}`, which its manifest lists"
        )));
    }
    // The manifest itself, beside what it describes.
    let manifest_path = dest.join(MANIFEST_FILE);
    std::fs::write(&manifest_path, manifest.to_toml())
        .map_err(|error| Error::io(&manifest_path, error))?;
    progress(total, total);
    Ok(manifest)
}

/// A checked relative path joined onto `base`, one part at a time.
pub fn join(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
    }
    path
}

/// Writes `manifest` and the files it lists, read from `folder`, into the
/// archive `out`: the manifest first, then the files in path order.
pub fn write(folder: &Path, manifest: &Manifest, out: &Path) -> Result<(), Error> {
    manifest.validate()?;
    let partial = out.with_extension(format!("{EXTENSION}.partial"));
    let file = std::fs::File::create(&partial).map_err(|error| Error::io(&partial, error))?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    let append = |builder: &mut tar::Builder<_>, name: &str, size: u64, reader: &mut dyn Read| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(size);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        builder
            .append_data(&mut header, name, reader)
            .map_err(|error| Error::io(&partial, error))
    };
    let text = manifest.to_toml();
    append(
        &mut builder,
        MANIFEST_FILE,
        text.len() as u64,
        &mut text.as_bytes(),
    )?;
    let mut files: Vec<&manifest::FileEntry> = manifest.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for entry in files {
        let source = join(folder, &entry.path);
        let mut reader = std::fs::File::open(&source).map_err(|error| Error::io(&source, error))?;
        append(&mut builder, &entry.path, entry.size, &mut reader)?;
    }
    let encoder = builder
        .into_inner()
        .map_err(|error| Error::io(&partial, error))?;
    let file = encoder
        .finish()
        .map_err(|error| Error::io(&partial, error))?;
    file.sync_all()
        .map_err(|error| Error::io(&partial, error))?;
    drop(file);
    std::fs::rename(&partial, out).map_err(|error| Error::io(out, error))
}

/// The SHA-256 of a file, as lowercase hex, stopping when `cancel` is set.
pub fn sha256_file(path: &Path, cancel: &AtomicBool) -> Result<String, Error> {
    let mut file = std::fs::File::open(path).map_err(|error| Error::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = file
            .read(&mut buffer)
            .map_err(|error| Error::io(path, error))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(crate::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileEntry, Kind, Provide};
    use crate::pack;

    /// A folder with a valid extra in it, and its packed archive.
    pub(crate) fn sample(dir: &Path, id: &str) -> (PathBuf, Manifest) {
        let folder = dir.join(format!("{id}-src"));
        std::fs::create_dir_all(folder.join("effects").join(format!("{id}.look"))).unwrap();
        std::fs::write(
            folder
                .join("effects")
                .join(format!("{id}.look"))
                .join("effect.toml"),
            "format = 2\n",
        )
        .unwrap();
        std::fs::write(folder.join("README.md"), "# readme\n").unwrap();
        std::fs::write(
            folder.join(MANIFEST_FILE),
            format!(
                "format = 1\n[extra]\nid = \"{id}\"\nname = \"Pack\"\nversion = \"1.0.0\"\nauthor = \"A\"\nlicence = \"MIT\"\n\n[[provides]]\nkind = \"effects\"\nroot = \"effects\"\n"
            ),
        )
        .unwrap();
        let out = dir.join(format!("{id}.{EXTENSION}"));
        let packed = pack::pack(&folder, &out).expect("packs");
        (out, packed.manifest)
    }

    /// Writes a raw tar.gz with these members, for archives `write` would
    /// never make.
    fn raw(out: &Path, members: &[(&str, tar::EntryType, &[u8])]) {
        let file = std::fs::File::create(out).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        for (name, kind, data) in members {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            // Set the raw name so a hostile path is not normalised away.
            let bytes = name.as_bytes();
            header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn manifest_for(path: &str, data: &[u8]) -> String {
        let manifest = Manifest {
            files: vec![FileEntry {
                path: path.to_owned(),
                size: data.len() as u64,
                sha256: crate::hex(&Sha256::digest(data)),
            }],
            provides: vec![Provide {
                kind: Kind::Effects,
                root: "effects".to_owned(),
            }],
            ..crate::manifest::tests::manifest("a.b")
        };
        manifest.to_toml()
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn a_packed_extra_unpacks_to_exactly_what_it_lists() {
        let temp = tempfile::tempdir().unwrap();
        let (archive, manifest) = sample(temp.path(), "a.pack");
        assert_eq!(read_manifest(&archive).unwrap(), manifest);
        let dest = temp.path().join("out");
        let mut last = (0, 0);
        let got = extract(&archive, &dest, &never(), &mut |done, all| {
            last = (done, all)
        })
        .unwrap();
        assert_eq!(got, manifest);
        assert!(dest.join("effects/a.pack.look/effect.toml").is_file());
        assert!(dest.join("README.md").is_file());
        assert!(dest.join(MANIFEST_FILE).is_file());
        assert_eq!(last.0, last.1);
        // Deterministic: packing again gives the same bytes.
        let first = std::fs::read(&archive).unwrap();
        let (again, _) = sample(temp.path(), "a.pack");
        assert_eq!(std::fs::read(again).unwrap(), first);
    }

    #[test]
    fn a_staging_folder_that_exists_is_not_unpacked_into() {
        let temp = tempfile::tempdir().unwrap();
        let (archive, _) = sample(temp.path(), "a.pack");
        let dest = temp.path().join("taken");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("mine.txt"), "keep").unwrap();
        assert!(extract(&archive, &dest, &never(), &mut |_, _| {}).is_err());
        assert!(
            dest.join("mine.txt").is_file(),
            "nothing of the caller's is touched"
        );
    }

    fn refuses(members: &[(&str, tar::EntryType, &[u8])], needle: &str) {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("x.concatpkg");
        raw(&archive, members);
        let dest = temp.path().join("out");
        let error = extract(&archive, &dest, &never(), &mut |_, _| {})
            .expect_err(needle)
            .to_string();
        assert!(error.contains(needle), "wanted {needle:?} in {error:?}");
        assert!(!dest.exists(), "a refused archive leaves nothing behind");
        assert!(!temp.path().join("etc").exists());
    }

    #[test]
    fn hostile_archives_are_refused_and_leave_nothing() {
        use tar::EntryType::{Directory, Regular, Symlink};
        let data: &[u8] = b"data";
        let good = manifest_for("effects/a.b/effect.toml", data);
        let m = good.as_bytes();
        refuses(
            &[(MANIFEST_FILE, Regular, m), ("../evil.toml", Regular, data)],
            "leaves its folder",
        );
        refuses(
            &[(MANIFEST_FILE, Regular, m), ("/etc/evil", Regular, data)],
            "absolute",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects\\..\\x", Regular, data),
            ],
            "character",
        );
        refuses(
            &[(MANIFEST_FILE, Regular, m), ("C:/x", Regular, data)],
            "character",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects/a.b/effect.toml", Symlink, b""),
            ],
            "symbolic link",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects/other.toml", Regular, data),
            ],
            "does not list",
        );
        refuses(&[(MANIFEST_FILE, Regular, m)], "missing");
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects/a.b/effect.toml", Regular, b"DATA"),
            ],
            "not the file",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects/a.b/effect.toml", Regular, b"longer"),
            ],
            "bytes in the archive",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("effects/a.b/effect.toml", Regular, data),
                ("effects/a.b/effect.toml", Regular, data),
            ],
            "twice",
        );
        refuses(
            &[
                (MANIFEST_FILE, Regular, m),
                ("junk", Directory, b""),
                ("effects/a.b/effect.toml", Regular, data),
            ],
            "folder",
        );
        refuses(
            &[
                ("effects/a.b/effect.toml", Regular, data),
                (MANIFEST_FILE, Regular, m),
            ],
            "first member",
        );
        refuses(
            &[(MANIFEST_FILE, Regular, b"not toml at all [")],
            "does not parse",
        );
    }

    #[test]
    fn something_that_is_not_an_archive_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("x.concatpkg");
        std::fs::write(&path, b"this is not gzip").unwrap();
        assert!(read_manifest(&path).is_err());
        assert!(extract(&path, &temp.path().join("out"), &never(), &mut |_, _| {}).is_err());
    }

    #[test]
    fn an_archive_that_inflates_past_its_manifest_is_stopped() {
        // A manifest that lists one small file, then a member far larger
        // than every limit it declares: the budget stops the read before
        // the size check would even see it.
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("bomb.concatpkg");
        let manifest = manifest_for("effects/a.b/effect.toml", b"data");
        let zeros = vec![0u8; 8 * 1024 * 1024];
        raw(
            &archive,
            &[
                (MANIFEST_FILE, tar::EntryType::Regular, manifest.as_bytes()),
                ("effects/a.b/effect.toml", tar::EntryType::Regular, &zeros),
            ],
        );
        assert!(
            std::fs::metadata(&archive).unwrap().len() < 64 * 1024,
            "it compresses small"
        );
        let error = extract(&archive, &temp.path().join("out"), &never(), &mut |_, _| {})
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("bytes in the archive") || error.contains("more than its manifest"),
            "{error}"
        );
    }

    #[test]
    fn a_cancel_stops_the_unpacking_and_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let (archive, _) = sample(temp.path(), "a.pack");
        let cancel = AtomicBool::new(true);
        let dest = temp.path().join("out");
        assert!(matches!(
            extract(&archive, &dest, &cancel, &mut |_, _| {}),
            Err(Error::Cancelled)
        ));
        assert!(!dest.exists());
        assert!(matches!(
            sha256_file(&archive, &cancel),
            Err(Error::Cancelled)
        ));
    }
}
