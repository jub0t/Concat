// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Selection consolidation: render first, then replace in one undo step.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use concat_host::consolidate::Consolidation;
use concat_project::consolidation_clips;

use super::*;

pub(super) struct MergeJob {
    cancel: Arc<AtomicBool>,
    progress: f32,
}

/// Removes an unadopted render even when its project was closed before
/// the worker's callback arrived. Successful edits keep the generated media
/// through undo/redo and across cache clearing.
struct RenderedFile {
    folder: PathBuf,
    keep: bool,
}

impl Drop for RenderedFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(self.folder.join("merged.mp4"));
            let _ = std::fs::remove_dir(&self.folder);
        }
    }
}

impl Studio {
    pub fn merge_blocked(&self) -> Option<String> {
        if self.merge_job.is_some() {
            return None; // The same action cancels a running merge.
        }
        if self.host.exporter.is_busy() {
            return Some(t("studio.mergeExportBusy"));
        }
        if self.selection.iter().any(|id| {
            self.clip(id)
                .is_some_and(|clip| self.locked(&clip.track_id))
        }) {
            return Some(t("studio.mergeTrackLocked"));
        }
        consolidation_clips(self.timeline(), &self.selection)
            .err()
            .map(|error| error.to_string())
    }

    pub(super) fn merge_label(&self) -> String {
        match &self.merge_job {
            Some(job) => tf(
                "studio.mergeCancelProgress",
                &[&format!("{:.0}", job.progress * 100.0)],
            ),
            None => t("studio.mergeSelected"),
        }
    }

    pub(super) fn cancel_merge(&mut self) {
        if let Some(job) = self.merge_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn merge(&mut self) {
        if let Some(job) = &self.merge_job {
            job.cancel.store(true, Ordering::Relaxed);
            return;
        }
        self.flush_commit();
        if let Some(reason) = self.merge_blocked() {
            self.notify(&reason, true);
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let plan = match Consolidation::new(session.project(), &self.selection) {
            Ok(plan) => plan,
            Err(error) => {
                self.notify(&error, true);
                return;
            }
        };
        let job = match self.host.exporter.begin() {
            Ok(job) => job,
            Err(error) => {
                self.notify(&error, true);
                return;
            }
        };
        let root = PathBuf::from(session.path());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let folder = root.join("media").join(format!("merged-{stamp}"));
        if let Err(error) =
            std::fs::create_dir_all(root.join("media")).and_then(|()| std::fs::create_dir(&folder))
        {
            self.notify(&tf("studio.mergeFailed", &[&error.to_string()]), true);
            return;
        }
        let rendered = RenderedFile {
            folder: folder.clone(),
            keep: false,
        };
        let output = folder.join("merged.mp4");
        let request = plan.request(session.project(), &root, &output);
        let compositor = self.host.monitor.off_thread();
        self.merge_job = Some(MergeJob {
            cancel: job.cancel_handle(),
            progress: 0.0,
        });
        self.pause();
        self.notify(&t("studio.mergeStarted"), false);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = -1.0;
                let result = concat_host::export::run_on(
                    &request,
                    compositor,
                    job.cancel_flag(),
                    |progress| {
                        let fraction =
                            (progress.frame as f32 / progress.total.max(1) as f32).clamp(0.0, 1.0);
                        if fraction - last >= 0.02 {
                            last = fraction;
                            on_ui_in_project(epoch, move |studio, _, _| {
                                if let Some(job) = studio.merge_job.as_mut() {
                                    job.progress = fraction;
                                }
                            });
                        }
                    },
                );
                let cancel = job.cancel_handle();
                (plan, rendered, output, cancel, result)
            },
            |studio, _, _, (plan, mut rendered, output, cancel, result)| {
                studio.merge_job = None;
                if cancel.load(Ordering::Relaxed) {
                    studio.notify(&t("studio.mergeCancelled"), false);
                    return;
                }
                let outcome = result.and_then(|_| {
                    let summary = media::probe(&output.to_string_lossy())?;
                    let mut item = summary.to_new_media();
                    item.name = t("studio.mergedClips");
                    item.origin = Some(model::MediaOrigin::Processed);
                    studio.flush_commit();
                    let command = plan.replacement(studio.project(), item)?;
                    // Locking a lane while rendering must also prevent adoption.
                    if studio.locked(plan.track_id()) {
                        return Err(t("studio.mergeTrackLocked"));
                    }
                    studio
                        .apply(command)
                        .ok_or_else(|| t("studio.mergeCouldNotReplace"))
                });
                match outcome {
                    Ok(id) => {
                        rendered.keep = true;
                        studio.selection = vec![id];
                        studio.transition_selected = None;
                        studio.request_media_art();
                        studio.request_preview();
                        studio.notify(&t("studio.mergeFinished"), false);
                    }
                    Err(error) => studio.notify(&tf("studio.mergeFailed", &[&error]), true),
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unadopted_render_is_removed_but_adopted_media_survives() {
        let folder =
            std::env::temp_dir().join(format!("concat-merge-cleanup-{}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let output = folder.join("merged.mp4");
        std::fs::write(&output, b"rendered source").unwrap();
        drop(RenderedFile {
            folder: folder.clone(),
            keep: false,
        });
        assert!(
            !folder.exists(),
            "cancel, failure and stale delivery clean up the source"
        );
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&output, b"adopted source").unwrap();
        drop(RenderedFile {
            folder: folder.clone(),
            keep: true,
        });
        assert!(output.is_file(), "undo/redo still needs adopted media");
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
