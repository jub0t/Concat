// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Making extras: a folder into a `.concatpkg`, archives into a repository
//! index, and the Ed25519 key that signs it. What `concat-cli extras`
//! runs, and what the tests build their fixtures with.
//!
//! An author writes `extra.toml` without any `[[file]]` tables and lays the
//! files out under the roots it names; [`pack`] lists every file with its
//! size and digest, checks the result as an install would, and writes the
//! archive. The index is then written from the archives ([`index_entry`],
//! [`write_index`]) and signed ([`sign`]) with a key from [`keygen`].

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Error;
use crate::archive;
use crate::index::{Download, Entry, Index, SCHEMA};
use crate::manifest::{FileEntry, MANIFEST_FILE, Manifest};

/// A packed archive.
#[derive(Clone, Debug)]
pub struct Packed {
    /// The manifest as written into it, with its file list.
    pub manifest: Manifest,
    /// The archive's size.
    pub size: u64,
    /// The archive's SHA-256.
    pub sha256: String,
}

/// Packs `folder` - its `extra.toml` and the files beside it - into `out`.
/// The file list in the manifest is replaced by what is in the folder.
/// Hidden files (a name starting with `.`) are left out.
pub fn pack(folder: &Path, out: &Path) -> Result<Packed, Error> {
    let manifest_path = folder.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| Error::io(&manifest_path, error))?;
    let mut manifest: Manifest = toml::from_str(&text)
        .map_err(|error| Error::Invalid(format!("{MANIFEST_FILE} does not parse: {error}")))?;
    let mut files = Vec::new();
    walk(folder, folder, &mut files)?;
    files.sort();
    manifest.files = Vec::with_capacity(files.len());
    for relative in files {
        if relative == MANIFEST_FILE {
            continue;
        }
        let path = archive::join(folder, &relative);
        let bytes = std::fs::read(&path).map_err(|error| Error::io(&path, error))?;
        manifest.files.push(FileEntry {
            size: bytes.len() as u64,
            sha256: crate::hex(&Sha256::digest(&bytes)),
            path: relative,
        });
    }
    manifest.validate()?;
    archive::write(folder, &manifest, out)?;
    let size = std::fs::metadata(out)
        .map_err(|error| Error::io(out, error))?
        .len();
    let sha256 = archive::sha256_file(out, &std::sync::atomic::AtomicBool::new(false))?;
    Ok(Packed {
        manifest,
        size,
        sha256,
    })
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|error| Error::io(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(dir, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| Error::io(&path, error))?;
        if kind.is_symlink() {
            return Err(Error::Invalid(format!(
                "{} is a link; an extra holds only plain files",
                path.display()
            )));
        }
        if kind.is_dir() {
            walk(base, &path, out)?;
        } else {
            let relative = path
                .strip_prefix(base)
                .expect("walked under the base")
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(relative);
        }
    }
    Ok(())
}

/// The index entry for a packed archive, downloadable from `url` (relative
/// to the index, or https://).
pub fn index_entry(packed: &Packed, url: &str) -> Entry {
    let info = &packed.manifest.extra;
    Entry {
        id: info.id.clone(),
        name: info.name.clone(),
        version: info.version.clone(),
        summary: info.summary.clone(),
        description: info.description.clone(),
        author: info.author.clone(),
        licence: info.licence.clone(),
        homepage: info.homepage.clone(),
        category: info.category,
        min_app: info.min_app.clone(),
        platforms: info.platforms.clone(),
        keywords: info.keywords.clone(),
        provides: packed.manifest.kinds(),
        provides_ids: provided_ids(&packed.manifest),
        requires: packed.manifest.requires.clone(),
        installed_size: packed.manifest.total_size(),
        featured: false,
        changelog: String::new(),
        download: Download {
            url: url.to_owned(),
            size: packed.size,
            sha256: packed.sha256.clone(),
        },
    }
}

