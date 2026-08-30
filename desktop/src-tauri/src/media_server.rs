//! A loopback HTTP server for preview media, for Linux only.
//!
//! WebKitGTK cannot play `<video>` from a custom URI scheme. It hands the URL
//! straight to GStreamer, which has no source element for `asset://`, so the
//! media fails to load before `webkitwebsrc` ever runs - the element reports
//! `MEDIA_ERR_SRC_NOT_SUPPORTED` and the preview stays black while audio,
//! which the engine decodes itself, plays on. Registering the scheme as
//! CORS-enabled and secure does not help; the same file plays immediately
//! over `file://` or `http://`.
//!
//! So on Linux the preview asks for media over `http://127.0.0.1`, and the
//! asset protocol keeps serving everything else - stills and artwork are
//! `<img>`, which has never had the problem. macOS and Windows do not start
//! this server at all.
//!
//! The scope discipline from `grant_asset` carries over: the server answers
//! for a path only after that path was admitted, and only with the token
//! minted at startup, so another process on the machine cannot read a user's
//! media by guessing the port.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How much of a file is moved per write while streaming a response.
const CHUNK: usize = 64 * 1024;

/// The loopback media server: a port, a token, and the set of files it will
/// serve.
pub(crate) struct MediaServer {
    port: u16,
    token: String,
    allowed: Arc<Mutex<HashSet<PathBuf>>>,
}

/// What the webview needs to build a media URL. `None` on platforms whose
/// webview plays the asset protocol correctly.
#[derive(serde::Serialize)]
pub(crate) struct Endpoint {
    /// The loopback port the server accepted on.
    pub port: u16,
    /// The per-run token every request must carry.
    pub token: String,
}

impl MediaServer {
    /// Starts the server on a free loopback port.
    fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let allowed: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));

        let serving = Arc::clone(&allowed);
        let expected = token.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let allowed = Arc::clone(&serving);
                let expected = expected.clone();
                // A thread per connection: WebKit opens a handful for
                // seeking, and a blocking handler is far less machinery than
                // pulling an async runtime in for three routes.
                std::thread::spawn(move || {
                    let _ = handle(stream, &allowed, &expected);
                });
            }
        });

        Ok(Self {
            port,
            token,
            allowed,
        })
    }

    /// What the webview should talk to.
    pub(crate) fn endpoint(&self) -> Endpoint {
        Endpoint {
            port: self.port,
            token: self.token.clone(),
        }
    }

    /// Admits one file, mirroring the asset protocol scope.
    pub(crate) fn allow(&self, path: &str) {
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        if let Ok(mut allowed) = self.allowed.lock() {
            allowed.insert(resolved);
        }
    }
}

/// Starts the server where the webview needs it, or explains why it did not.
///
/// A failure here is not fatal: the preview falls back to the asset protocol,
/// which is what the other platforms use anyway.
pub(crate) fn start() -> Option<MediaServer> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    match MediaServer::start() {
        Ok(server) => Some(server),
        Err(error) => {
            eprintln!("wolfcut: preview media server unavailable: {error}");
            None
        }
    }
}

/// Answers one request.
fn handle(
    mut stream: TcpStream,
    allowed: &Mutex<HashSet<PathBuf>>,
    expected: &str,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request = String::new();
    reader.read_line(&mut request)?;
    let target = match request.split_whitespace().nth(1) {
        Some(target) => target.to_owned(),
        None => return refuse(&mut stream, 400, "Bad Request"),
    };

    // Only the Range header matters; the rest is read to keep the client from
    // blocking on an unread request body.
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Range:").or(line.strip_prefix("range:")) {
            range = Some(value.trim().to_owned());
        }
    }

    let (path, token) = match parse_target(&target) {
        Some(parsed) => parsed,
        None => return refuse(&mut stream, 400, "Bad Request"),
    };

    if token != expected {
        return refuse(&mut stream, 403, "Forbidden");
    }

    let resolved = match std::fs::canonicalize(&path) {
        Ok(resolved) => resolved,
        Err(_) => return refuse(&mut stream, 404, "Not Found"),
    };

    let admitted = allowed
        .lock()
        .map(|allowed| allowed.contains(&resolved))
        .unwrap_or(false);
    if !admitted {
        return refuse(&mut stream, 403, "Forbidden");
    }

    send(&mut stream, &resolved, range.as_deref())
}

/// Pulls the media path and the token out of `/media?t=...&path=...`.
fn parse_target(target: &str) -> Option<(String, String)> {
    let query = target.split_once('?')?.1;
    let mut path = None;
    let mut token = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "path" => path = Some(percent_decode(value)),
            "t" => token = Some(percent_decode(value)),
            _ => {}
        }
    }
    Some((path?, token?))
}

/// Decodes the percent-escapes `encodeURIComponent` produces.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Writes the file, whole or as the requested range.
fn send(stream: &mut TcpStream, path: &Path, range: Option<&str>) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let kind = content_type(path);

    let (start, end) = match range.and_then(|range| parse_range(range, length)) {
        Some(bounds) => bounds,
        None => (0, length.saturating_sub(1)),
    };

    if length == 0 || start > end || start >= length {
        let head = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{length}\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return stream.write_all(head.as_bytes());
    }

    let count = end - start + 1;
    let head = if range.is_some() {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: {kind}\r\n\
             Content-Length: {count}\r\nAccept-Ranges: bytes\r\n\
             Content-Range: bytes {start}-{end}/{length}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {count}\r\n\
             Accept-Ranges: bytes\r\nConnection: close\r\n\r\n"
        )
    };
    stream.write_all(head.as_bytes())?;

    file.seek(SeekFrom::Start(start))?;
    let mut remaining = count;
    let mut buffer = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = remaining.min(CHUNK as u64) as usize;
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        // A seek that moved on leaves the old response half-written; the
        // client hanging up is normal, not an error worth reporting.
        if stream.write_all(&buffer[..read]).is_err() {
            return Ok(());
        }
        remaining -= read as u64;
    }
    Ok(())
}

/// Reads `bytes=start-end`, `bytes=start-` and `bytes=-suffix`.
fn parse_range(header: &str, length: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        let suffix = suffix.min(length);
        return Some((length.saturating_sub(suffix), length.saturating_sub(1)));
    }
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() {
        length.saturating_sub(1)
    } else {
        end.parse::<u64>().ok()?.min(length.saturating_sub(1))
    };
    Some((start, end))
}

/// A media type the webview will accept, guessed from the extension.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" | "qt" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mpg" | "mpeg" => "video/mpeg",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        _ => "application/octet-stream",
    }
}

/// A bodyless refusal.
fn refuse(stream: &mut TcpStream, code: u16, reason: &str) -> std::io::Result<()> {
    let head =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes())
}

/// A per-run token, from the process-random seed `RandomState` already keeps.
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut token = String::with_capacity(32);
    for salt in 0..2u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u32(std::process::id());
        token.push_str(&format!("{:016x}", hasher.finish()));
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_the_forms_webkit_sends() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=-20", 100), Some((80, 99)));
        // An end past the file is clamped rather than refused.
        assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(parse_range("chunks=0-", 100), None);
    }

    #[test]
    fn target_yields_path_and_token() {
        let target = "/media?t=abc&path=%2Ftmp%2Fa%20b.mov";
        assert_eq!(
            parse_target(target),
            Some(("/tmp/a b.mov".to_owned(), "abc".to_owned()))
        );
        assert_eq!(parse_target("/media"), None);
    }

    #[test]
    fn token_is_not_constant() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 32);
    }
}
