// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A transcription in a process of its own: the [`concat_worker::Job`] the
//! window hands its captions to on a desktop.
//!
//! whisper.cpp is native code, and when it fails it does not return an
//! error: a transcription that overran its stack closed the editor with
//! STATUS_STACK_BUFFER_OVERRUN (#673), and others closed it outright (#736,
//! #715), taking unsaved work along. In a worker the same failure costs the
//! captions one run and the editor nothing: the sheet says the
//! transcription stopped, and the person can try a smaller model.
//!
//! A worker that dies is not run again: whisper fails the same way on the
//! same sound, and a second crash is only a second wait. The model is
//! loaded per run, the price of the worker's separate memory; a phone, which
//! starts no worker, keeps it loaded between runs as before.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use concat_core::Halt;
use concat_host::AppDirs;
use concat_worker::{Context, Event, Policy};
use serde::{Deserialize, Serialize};

use crate::JobStage;
use crate::transcribe::{Segment, TranscribeRequest, Transcriber};

/// The transcription job, as the worker table names it.
pub struct Transcribe;

/// What a worker needs to hear a clip: where the models are, and what to
/// hear.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    /// [`AppDirs::config`].
    pub config: PathBuf,
    /// [`AppDirs::data`], where the models are.
    pub data: PathBuf,
    /// The clip's sound, and the model and language to hear it with.
    pub request: TranscribeRequest,
}

/// Raised by [`cancel`], read by the run in flight.
static CANCEL: AtomicBool = AtomicBool::new(false);

impl concat_worker::Job for Transcribe {
    const NAME: &'static str = "transcribe";
    type Request = Order;
    type Progress = JobStage;
    type Output = Vec<Segment>;

    fn run(order: Order, context: &Context<JobStage>) -> Result<Vec<Segment>, Halt> {
        let dirs = AppDirs {
            config: order.config,
            data: order.data,
        };
        let transcriber = Arc::new(Transcriber::new());
        // The window's cancel reaches the worker as its stop flag; whisper
        // looks at its own, so the one is handed on to the other.
        let watch = {
            let transcriber = Arc::clone(&transcriber);
            let flag = context.cancel_handle();
            std::thread::Builder::new()
                .name("concat transcribe cancel".into())
                .spawn(move || {
                    while Arc::strong_count(&transcriber) > 1 {
                        if flag.load(Ordering::Relaxed) {
                            transcriber.cancel();
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                })
                .ok()
        };
        let report = context.reporter();
        let heard = transcriber.transcribe(&dirs, &order.request, move |stage| report(&stage));
        drop(transcriber);
        if let Some(watch) = watch {
            let _ = watch.join();
        }
        heard
    }

    /// whisper reports once per thirty seconds of sound, and on a slow CPU
    /// with a large model thirty seconds takes minutes.
    fn quiet(_last: Option<&JobStage>) -> Duration {
        Duration::from_secs(15 * 60)
    }
}

/// The window's side: hears `request` in a worker of `program`, reporting
/// each stage, and hands back what was heard. `None` when no worker could
/// be started, and the caller transcribes in its own process.
pub fn transcribe(
    program: &Path,
    dirs: &AppDirs,
    request: &TranscribeRequest,
    mut progress: impl FnMut(JobStage),
) -> Option<Result<Vec<Segment>, Halt>> {
    CANCEL.store(false, Ordering::Relaxed);
    let order = Order {
        config: dirs.config.clone(),
        data: dirs.data.clone(),
        request: request.clone(),
    };
    let heard =
        concat_worker::run::<Transcribe>(program, &order, &Policy::once(), &CANCEL, |event| {
            if let Event::Progress(stage) = event {
                progress(stage);
            }
        })?;
    Some(heard.map_err(|halt| match halt {
        Halt::Failed(why) if why.contains("stopped each time") => Halt::Failed(format!(
            "The transcription stopped unexpectedly. A smaller model may get through: {why}"
        )),
        other => other,
    }))
}

/// Stops the transcription running in a worker, if one is.
pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}
