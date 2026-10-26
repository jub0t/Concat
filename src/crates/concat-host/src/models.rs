// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Where a downloadable model is fetched from, and what proves it arrived
//! whole.
//!
//! Concat carries no weights in its bundle: a cutout network, a Kokoro voice
//! bank and a whisper model together outweigh the editor many times over,
//! and most people never ask for all three. Each is fetched the first time a
//! feature needs one.
//!
//! Every model Concat offers is mirrored onto a release of its own
//! repository, and the mirror is what a download asks for first. Upstream -
//! the third party the mirror was filled from - is the second try and
//! nothing more: a model the editor offers should not stop existing because
//! somebody else moved a path, and the bytes that arrive should be the bytes
//! that were tested.
//!
//! The table of what is mirrored is `models/manifest.toml` at the root of
//! the repository, and `scripts/models.py --check` is what keeps it and the
//! engine's own tables saying the same thing. Every row carries the SHA-256
//! of its bytes, and a download is refused without one to check against;
//! Hugging Face upstreams are pinned to a commit, since `main` moves. The tables are here rather
//! than parsed from that file because a model's identity is something the
//! engine should not be able to start without.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::net::Cause;

/// The repository the mirror lives on.
pub const REPO: &str = "jub0t/Concat";

/// The release every model is mirrored on.
///
/// Bumped only when a model's bytes change - app releases point at it by
/// name, so an unchanged model is never re-uploaded. Must match `release`
/// in `models/manifest.toml`; the check script enforces that.
pub const RELEASE: &str = "models-v1";

/// Where `file` is mirrored: Concat's own release.
pub fn mirror(file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{RELEASE}/{file}")
}

/// The Audio page's Concat sounds: the list of them, read the first time
/// the shelf is asked for and never at startup. The schema, and where the
/// files go, is `docs/AUDIO_LIBRARY.md`.
pub const SOUNDS_MANIFEST: &str =
    "https://raw.githubusercontent.com/concatapp/assets/main/audio/manifest.json";

/// The repository the sounds live in.
pub const SOUNDS_REPO: &str = "concatapp/assets";

/// The release the sounds are mirrored on, so a file moved in the
/// repository does not take a sound off the shelf.
pub const SOUNDS_RELEASE: &str = "audio-v1";

/// Where to try for a sound's file, in order: the release, then the
/// repository. The bytes are checked against the manifest's digest
/// whichever answers.
pub fn sound_sources(file: &str) -> Vec<String> {
    vec![
        format!("https://github.com/{SOUNDS_REPO}/releases/download/{SOUNDS_RELEASE}/{file}"),
        format!("https://raw.githubusercontent.com/{SOUNDS_REPO}/main/audio/{file}"),
    ]
}

/// A Hugging Face repository holding the same mirror, when there is one -
/// "owner/name". Hugging Face is what hf-mirror.com mirrors in turn, which
/// is how the models reach the people GitHub's downloads do not. Empty
/// until the mirror workflow fills one.
pub const HF_REPO: &str = "";

/// The host that mirrors Hugging Face for mainland China.
const HF_MIRROR_HOST: &str = "https://hf-mirror.com/";

/// `url`, served by hf-mirror.com instead, for a Hugging Face URL; `None`
/// for any other host, which that mirror does not carry.
pub fn hf_mirror(url: &str) -> Option<String> {
    url.strip_prefix("https://huggingface.co/")
        .or_else(|| url.strip_prefix("https://hf.co/"))
        .map(|rest| format!("{HF_MIRROR_HOST}{rest}"))
}

/// Which source a download tries first. The rest still follow: a
/// preference is where to look first, never the only place to look.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SourcePreference {
    /// Concat's mirror, then Hugging Face, then upstream, then hf-mirror.
    #[default]
    Auto,
    /// Concat's own release first.
    Mirror,
    /// hf-mirror.com first: for mainland China, where GitHub's downloads
    /// crawl and huggingface.co needs a VPN, and hf-mirror.com does not.
    HfMirror,
    /// The third party each model was mirrored from, first.
    Upstream,
    /// A base URL of one's own, with the model's file name appended: a
    /// corporate mirror, a self-hosted copy.
    Custom,
}

impl SourcePreference {
    /// Every preference, in the order a menu lists them.
    pub const ALL: [SourcePreference; 5] = [
        SourcePreference::Auto,
        SourcePreference::Mirror,
        SourcePreference::HfMirror,
        SourcePreference::Upstream,
        SourcePreference::Custom,
    ];

