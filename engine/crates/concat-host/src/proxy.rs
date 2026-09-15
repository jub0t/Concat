// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Small local decode copies for live preview of large source videos.
//!
//! Export and a settled monitor frame always read the original file. A
//! background proxy only replaces the source while playback or a pointer
//! gesture needs low-latency feedback. The clip's source clock, crop and
//! masks remain unchanged.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use concat_export::{ClipKind, ExportClip};

const PROXY_EDGE: u32 = 960;
const PROXY_FPS: u32 = 60;
const PROXY_VERSION: u8 = 1;
const MAX_BUILDING: usize = 2;
static PROXY_JOB_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
pub(crate) struct ProxyStore {
    entries: Arc<Mutex<HashMap<PathBuf, Entry>>>,
}

struct Entry {
    fingerprint: u64,
    state: State,
}

enum State {
    Building,
    Ready {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    Failed,
}

impl ProxyStore {
    /// Starts eligible encodes in the background. Only `live` substitutes
    /// ready proxies; a paused full-quality frame stays source-exact.
    pub(crate) fn clips(
        &self,
        clips: Arc<Vec<ExportClip>>,
        live: bool,
        time: f64,
    ) -> Arc<Vec<ExportClip>> {
        let mut changed: Option<Vec<ExportClip>> = None;
        for (index, clip) in clips.iter().enumerate() {
            // Prepare the visible clip and a few seconds ahead, not every
            // large file in a long project at once.
            if !should_proxy(clip)
                || clip.start > time + 3.0
                || clip.start + clip.duration < time - 0.5
            {
                continue;
            }
            let landscape = clip.media_width.unwrap_or(1) >= clip.media_height.unwrap_or(1);
            let Some((path, width, height)) = self.ready_or_start(Path::new(&clip.path), landscape)
            else {
                continue;
            };
            if live {
                let resolved = changed.get_or_insert_with(|| (*clips).clone());
                resolved[index].path = path.to_string_lossy().into_owned();
                resolved[index].media_width = Some(width);
                resolved[index].media_height = Some(height);
            }
        }
        changed.map(Arc::new).unwrap_or(clips)
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    fn ready_or_start(&self, source: &Path, landscape: bool) -> Option<(PathBuf, u32, u32)> {
        let metadata = std::fs::metadata(source).ok()?;
        let fingerprint = fingerprint(source, &metadata);
        let output = std::env::temp_dir()
            .join("concat-preview-proxies")
            .join(format!("{fingerprint:016x}.mp4"));
        let mut entries = self.entries.lock().ok()?;
        if let Some(entry) = entries.get(source)
            && entry.fingerprint == fingerprint
        {
            match &entry.state {
                State::Ready {
                    path,
                    width,
                    height,
                } if path.is_file() => return Some((path.clone(), *width, *height)),
                State::Ready { .. } => {
                    entries.remove(source);
                }
                State::Building | State::Failed => return None,
            }
        }
        if let Some((width, height)) = probe_proxy(&output) {
            entries.insert(
                source.to_path_buf(),
                Entry {
                    fingerprint,
                    state: State::Ready {
                        path: output.clone(),
                        width,
                        height,
                    },
                },
            );
            return Some((output, width, height));
        }
        if entries
            .values()
            .filter(|entry| matches!(&entry.state, State::Building))
            .count()
            >= MAX_BUILDING
        {
            return None;
        }
        entries.insert(
            source.to_path_buf(),
            Entry {
                fingerprint,
                state: State::Building,
            },
        );
        drop(entries);

        let source = source.to_path_buf();
        let entries = Arc::clone(&self.entries);
        std::thread::spawn(move || {
            let result = build_proxy(&source, &output, landscape);
            if let Ok(mut entries) = entries.lock()
                && let Some(entry) = entries.get_mut(&source)
                && entry.fingerprint == fingerprint
            {
                entry.state = match result {
                    Some((width, height)) => State::Ready {
                        path: output,
                        width,
                        height,
                    },
                    None => State::Failed,
                };
            }
        });
        None
    }
}

fn should_proxy(clip: &ExportClip) -> bool {
    if clip.kind != ClipKind::Video || clip.path.is_empty() {
        return false;
    }
    let (Some(width), Some(height)) = (clip.media_width, clip.media_height) else {
        return false;
    };
    large_source(width, height)
}

fn large_source(width: u32, height: u32) -> bool {
    width.max(height) > 1920 || u64::from(width) * u64::from(height) > 2_000_000
}

fn fingerprint(source: &Path, metadata: &std::fs::Metadata) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    PROXY_VERSION.hash(&mut hasher);
    PROXY_EDGE.hash(&mut hasher);
    PROXY_FPS.hash(&mut hasher);
    source.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .hash(&mut hasher);
    hasher.finish()
}

fn probe_proxy(path: &Path) -> Option<(u32, u32)> {
    let video = concat_media::probe(path).ok()?.video?;
    (video.width.max(video.height) <= PROXY_EDGE + 2
        && video.frame_rate.fps().as_f64() == f64::from(PROXY_FPS))
    .then_some((video.width, video.height))
}

fn build_proxy(source: &Path, output: &Path, landscape: bool) -> Option<(u32, u32)> {
    std::fs::create_dir_all(output.parent()?).ok()?;
    let job_id = PROXY_JOB_ID.fetch_add(1, Ordering::Relaxed);
    let pending = output.with_extension(format!("{}-{job_id}.pending.mp4", std::process::id()));
    let scale = if landscape {
        format!("fps={PROXY_FPS},scale={PROXY_EDGE}:-2")
    } else {
        format!("fps={PROXY_FPS},scale=-2:{PROXY_EDGE}")
    };
    let status = Command::new(ffmpeg_binary())
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(source)
        .args(["-map", "0:v:0", "-vf"])
        .arg(scale)
        .args([
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(&pending)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        let _ = std::fs::remove_file(&pending);
        return None;
    }
    let Some(dimensions) = probe_proxy(&pending) else {
        let _ = std::fs::remove_file(&pending);
        return None;
    };
    if std::fs::rename(&pending, output).is_err() {
        let _ = std::fs::remove_file(&pending);
        return probe_proxy(output);
    }
    Some(dimensions)
}

fn ffmpeg_binary() -> PathBuf {
    // Launch Services does not reliably pass a Terminal's PATH to a macOS
    // app. Check the usual Homebrew locations before falling back to PATH.
    for path in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        let binary = Path::new(path);
        if binary.is_file() {
            return binary.to_path_buf();
        }
    }
    PathBuf::from("ffmpeg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_large_video_sources_need_preview_proxies() {
        assert!(large_source(3456, 2234));
        assert!(large_source(2048, 1080));
        assert!(!large_source(1280, 720));
    }

    #[test]
    #[ignore]
    fn a_real_source_builds_a_small_local_proxy_in_the_background() {
        let source = PathBuf::from(std::env::var_os("CONCAT_DIAG_MEDIA").unwrap());
        let video = concat_media::probe(&source).unwrap().video.unwrap();
        let store = ProxyStore::default();
        let start = std::time::Instant::now();
        let (proxy, width, height) = loop {
            if let Some(ready) = store.ready_or_start(&source, video.width >= video.height) {
                break ready;
            }
            assert!(start.elapsed().as_secs() < 60, "proxy did not become ready");
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        assert!(width.max(height) <= PROXY_EDGE + 2);
        assert_ne!(proxy, source);
        assert!(proxy.exists());

        let clip: ExportClip = serde_json::from_value(serde_json::json!({
            "path": source.to_string_lossy(),
            "kind": "video",
            "start": 0.0,
            "duration": 1.0,
            "sourceStart": 0.0,
            "track": 0,
            "hidden": false,
            "muted": true,
            "mediaWidth": video.width,
            "mediaHeight": video.height
        }))
        .unwrap();
        let original = Arc::new(vec![clip]);
        let paused = store.clips(Arc::clone(&original), false, 0.0);
        assert_eq!(paused[0].path, original[0].path);
        let live = store.clips(original, true, 0.0);
        assert_eq!(live[0].path, proxy.to_string_lossy());
        assert_eq!(live[0].media_width, Some(width));
        assert_eq!(live[0].media_height, Some(height));
    }
}