/// The ids a manifest provides: what it declares, and the folder names of
/// its effect packages, which are their ids.
pub fn provided_ids(manifest: &Manifest) -> Vec<String> {
    let mut ids = manifest.extra.provides_ids.clone();
    for provide in &manifest.provides {
        if provide.kind != crate::Kind::Effects {
            continue;
        }
        for file in &manifest.files {
            let Some(rest) = file.path.strip_prefix(&format!("{}/", provide.root)) else {
                continue;
            };
            if let Some((folder, "effect.toml")) = rest.split_once('/')
                && !ids.iter().any(|id| id == folder)
            {
                ids.push(folder.to_owned());
            }
        }
    }
    ids
}

/// An index's bytes: what [`sign`] signs and a repository publishes. Its
/// serial is now and it never expires; see [`write_index_with`].
pub fn write_index(
    repository: &str,
    revision: &str,
    entries: Vec<Entry>,
) -> Result<Vec<u8>, Error> {
    write_index_with(repository, revision, crate::index::unix_now(), 0, entries)
}

/// [`write_index`] with the serial readers hold it to (raise it on every
/// publish) and when it expires (Unix seconds, 0 for never).
pub fn write_index_with(
    repository: &str,
    revision: &str,
    serial: u64,
    expires: u64,
    entries: Vec<Entry>,
) -> Result<Vec<u8>, Error> {
    let index = Index {
        schema: SCHEMA,
        repository: repository.to_owned(),
        revision: revision.to_owned(),
        serial,
        expires,
        extras: entries,
    };
    let bytes = serde_json::to_vec_pretty(&index).expect("an index always serialises");
    // Checked as a reader would check it, so a bad entry is found here.
    Index::parse(&bytes)?;
    Ok(bytes)
}

/// A new signing key: the secret (a PKCS#8 document, in hex - keep it out
/// of every repository) and the public key (hex) a repository is
/// configured with.
pub fn keygen() -> Result<(String, String), Error> {
    use ring::signature::KeyPair as _;
    let random = ring::rand::SystemRandom::new();
    let document = ring::signature::Ed25519KeyPair::generate_pkcs8(&random).map_err(|_| {
        Error::Invalid("could not make a key: the system gave no randomness".to_owned())
    })?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(document.as_ref())
        .map_err(|_| Error::Invalid("could not read back the key just made".to_owned()))?;
    Ok((
        crate::hex(document.as_ref()),
        crate::hex(pair.public_key().as_ref()),
    ))
}

/// Signs `bytes` with a secret from [`keygen`]; the signature in hex.
pub fn sign(secret: &str, bytes: &[u8]) -> Result<String, Error> {
    let document = crate::unhex(secret)
        .ok_or_else(|| Error::Invalid("the signing key is not hex".to_owned()))?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(&document)
        .map_err(|_| Error::Invalid("the signing key is not an Ed25519 key".to_owned()))?;
    Ok(crate::hex(pair.sign(bytes).as_ref()))
}

/// The public key, in hex, of a secret from [`keygen`].
pub fn public_key(secret: &str) -> Result<String, Error> {
    use ring::signature::KeyPair as _;
    let document = crate::unhex(secret)
        .ok_or_else(|| Error::Invalid("the signing key is not hex".to_owned()))?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(&document)
        .map_err(|_| Error::Invalid("the signing key is not an Ed25519 key".to_owned()))?;
    Ok(crate::hex(pair.public_key().as_ref()))
}

/// Builds a whole repository in `out`: packs every folder in `sources`
/// that holds an `extra.toml`, writes `index.json` beside the archives and
/// signs it into `index.json.sig`. The archives' paths in the index are
/// relative, so the folder can be served from anywhere, or read from disk.
pub fn build_repository(
    sources: &[PathBuf],
    out: &Path,
    repository: &str,
    revision: &str,
    secret: &str,
) -> Result<Vec<Packed>, Error> {
    build_repository_with(
        sources,
        out,
        repository,
        revision,
        secret,
        crate::index::unix_now(),
        0,
    )
}