    /// The name a preferences file stores.
    pub fn name(self) -> &'static str {
        match self {
            SourcePreference::Auto => "auto",
            SourcePreference::Mirror => "mirror",
            SourcePreference::HfMirror => "hf-mirror",
            SourcePreference::Upstream => "upstream",
            SourcePreference::Custom => "custom",
        }
    }

    /// The preference a stored name means; `Auto` for one nobody stores.
    pub fn parse(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|preference| preference.name() == name)
            .unwrap_or_default()
    }
}

/// The preference in force, and the base URL a custom one points at.
static PREFERENCE: std::sync::RwLock<(SourcePreference, String)> =
    std::sync::RwLock::new((SourcePreference::Auto, String::new()));

/// Sets where downloads look first. From the window, when the preference
/// is read from disk and whenever it is changed.
pub fn set_preference(preference: SourcePreference, custom_base: &str) {
    if let Ok(mut held) = PREFERENCE.write() {
        *held = (
            preference,
            custom_base.trim().trim_end_matches('/').to_owned(),
        );
    }
}

/// Where downloads look first, and the custom base URL.
pub fn preference() -> (SourcePreference, String) {
    PREFERENCE
        .read()
        .map(|held| held.clone())
        .unwrap_or_default()
}

/// Where to try for `file`, in order, for the preference in force.
///
/// Every source that could hold the file is in the list once; the
/// preference only decides which comes first. A mirror that cannot be
/// reached - a proxy that blocks a host, a release still being published,
/// a region that cannot see it - should cost a retry rather than a
/// feature, and the bytes are checked against the same digest whichever
/// source they came from.
pub fn sources(file: &str, upstream: &str) -> Vec<String> {
    let (preference, custom_base) = preference();
    sources_for(file, upstream, preference, &custom_base)
}

fn sources_for(
    file: &str,
    upstream: &str,
    preference: SourcePreference,
    custom_base: &str,
) -> Vec<String> {
    let github = mirror(file);
    let hugging_face = (!HF_REPO.is_empty())
        .then(|| format!("https://huggingface.co/{HF_REPO}/resolve/main/{file}"));
    let custom = (!custom_base.is_empty()).then(|| format!("{custom_base}/{file}"));
    // hf-mirror carries whatever Hugging Face does: our own repository
    // there, and any upstream that lives there.
    let hf_mirrored: Vec<String> = hugging_face
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(upstream))
        .filter_map(hf_mirror)
        .collect();

    let mut ordered: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if !url.is_empty() && !ordered.contains(&url) {
            ordered.push(url);
        }
    };
    match preference {
        SourcePreference::Auto | SourcePreference::Mirror => {}
        SourcePreference::HfMirror => hf_mirrored.iter().cloned().for_each(&mut push),
        SourcePreference::Upstream => push(upstream.to_owned()),
        SourcePreference::Custom => custom.iter().cloned().for_each(&mut push),
    }
    push(github);
    hugging_face.iter().cloned().for_each(&mut push);
    push(upstream.to_owned());
    hf_mirrored.iter().cloned().for_each(&mut push);
    custom.iter().cloned().for_each(&mut push);
    ordered
}

/// Why a model could not be had, for the window to say in the reader's
/// language and to offer what can be done instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Every source was tried and none answered; `cause` is the one that
    /// best explains them all, and `proxy` the proxy they went through.
    Unreachable {
        /// Why, as [`crate::net::get`] classed it.
        cause: Cause,
        /// The proxy the requests went through, as a message names it.
        proxy: Option<String>,
    },
    /// The file the person chose is not the model: its digest is another.
    NotTheFile,
}

/// The last [`fetch`] of each file that failed, and how.
static FAILURES: Mutex<Vec<(String, Failure)>> = Mutex::new(Vec::new());

/// Files the person downloaded themselves, each standing in for the
/// download of the model file it is named for; see [`supply`].
static SUPPLIED: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

/// How the last [`fetch`] of `file` failed, if it did and nothing since
/// has fetched it.
pub fn last_failure(file: &str) -> Option<Failure> {
    let held = FAILURES.lock().ok()?;
    held.iter()
        .find(|(name, _)| name == file)
        .map(|(_, failure)| failure.clone())
}

fn record_failure(file: &str, failure: Option<Failure>) {
    if let Ok(mut held) = FAILURES.lock() {
        held.retain(|(name, _)| name != file);
        if let Some(failure) = failure {
            held.push((file.to_owned(), failure));
        }
    }
}

