// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Proxies: a smaller copy of a large file, written once into the
//! project's cache, that playback and the filmstrips read instead of the
//! original.
//!
//! A 4K file decodes at a few frames a second on a laptop without
//! hardware decode; its quarter-size copy decodes at dozens. So a file
//! larger than HD gets one, written on the scheduler's proxy lane - one
//! at a time, on two threads, so the machine stays usable meanwhile - and
//! adopted into the reader pool as the file's stand-in. A request that
//! asks for the proxy reads it; the paused monitor and the export never
//! do, and see the original. The copy is named after the file's path,
//! size and modification time, so a file replaced on disk gets a fresh
//! one and the old copy is swept with the rest of the cache.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use concat_media::decode::{DecodeOptions, Decoder, FrameSource};
use concat_media::{ColorRange, EncodeOptions, Encoder, FrameSink, Priority, RateMode, VideoCodec};

/// A file with more pixels than this gets a proxy: anything larger than
/// full HD.
const PIXELS_WITHOUT_PROXY: u64 = 1920 * 1080;
/// The proxy's long side is the original's divided by this, and never
/// shorter than [`SHORTEST_LONG_SIDE`].
const DIVISOR: u32 = 4;
const SHORTEST_LONG_SIDE: u32 = 960;

/// Whether a `width` by `height` file is worth a proxy.
pub fn wanted(width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) > PIXELS_WITHOUT_PROXY
}

/// The proxy's size for a `width` by `height` original: the long side a
/// quarter of the original's, not under [`SHORTEST_LONG_SIDE`] and never
/// over the original, the aspect kept, both sides even.
pub fn size_for(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (width.max(2), height.max(2));
    let long = width.max(height);
    let target = (long / DIVISOR).max(SHORTEST_LONG_SIDE).min(long);
    let scale = f64::from(target) / f64::from(long);
    let side = |px: u32| (((f64::from(px) * scale).round() as u32) & !1).max(2);
    (side(width), side(height))
}

/// Where the proxy of `media` lives under `project`: named by the file's
/// path, size and modification time, and by the range it is read as,
/// since the copy is written from the corrected picture and a correction
/// changed is a different copy. None for a file that cannot be stat'ed,
/// which cannot be read either.
pub fn path_for(project: &Path, media: &str, range: Option<ColorRange>) -> Option<PathBuf> {
    let meta = std::fs::metadata(media).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    media.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Some(range) = range {
        range.name().hash(&mut hasher);
    }
    if let Ok(modified) = meta.modified()
        && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        since.as_secs().hash(&mut hasher);
    }
    Some(
        project
            .join("cache")
            .join("proxy")
            .join(format!("{:016x}.mp4", hasher.finish())),
    )
}

/// The proxy of `media` under `project`, read as `range`, when it has
/// been written.
pub fn existing(project: &Path, media: &str, range: Option<ColorRange>) -> Option<PathBuf> {
    path_for(project, media, range).filter(|path| path.is_file())
}

/// Sees to it that `media` - `width` by `height`, read as `range` - has
/// a proxy when it is worth one: an existing copy is adopted into the
/// reader pool at once; otherwise one is queued to be written on the
/// proxy lane and adopted when done. Asking again while it is being
/// written is free. Returns whether the file has, or will have, a proxy.
pub fn ensure(
    project: &Path,
    media: &str,
    width: u32,
    height: u32,
    range: Option<ColorRange>,
) -> bool {
    if !wanted(width, height) {
        return false;
    }
    let Some(target) = path_for(project, media, range) else {
        return false;
    };
    let pool = crate::scheduler().pool();
    if target.is_file() {
        if pool.proxy_of(Path::new(media)).as_deref() != Some(target.as_path()) {
            pool.adopt_proxy(Path::new(media), target);
        }
        return true;
    }
    let writing = writing();
    {
        let mut writing = writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !writing.insert(target.clone()) {
            return true;
        }
    }
    let source = media.to_owned();
    let size = size_for(width, height);
    crate::scheduler().submit(Priority::Proxy, move || {
        match write(&source, &target, size, range) {
            Ok(()) => {
                crate::scheduler()
                    .pool()
                    .adopt_proxy(Path::new(&source), target.clone());
                log::info!("{source}: proxy written to {}", target.display());
            }
            Err(error) => log::warn!("{source}: no proxy: {error}"),
        }
        writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&target);
    });
    true
}