/// [`build_repository`] with the index's serial and expiry; see
/// [`write_index_with`].
pub fn build_repository_with(
    sources: &[PathBuf],
    out: &Path,
    repository: &str,
    revision: &str,
    secret: &str,
    serial: u64,
    expires: u64,
) -> Result<Vec<Packed>, Error> {
    std::fs::create_dir_all(out).map_err(|error| Error::io(out, error))?;
    let mut packed = Vec::new();
    let mut entries = Vec::new();
    for source in sources {
        let text = std::fs::read_to_string(source.join(MANIFEST_FILE))
            .map_err(|error| Error::io(&source.join(MANIFEST_FILE), error))?;
        let manifest: Manifest = toml::from_str(&text)
            .map_err(|error| Error::Invalid(format!("{}: {error}", source.display())))?;
        let name = format!(
            "{}-{}.{}",
            manifest.extra.id,
            manifest.extra.version,
            archive::EXTENSION
        );
        let one = pack(source, &out.join(&name))?;
        entries.push(index_entry(&one, &name));
        packed.push(one);
    }
    let bytes = write_index_with(repository, revision, serial, expires, entries)?;
    let signature = sign(secret, &bytes)?;
    let index = out.join("index.json");
    std::fs::write(&index, &bytes).map_err(|error| Error::io(&index, error))?;
    let sig = out.join("index.json.sig");
    std::fs::write(&sig, &signature).map_err(|error| Error::io(&sig, error))?;
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_signs_what_its_public_half_verifies() {
        let (secret, public) = keygen().unwrap();
        assert_eq!(public_key(&secret).unwrap(), public);
        let signature = sign(&secret, b"hello").unwrap();
        crate::index::verify(b"hello", &signature, &public).unwrap();
        assert!(crate::index::verify(b"hellp", &signature, &public).is_err());
        assert!(sign("zz", b"x").is_err());
    }

    #[test]
    fn packing_lists_every_file_and_skips_hidden_ones() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("src");
        let look = folder.join("effects").join("me.glow");
        std::fs::create_dir_all(&look).unwrap();
        std::fs::write(look.join("effect.toml"), "x").unwrap();
        std::fs::write(look.join("effect.wgsl"), "y").unwrap();
        std::fs::write(folder.join(".DS_Store"), "junk").unwrap();
        std::fs::write(
            folder.join(MANIFEST_FILE),
            "format = 1\n[extra]\nid = \"me.glow-pack\"\nname = \"Glow\"\nversion = \"0.1.0\"\nauthor = \"Me\"\nlicence = \"MIT\"\n[[provides]]\nkind = \"effects\"\nroot = \"effects\"\n",
        )
        .unwrap();
        let packed = pack(&folder, &temp.path().join("glow.concatpkg")).unwrap();
        let paths: Vec<&str> = packed
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(
            paths,
            ["effects/me.glow/effect.toml", "effects/me.glow/effect.wgsl"]
        );
        assert_eq!(provided_ids(&packed.manifest), ["me.glow"]);
        let entry = index_entry(&packed, "glow.concatpkg");
        assert_eq!(entry.provides, [crate::Kind::Effects]);
        assert_eq!(entry.download.sha256, packed.sha256);

        // A file of a type the root does not take is refused at pack time.
        std::fs::write(look.join("tool.exe"), "MZ").unwrap();
        assert!(pack(&folder, &temp.path().join("bad.concatpkg")).is_err());
    }

    #[test]
    fn a_built_repository_verifies_end_to_end() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("src");
        std::fs::create_dir_all(folder.join("fonts")).unwrap();
        std::fs::write(folder.join("fonts").join("Face.ttf"), "font bytes").unwrap();
        std::fs::write(
            folder.join(MANIFEST_FILE),
            "format = 1\n[extra]\nid = \"me.fonts\"\nname = \"Fonts\"\nversion = \"1.0.0\"\nauthor = \"Me\"\nlicence = \"OFL-1.1\"\ncategory = \"fonts\"\n[[provides]]\nkind = \"fonts\"\nroot = \"fonts\"\n",
        )
        .unwrap();
        let (secret, public) = keygen().unwrap();
        let out = temp.path().join("repo");
        build_repository(&[folder], &out, "Mine", "1", &secret).unwrap();
        let bytes = std::fs::read(out.join("index.json")).unwrap();
        let signature = std::fs::read_to_string(out.join("index.json.sig")).unwrap();
        let index = Index::verify_and_parse(&bytes, &signature, &public).unwrap();
        let entry = index.get("me.fonts").unwrap();
        assert_eq!(entry.download.url, "me.fonts-1.0.0.concatpkg");
        assert!(out.join(&entry.download.url).is_file());
    }
}