/// Hands [`fetch`] a copy of `file` the person downloaded themselves - in
/// a browser the network lets through when it does not let Concat - for
/// the next fetch of that file to install instead of downloading.
///
/// It is held to the same digest as a download, so a wrong file is
/// refused rather than installed; and it is copied, not moved, so the
/// person's own copy stays where they put it.
pub fn supply(file: &str, path: PathBuf) {
    if let Ok(mut held) = SUPPLIED.lock() {
        held.retain(|(name, _)| name != file);
        held.push((file.to_owned(), path));
    }
}

fn take_supplied(file: &str) -> Option<PathBuf> {
    let mut held = SUPPLIED.lock().ok()?;
    let at = held.iter().position(|(name, _)| name == file)?;
    Some(held.remove(at).1)
}

/// Fetches the model file `file` into `partial` and checks it: from the
/// copy the person chose with [`supply`], when there is one, or else from
/// each of [`sources`] in turn, taking up from whatever of `partial` an
/// earlier attempt left. The bytes are checked against `sha256` whichever
/// way they came, and a file that hashes wrong is deleted and refused
/// rather than tried again elsewhere, which would only hide which source
/// served it.
///
/// When every source fails, the error names every host tried and the
/// likeliest reason in plain words - a firewall on this computer, no
/// network, a proxy that is not running - rather than whatever the last
/// host said, and [`last_failure`] has it for the window. Each source's
/// own error is in the log.
///
/// `progress` is called with (received, total); `cancelled` is the error
/// to return when `cancel` is set.
pub fn fetch(
    file: &str,
    upstream: &str,
    partial: &Path,
    estimate: u64,
    sha256: &str,
    cancel: &AtomicBool,
    cancelled: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, u64), String> {
    record_failure(file, None);
    if let Some(chosen) = take_supplied(file) {
        return adopt(&chosen, file, partial, sha256, cancel, cancelled, progress);
    }
    let mut missed: Vec<(String, Missed)> = Vec::new();
    for url in sources(file, upstream) {
        match attempt(&url, partial, estimate, cancel, cancelled, progress) {
            Ok(totals) => {
                verify(partial, sha256).inspect_err(|_| {
                    let _ = std::fs::remove_file(partial);
                })?;
                return Ok(totals);
            }
            Err(miss) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(cancelled.to_owned());
                }
                log::warn!("models: {}", miss.message);
                // Not the network's doing - a full disk, a folder that
                // cannot be written - and no other source would fix it.
                if miss.cause.is_none() {
                    return Err(miss.message);
                }
                let host = url
                    .split_once("://")
                    .map_or(url.as_str(), |(_, rest)| rest)
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                missed.push((host, miss));
            }
        }
    }
    let cause =
        Cause::telling(missed.iter().filter_map(|(_, miss)| miss.cause)).unwrap_or(Cause::Other);
    let proxy = missed.iter().find_map(|(_, miss)| miss.proxy.clone());
    record_failure(
        file,
        Some(Failure::Unreachable {
            cause,
            proxy: proxy.clone(),
        }),
    );
    let mut hosts: Vec<&str> = Vec::new();
    for (host, _) in &missed {
        if !hosts.contains(&host.as_str()) {
            hosts.push(host);
        }
    }
    let through = proxy
        .map(|proxy| format!(" through the proxy {proxy}"))
        .unwrap_or_default();
    let last = missed
        .last()
        .map_or("there was nowhere to download it from", |(_, miss)| {
            miss.message.as_str()
        });
    Err(format!(
        "could not download {file} from {}{through}: {}. The last one said: {last}",
        hosts.join(", "),
        cause.describe(),
    ))
}