/// The proxies being written now, by target, so one is never queued
/// twice and a sweep leaves it for its writer to finish.
fn writing() -> &'static Mutex<HashSet<PathBuf>> {
    static WRITING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    WRITING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Removes the proxies under `project` that `keep` does not name: the
/// copies of media no longer in the project, or read as a range they no
/// longer are. Nothing else sweeps this folder, and a proxy is a
/// quarter-size copy of every file over HD the project ever held
/// (audit 2026-09-23, #15). One being written is left for its writer.
/// Returns how many files went.
pub fn sweep(project: &Path, keep: &HashSet<PathBuf>) -> usize {
    let dir = project.join("cache").join("proxy");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let writing = writing()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "mp4")
            && !keep.contains(&path)
            && !writing.contains(&path)
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Writes the proxy of `source`, read as `range`, at `size` to `target`:
/// H.264, a fast preset, two threads, in software, into a temporary name
/// that is moved into place only when whole, so a copy that is there is a
/// copy that is complete. The copy is tagged video range and converted to
/// match, so it is read as it is and needs no correction of its own.
pub fn write(
    source: &str,
    target: &Path,
    (width, height): (u32, u32),
    range: Option<ColorRange>,
) -> Result<(), String> {
    let info = concat_media::probe(source).map_err(|error| error.to_string())?;
    let rate = info
        .video
        .as_ref()
        .map(|video| video.frame_rate)
        .ok_or_else(|| "no video stream".to_owned())?;
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
    }
    // Written beside the target under a name the muxer still knows the
    // format of, and moved into place only when whole.
    let partial = target.with_extension("part.mp4");
    let options = DecodeOptions::default()
        .scaled_to(width, height)
        .in_software()
        .threaded(2)
        .in_range(range);
    let mut decoder = Decoder::open(source, &options).map_err(|error| error.to_string())?;
    let mut encoder = Encoder::create(
        &partial,
        width,
        height,
        rate,
        &EncodeOptions {
            codec: VideoCodec::H264,
            preset: "veryfast".to_owned(),
            crf: 23,
            // A proxy is a working copy: the CRF says how good, and no
            // bitrate target ever applies to it.
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            color_range: ColorRange::Limited,
            hardware: false,
            threads: 2,
        },
    )
    .map_err(|error| error.to_string())?;
    let result = (|| {
        while let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? {
            encoder
                .write_frame(&frame)
                .map_err(|error| error.to_string())?;
        }
        encoder.finish().map_err(|error| error.to_string())
    })();
    match result {
        Ok(()) => std::fs::rename(&partial, target)
            .map_err(|error| format!("{}: {error}", target.display())),
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_larger_than_hd_get_a_proxy() {
        assert!(!wanted(1920, 1080));
        assert!(!wanted(1280, 720));
        assert!(wanted(3840, 2160));
        assert!(wanted(2560, 1440));
        assert!(!wanted(0, 0));
    }

    #[test]
    fn a_proxy_is_a_quarter_of_the_long_side_but_never_tiny_or_larger() {
        assert_eq!(size_for(3840, 2160), (960, 540));
        assert_eq!(size_for(7680, 4320), (1920, 1080));
        // A tall file keeps its way up.
        assert_eq!(size_for(2160, 3840), (540, 960));
        // A file too small for a quarter gets the floor, never upscaled.
        assert_eq!(size_for(2560, 1440), (960, 540));
        assert_eq!(size_for(640, 480), (640, 480));
        // Odd sizes come out even, and nothing is ever under two.
        let (w, h) = size_for(3841, 2161);
        assert!(w % 2 == 0 && h % 2 == 0);
        assert_eq!(size_for(1, 1), (2, 2));
    }

    /// A proxy written from a file larger than HD is a quarter of it, is
    /// adopted by the pool, and serves a moving request while the paused
    /// one still reads the original.
    #[test]
    fn a_written_proxy_is_read_for_a_moving_picture_only() {
        use concat_core::frame::Frame;
        use concat_core::time::FrameRate;
        let dir = std::env::temp_dir().join(format!("concat-proxy-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let media = dir.join("big.mp4");
        {
            let mut encoder = Encoder::create(
                &media,
                2560,
                1440,
                FrameRate::THIRTY,
                &EncodeOptions {
                    preset: "ultrafast".to_owned(),
                    hardware: false,
                    ..EncodeOptions::default()
                },
            )
            .expect("encodes");
            for index in 0..6u8 {
                let mut frame = Frame::black(2560, 1440);
                frame.fill([index * 40, 20, 200, 255]);
                encoder.write_frame(&frame).expect("writes");
            }
            encoder.finish().expect("finishes");
        }
        let project = dir.join("project");
        let source = media.to_string_lossy().into_owned();
        assert!(ensure(&project, &source, 2560, 1440, None), "worth a proxy");
        crate::scheduler().drain();
        let proxy = existing(&project, &source, None).expect("the proxy was written");
        let info = concat_media::probe(&proxy).expect("probes");
        let video = info.video.expect("has pictures");
        assert_eq!((video.width, video.height), (960, 540));
        let pool = crate::scheduler().pool();
        assert_eq!(
            pool.proxy_of(Path::new(&source)).as_deref(),
            Some(proxy.as_path())
        );
        // Second time round: nothing to write, still adopted.
        assert!(ensure(&project, &source, 2560, 1440, None));
        let time = FrameRate::THIRTY.time_of_frame(2);
        let moving = pool
            .frame(&concat_media::FrameRequest::new(&source, time, 480, 270).from_proxy(true))
            .expect("reads the proxy");
        assert_eq!((moving.width(), moving.height()), (480, 270));
        let paused = pool
            .frame(&concat_media::FrameRequest::new(&source, time, 480, 270))
            .expect("reads the original");
        assert_eq!((paused.width(), paused.height()), (480, 270));
        // A file not worth a proxy is left alone.
        assert!(!ensure(&project, &source, 1280, 720, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_proxy_path_follows_the_file_and_changes_when_the_file_does() {
        let dir = std::env::temp_dir().join(format!("concat-proxy-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"one").expect("writes");
        let project = dir.join("project");
        let first = path_for(&project, &media.to_string_lossy(), None).expect("a path");
        assert!(first.starts_with(project.join("cache").join("proxy")));
        assert!(existing(&project, &media.to_string_lossy(), None).is_none());
        // Read as another range, the file is another proxy too: the copy
        // holds the corrected picture.
        let full =
            path_for(&project, &media.to_string_lossy(), Some(ColorRange::Full)).expect("a path");
        assert_ne!(first, full, "a file read as full range is another proxy");
        std::fs::write(&media, b"one more byte").expect("writes");
        let second = path_for(&project, &media.to_string_lossy(), None).expect("a path");
        assert_ne!(first, second, "a changed file is another proxy");
        assert!(path_for(&project, &dir.join("missing.mp4").to_string_lossy(), None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sweep_keeps_what_is_named_and_what_is_being_written() {
        let scratch =
            std::env::temp_dir().join(format!("concat-proxy-sweep-{}", std::process::id()));
        let dir = scratch.join("cache").join("proxy");
        std::fs::create_dir_all(&dir).expect("a proxy folder");
        let kept = dir.join("1111111111111111.mp4");
        let stale = dir.join("2222222222222222.mp4");
        let busy = dir.join("3333333333333333.mp4");
        let partial = dir.join("4444444444444444.mp4.part");
        for file in [&kept, &stale, &busy, &partial] {
            std::fs::write(file, b"x").expect("writes");
        }
        writing()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(busy.clone());
        let keep: HashSet<PathBuf> = [kept.clone()].into_iter().collect();
        assert_eq!(sweep(&scratch, &keep), 1);
        assert!(kept.is_file() && busy.is_file() && partial.is_file());
        assert!(!stale.is_file());
        writing()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&busy);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
