// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Enhance: an enhanced copy of a media file, written once.
//!
//! Restoration is not real time - a frame takes the model a good part of
//! a second on an accelerator and many seconds without one - so a clip is
//! not enhanced as it plays. Instead the job here reads the file frame by
//! frame, runs each through [`concat_vision::enhance`], and writes the
//! result into the project's cache as a file of its own: a PNG for a
//! still, an MP4 with the original's sound for footage. The window then
//! points the clip at the copy, the way a freeze frame points at its
//! still, and the monitor and the export read it like any other file.
//!
//! One job at a time through a [`SingleFlight`], like every long job the
//! host runs, with the model fetched on first use and kept loaded. The
//! copy is named after the file's path, size and modification time and
//! the factor it was enlarged by, so a file replaced on disk gets a fresh
//! one and the same file asked for twice gets the one already there.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use concat_media::{
    DecodeOptions, Decoder, EncodeOptions, Encoder, FrameSink, FrameSource, RateMode, VideoCodec,
};
use concat_vision::enhance::Enhancer;
use concat_vision::{ModelId, models};

pub use crate::cutout::Progress;
use crate::jobs::SingleFlight;

/// What one enhancement covers.
#[derive(Clone, Debug)]
pub struct EnhanceRequest {
    /// The media file to read.
    pub media_path: String,
    /// A still: one frame in, one PNG out.
    pub still: bool,
    /// How many times larger the copy is; see
    /// [`concat_vision::enhance::factor_for`].
    pub factor: u32,
    /// Where the copy goes; see [`target_for`].
    pub target: PathBuf,
}

/// The enhancement service: the model, loaded once, and the one-job slot.
pub struct Enhancers {
    gate: Arc<SingleFlight>,
    /// The app's data directory, where the downloaded model lives.
    data: PathBuf,
    model: Mutex<Option<Arc<Enhancer>>>,
}

/// Where the enhanced copy of `media` at `factor` lives under `project`:
/// in the cache, named by the file's path, size and modification time.
/// None for a file that cannot be stat'ed, which cannot be read either.
pub fn target_for(project: &Path, media: &str, factor: u32, still: bool) -> Option<PathBuf> {
    let meta = std::fs::metadata(media).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    media.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Ok(modified) = meta.modified()
        && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        since.as_secs().hash(&mut hasher);
    }
    let extension = if still { "png" } else { "mp4" };
    Some(project.join("cache").join(format!(
        "enhance-{:016x}-{factor}x.{extension}",
        hasher.finish()
    )))
}

impl Enhancers {
    /// A service with nothing loaded yet, keeping its model under `data`.
    pub fn new(data: &Path) -> Enhancers {
        Enhancers {
            gate: Arc::new(SingleFlight::new()),
            data: data.to_path_buf(),
            model: Mutex::new(None),
        }
    }

    /// Whether an enhancement is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running enhancement to stop after the frame in hand.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Whether the model is on disk.
    pub fn installed(&self) -> bool {
        models::installed(&self.data, ModelId::Enhance)
    }