/// Installs the person's own copy of `file` as if it had been downloaded:
/// copied into `partial` by way of a file of its own, hashed as it is
/// copied, and kept only if the hash is the table's. A download half done
/// in `partial` survives a wrong file.
fn adopt(
    chosen: &Path,
    file: &str,
    partial: &Path,
    sha256: &str,
    cancel: &AtomicBool,
    cancelled: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, u64), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    if sha256.is_empty() {
        return Err(format!(
            "{file} cannot be checked: the model table carries no digest for it"
        ));
    }
    let mut source = std::fs::File::open(chosen)
        .map_err(|error| format!("could not read {}: {error}", chosen.display()))?;
    let total = source.metadata().map(|meta| meta.len()).unwrap_or(0);
    let copy = partial.with_extension("chosen");
    let mut out = std::fs::File::create(&copy)
        .map_err(|error| format!("could not write {}: {error}", copy.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0u64;
    let mut reported = 0u64;
    progress(0, total);
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(cancelled.to_owned());
        }
        let read = match source.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(read) => read,
            Err(error) => break Err(format!("could not read {}: {error}", chosen.display())),
        };
        hasher.update(&buffer[..read]);
        if let Err(error) = out.write_all(&buffer[..read]) {
            break Err(format!("could not write {}: {error}", copy.display()));
        }
        copied += read as u64;
        if copied - reported >= 8 * 1024 * 1024 {
            reported = copied;
            progress(copied, total.max(copied));
        }
    }
    .and_then(|()| {
        out.flush()
            .map_err(|error| format!("could not write {}: {error}", copy.display()))
    });
    drop(out);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&copy);
        return Err(error);
    }
    let actual = hex(&hasher.finalize());
    if actual != sha256 {
        let _ = std::fs::remove_file(&copy);
        record_failure(file, Some(Failure::NotTheFile));
        return Err(format!(
            "{} is not {file}: its sha256 is {actual}, where {file}'s is {sha256}",
            chosen.display()
        ));
    }
    std::fs::rename(&copy, partial)
        .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
    progress(copied, copied);
    Ok((copied, copied))
}

/// One source that did not answer, or stopped answering.
struct Missed {
    message: String,
    /// The network's reason; `None` for a cancel or a local failure.
    cause: Option<Cause>,
    proxy: Option<String>,
}

/// One attempt at one URL, streamed into `partial`, taking up from
/// however much of `partial` is already there. [`attempt`] with its
/// reasons spelled out as text, for a caller that tries its own sources.
pub fn download(
    url: &str,
    partial: &Path,
    estimate: u64,
    cancel: &AtomicBool,
    cancelled: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, u64), String> {
    attempt(url, partial, estimate, cancel, cancelled, progress).map_err(|miss| miss.message)
}

/// One attempt at one URL, streamed into `partial`, taking up from
/// however much of `partial` is already there.
///
/// A model is hundreds of megabytes, and the links this matters most on
/// are the slow ones: a dropped connection at 280 MB of 300 used to start
/// again from nothing. The bytes already on disk are kept and the server
/// is asked for the rest, by a Range request every host on the list
/// honours; a server that answers with the whole file instead is taken
/// at its word and the partial started over. What arrives is checked
/// against the model's digest afterwards, so a partial finished from a
/// second source that happens to hold different bytes is refused there,
/// not trusted here.
///
/// `progress` is called with (received, total) every couple of megabytes.
/// `cancelled` is the error to return when `cancel` is set: the caller's
/// words, since it knows what was being fetched.
fn attempt(
    url: &str,
    partial: &Path,
    estimate: u64,
    cancel: &AtomicBool,
    cancelled: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, u64), Missed> {
    use std::io::{Read, Write};

    let local = |message: String| Missed {
        message,
        cause: None,
        proxy: None,
    };
    let already = std::fs::metadata(partial)
        .map(|meta| meta.len())
        .unwrap_or(0);

    // A read timeout: a stalled connection blocks in `read` with the
    // cancel flag unreachable, and thirty seconds without a byte means
    // the download is dead.
    let response = match crate::net::get(
        url,
        std::time::Duration::from_secs(20),
        std::time::Duration::from_secs(30),
        crate::net::Trust::System,
        &|request| {
            if already > 0 {
                request.set("Range", &format!("bytes={already}-"))
            } else {
                request
            }
        },
    ) {
        Ok(response) => response,
        // Range not satisfiable: everything is already here. Let the
        // caller's digest check say whether it is the file.
        Err(failure) if already > 0 && matches!(failure.error, ureq::Error::Status(416, _)) => {
            return Ok((already, already));
        }
        Err(failure) => {
            return Err(Missed {
                message: format!("{url} did not answer: {failure}"),
                cause: Some(failure.cause),
                proxy: failure.proxy,
            });
        }
    };

    let resumed = response.status() == 206;
    let remaining = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    let (mut received, total) = if resumed {
        (
            already,
            remaining.map_or(estimate.max(already), |rest| already + rest),
        )
    } else {
        (0, remaining.unwrap_or(estimate))
    };
    let mut file = if resumed {
        std::fs::OpenOptions::new().append(true).open(partial)
    } else {
        std::fs::File::create(partial)
    }
    .map_err(|error| local(format!("could not write {}: {error}", partial.display())))?;

    let mut reader = response.into_reader();
    let mut buffer = [0u8; 64 * 1024];
    let mut reported = received;
    progress(received, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            // The partial stays: it is where the next attempt picks up.
            return Err(local(cancelled.to_owned()));
        }
        let read = reader.read(&mut buffer).map_err(|error| Missed {
            message: format!("{url} stopped partway: {error}"),
            cause: Some(crate::net::classify_io(&error)),
            proxy: None,
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| local(format!("could not write {}: {error}", partial.display())))?;
        received += read as u64;
        // Every 2 MB, not every chunk: a progress bar cannot use more.
        if received - reported >= 2 * 1024 * 1024 {
            reported = received;
            progress(received, total);
        }
    }
    file.flush()
        .map_err(|error| local(format!("could not write {}: {error}", partial.display())))?;
    if received == 0 {
        return Err(Missed {
            message: format!("{url} came back empty"),
            cause: Some(Cause::Other),
            proxy: None,
        });
    }
    Ok((received, received.max(total)))
}

/// Checks a finished download against the digest the table carries.
///
/// A file that does not match is refused - the caller deletes it rather
/// than leaving something that looks installed - and so is a download with
/// no digest to check against: a model added to a table is unusable until
/// its digest is there too, rather than trusted until someone gets round
/// to it. The sources include mirrors and a base the user can set, so a
/// download nobody can check is a download anyone can substitute
/// (audit 2026-09-23, #6).
pub fn verify(file: &Path, expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Err(format!(
            "{} cannot be checked: the model table carries no digest for it",
            file.display()
        ));
    }
    let actual = sha256(file)?;
    if actual == expected {
        return Ok(());
    }
    Err(format!(
        "{} is not the file it should be: expected sha256 {expected}, got {actual}",
        file.display()
    ))
}

/// The SHA-256 of a file, as lowercase hex.
fn sha256(file: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut handle = std::fs::File::open(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = handle
            .read(&mut buffer)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Bytes as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mirror_comes_before_upstream() {
        let list = sources_for(
            "model.onnx",
            "https://elsewhere.example/model.onnx",
            SourcePreference::Auto,
            "",
        );
        assert_eq!(
            list[0],
            format!("https://github.com/{REPO}/releases/download/{RELEASE}/model.onnx")
        );
        assert_eq!(list[1], "https://elsewhere.example/model.onnx");
        assert_eq!(
            list.len(),
            2,
            "nothing else holds a file from an unmirrored host"
        );
    }

    #[test]
    fn a_hugging_face_upstream_is_also_offered_through_hf_mirror() {
        let upstream = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
        let list = sources_for("ggml-tiny.bin", upstream, SourcePreference::Auto, "");
        assert_eq!(list.len(), 3);
        assert_eq!(list[1], upstream);
        assert_eq!(
            list[2],
            "https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
        );
        // and first, when that is the preference
        let list = sources_for("ggml-tiny.bin", upstream, SourcePreference::HfMirror, "");
        assert!(list[0].starts_with("https://hf-mirror.com/"));
        assert_eq!(list.len(), 3);
        assert_eq!(hf_mirror("https://github.com/x/y"), None);
    }

    #[test]
    fn a_custom_base_leads_when_asked_and_trails_otherwise() {
        let list = sources_for(
            "model.onnx",
            "https://elsewhere.example/model.onnx",
            SourcePreference::Custom,
            "https://models.example/concat",
        );
        assert_eq!(list[0], "https://models.example/concat/model.onnx");
        assert_eq!(list.len(), 3);
        let list = sources_for(
            "model.onnx",
            "https://elsewhere.example/model.onnx",
            SourcePreference::Upstream,
            "https://models.example/concat",
        );
        assert_eq!(list[0], "https://elsewhere.example/model.onnx");
        assert_eq!(list[2], "https://models.example/concat/model.onnx");
        // a custom preference with no base is a plain automatic
        let list = sources_for("m", "https://u.example/m", SourcePreference::Custom, "");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn preferences_have_stable_names() {
        for preference in SourcePreference::ALL {
            assert_eq!(SourcePreference::parse(preference.name()), preference);
        }
        assert_eq!(SourcePreference::parse("nonsense"), SourcePreference::Auto);
    }

    #[test]
    fn a_partial_download_is_finished_from_a_range() {
        use std::io::{Read, Write};
        // A server that honours Range on one 1 MB file.
        let body: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let served = body.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.expect("a connection");
                let mut head = [0u8; 4096];
                let read = stream.read(&mut head).unwrap_or(0);
                let head = String::from_utf8_lossy(&head[..read]).to_string();
                let from = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Range: bytes="))
                    .and_then(|range| range.trim_end_matches('-').parse::<usize>().ok());
                let (status, slice) = match from {
                    Some(from) if from < served.len() => ("206 Partial Content", &served[from..]),
                    _ => ("200 OK", &served[..]),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    slice.len()
                );
                let _ = stream.write_all(slice);
            }
        });
        let url = format!("http://127.0.0.1:{port}/model.bin");
        let partial = std::env::temp_dir().join("concat-models-resume.part");
        // Half of it is already here, as a dropped connection leaves it.
        std::fs::write(&partial, &body[..600_000]).expect("a partial");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut reports = Vec::new();
        let (received, total) = download(&url, &partial, 0, &cancel, "stopped", &mut |got, of| {
            reports.push((got, of))
        })
        .expect("the rest arrives");
        assert_eq!((received, total), (body.len() as u64, body.len() as u64));
        assert_eq!(
            std::fs::read(&partial).expect("the file"),
            body,
            "the halves join up"
        );
        assert_eq!(
            reports[0],
            (600_000, body.len() as u64),
            "progress starts where the partial ended"
        );
        // A second request with the file complete: whole again, the
        // server having nothing to add, is still the file.
        let (received, _) =
            download(&url, &partial, 0, &cancel, "stopped", &mut |_, _| {}).expect("again");
        assert!(received > 0);
        let _ = std::fs::remove_file(&partial);
    }

    #[test]
    fn a_file_the_person_chose_is_installed_only_if_it_is_the_model() {
        use sha2::{Digest, Sha256};

        let dir = std::env::temp_dir().join(format!("concat-models-chosen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let chosen = dir.join("ggml-test.bin");
        let body = vec![7u8; 3 * 1024 * 1024 + 11];
        std::fs::write(&chosen, &body).expect("the chosen file");
        let digest = hex(&Sha256::digest(&body));
        let partial = dir.join("ggml-test.bin.part");
        // Half a download from before: a wrong file must not cost it.
        std::fs::write(&partial, b"half").expect("a partial");
        let cancel = AtomicBool::new(false);
        // The name is one no table has, and nothing is downloaded: the
        // chosen file stands in for every source.
        let file = "concat-test-chosen.bin";

        supply(file, chosen.clone());
        let wrong = "0".repeat(64);
        let refused = fetch(
            file,
            "",
            &partial,
            0,
            &wrong,
            &cancel,
            "stopped",
            &mut |_, _| {},
        )
        .expect_err("not the model");
        assert!(refused.contains("is not"), "{refused}");
        assert_eq!(last_failure(file), Some(Failure::NotTheFile));
        assert_eq!(std::fs::read(&partial).expect("the partial"), b"half");
        assert!(
            std::fs::exists(&chosen).unwrap_or(false),
            "the person's copy stays"
        );

        supply(file, chosen.clone());
        let mut last = (0, 0);
        let totals = fetch(
            file,
            "",
            &partial,
            0,
            &digest,
            &cancel,
            "stopped",
            &mut |got, of| last = (got, of),
        )
        .expect("the model");
        assert_eq!(totals, (body.len() as u64, body.len() as u64));
        assert_eq!(last, totals, "progress ends full");
        assert_eq!(std::fs::read(&partial).expect("installed"), body);
        assert_eq!(
            last_failure(file),
            None,
            "a fetch that worked clears the failure"
        );
        assert!(
            std::fs::exists(&chosen).unwrap_or(false),
            "copied, not moved"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_digest_is_checked_and_an_empty_one_is_refused() {
        let dir = std::env::temp_dir().join("concat-models-verify");
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let file = dir.join("bytes");
        std::fs::write(&file, b"concat").expect("a temp file");
        // echo -n concat | sha256sum
        let known = "3f4c1a4b3c9fb1b0e24bd1ec0bd7b16d0db4dbf1fe2f3b5dcdba0bd38b8f8b3a";
        assert!(verify(&file, "").is_err(), "nothing to check against");
        assert!(verify(&file, known).is_err());
        let actual = sha256(&file).expect("a digest");
        assert_eq!(actual.len(), 64);
        assert!(verify(&file, &actual).is_ok());
        let _ = std::fs::remove_file(&file);
    }
}