    /// The model, fetched first when it is not on disk.
    fn loaded(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Arc<Enhancer>, String> {
        if let Some(loaded) = self
            .model
            .lock()
            .map_err(|_| "model slot poisoned")?
            .as_ref()
        {
            return Ok(Arc::clone(loaded));
        }
        let file = models::model_file(&self.data, ModelId::Enhance);
        if !self.installed() {
            crate::cutout::fetch(ModelId::Enhance, &file, cancel, progress)?;
        }
        let enhancer = Arc::new(Enhancer::load(&file)?);
        *self.model.lock().map_err(|_| "model slot poisoned")? = Some(Arc::clone(&enhancer));
        Ok(enhancer)
    }

    /// Writes the copy `request` asks for, or finds it already written.
    /// Blocks until done, so run it on its own thread; `progress` is
    /// called as it goes. Returns where the copy is.
    pub fn enhance(
        &self,
        request: &EnhanceRequest,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, String> {
        if request.target.is_file() {
            return Ok(request.target.clone());
        }
        let job = self.gate.begin("enhance")?;
        let cancel = job.cancel_handle();
        let enhancer = self.loaded(&cancel, progress)?;
        if let Some(folder) = request.target.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        progress(Progress::Analysing(0.0));
        let result = if request.still {
            still(&enhancer, request)
        } else {
            footage(&enhancer, request, &job, progress)
        };
        result.map(|()| request.target.clone())
    }
}

/// One frame through the model, as a PNG: lossless, since the point was
/// the detail.
fn still(enhancer: &Enhancer, request: &EnhanceRequest) -> Result<(), String> {
    let mut decoder = Decoder::open(&request.media_path, &DecodeOptions::default().in_software())
        .map_err(|error| error.to_string())?;
    let frame = decoder
        .next_frame()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{} holds no picture", request.media_path))?;
    let enhanced = enhancer.enhance(&frame, request.factor)?;
    let partial = request.target.with_extension("part.png");
    let written = (|| {
        let file = std::fs::File::create(&partial).map_err(|error| error.to_string())?;
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            enhanced.width(),
            enhanced.height(),
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(enhanced.pixels())
            .map_err(|error| error.to_string())
    })();
    match written {
        Ok(()) => std::fs::rename(&partial, &request.target)
            .map_err(|error| format!("{}: {error}", request.target.display())),
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

/// Every frame through the model, encoded at the enlarged size, then the
/// original's sound copied in beside the picture. Written under partial
/// names and moved into place only when whole, so a copy that is there is
/// a copy that is complete.
fn footage(
    enhancer: &Enhancer,
    request: &EnhanceRequest,
    job: &crate::jobs::Job,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    let info = concat_media::probe(&request.media_path).map_err(|error| error.to_string())?;
    let video = info
        .video
        .as_ref()
        .ok_or_else(|| "no video stream".to_owned())?;
    let rate = video.frame_rate;
    // About how many frames there are, for the readout; the count is not
    // known until the last one is read.
    let expected = info
        .duration
        .map(|duration| duration.as_f64() * rate.fps().as_f64())
        .unwrap_or(0.0)
        .max(1.0);
    // Even sides, which the encoder wants: a stray pixel at the right or
    // bottom is cropped rather than the whole copy refused.
    let width = (video.width * request.factor) & !1;
    let height = (video.height * request.factor) & !1;
    if width == 0 || height == 0 {
        return Err("the picture is too small to enhance".to_owned());
    }

    let partial_video = request.target.with_extension("part.mp4");
    let partial_muxed = request.target.with_extension("muxed.part.mp4");
    let options = DecodeOptions::default().in_software().threaded(2);
    let mut decoder =
        Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())?;
    let mut encoder = Encoder::create(
        &partial_video,
        width,
        height,
        rate,
        &EncodeOptions {
            codec: VideoCodec::H264,
            // A copy the timeline reads for as long as the project lives:
            // near-transparent quality, and the time it takes is spent in
            // the model anyway.
            preset: "medium".to_owned(),
            crf: 16,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: false,
            color_range: concat_media::ColorRange::Limited,
            hardware: false,
            threads: 2,
        },
    )
    .map_err(|error| error.to_string())?;
    let encoded = (|| {
        let mut count = 0u64;
        while let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? {
            if job.cancelled() {
                return Err("enhance cancelled".to_owned());
            }
            let enhanced = enhancer.enhance(&frame, request.factor)?;
            let enhanced = crop(&enhanced, width, height);
            encoder
                .write_frame(&enhanced)
                .map_err(|error| error.to_string())?;
            count += 1;
            progress(Progress::Analysing(
                (count as f64 / expected).min(0.99) as f32
            ));
        }
        encoder.finish().map_err(|error| error.to_string())
    })();
    if let Err(error) = encoded {
        let _ = std::fs::remove_file(&partial_video);
        return Err(error);
    }
    let finished = if info.audio.is_some() {
        let muxed = concat_media::audio::mux(
            &partial_video,
            Path::new(&request.media_path),
            &partial_muxed,
        )
        .map_err(|error| error.to_string());
        let _ = std::fs::remove_file(&partial_video);
        muxed.and_then(|()| {
            std::fs::rename(&partial_muxed, &request.target)
                .map_err(|error| format!("{}: {error}", request.target.display()))
        })
    } else {
        std::fs::rename(&partial_video, &request.target)
            .map_err(|error| format!("{}: {error}", request.target.display()))
    };
    if finished.is_err() {
        let _ = std::fs::remove_file(&partial_muxed);
    }
    finished
}

/// `frame` cut to `width` by `height` from its top-left, or the frame
/// itself when it is that size already.
pub(crate) fn crop(frame: &concat_core::Frame, width: u32, height: u32) -> concat_core::Frame {
    if frame.width() == width && frame.height() == height {
        return frame.clone();
    }
    let mut out = concat_core::Frame::black(width, height);
    let bytes = width as usize * concat_core::frame::BYTES_PER_PIXEL;
    let stride = frame.width() as usize * concat_core::frame::BYTES_PER_PIXEL;
    let source = frame.pixels();
    let pixels = out.pixels_mut();
    for y in 0..height as usize {
        pixels[y * bytes..(y + 1) * bytes].copy_from_slice(&source[y * stride..y * stride + bytes]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_named_after_the_file_and_the_factor() {
        let dir = std::env::temp_dir().join(format!("concat-enhance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"not really").expect("write");
        let media = media.to_string_lossy().into_owned();
        let project = dir.join("project");
        let two = target_for(&project, &media, 2, false).expect("named");
        assert!(two.starts_with(project.join("cache")));
        assert!(two.to_string_lossy().ends_with("-2x.mp4"), "{two:?}");
        assert_eq!(target_for(&project, &media, 2, false), Some(two.clone()));
        assert_ne!(target_for(&project, &media, 1, false), Some(two.clone()));
        let still = target_for(&project, &media, 2, true).expect("named");
        assert!(still.to_string_lossy().ends_with("-2x.png"), "{still:?}");
        assert_eq!(
            target_for(&project, &dir.join("gone.mp4").to_string_lossy(), 2, false),
            None
        );
        // A file with different bytes is a different copy.
        std::fs::write(&media, b"not really, but longer").expect("write");
        assert_ne!(target_for(&project, &media, 2, false), Some(two));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crop_takes_the_top_left_and_leaves_a_fitting_frame_alone() {
        let mut frame = concat_core::Frame::black(4, 3);
        frame.set_pixel(3, 2, [1, 2, 3, 255]);
        frame.set_pixel(0, 0, [9, 9, 9, 255]);
        let same = crop(&frame, 4, 3);
        assert_eq!(same.pixels(), frame.pixels());
        let cut = crop(&frame, 2, 2);
        assert_eq!((cut.width(), cut.height()), (2, 2));
        assert_eq!(cut.pixel(0, 0), Some([9, 9, 9, 255]));
        assert_eq!(cut.pixel(1, 1), Some([0, 0, 0, 255]));
    }
}
