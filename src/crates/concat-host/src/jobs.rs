// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! One-at-a-time job slots for long-running work.
//!
//! Export, transcription and model downloads are all "one at a time", and
//! a doc comment cannot enforce that: two exports would race each other's
//! temp files, and a cancel flag shared between runs would let a second
//! start clear it out from under the first. [`SingleFlight`] makes the
//! invariant code: `begin` refuses a second
//! concurrent job, and every run gets its own cancel flag, so a cancel can
//! only ever stop the job that is actually running and a new job can never be
//! un-cancelled by a stale start or stopped by a stale cancel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The slot: at most one running job, identified by its own cancel flag.
pub struct SingleFlight {
    slot: Mutex<Option<Arc<AtomicBool>>>,
}

impl Default for SingleFlight {
    fn default() -> Self {
        Self::new()
    }
}

impl SingleFlight {
    /// An empty slot.
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    /// Claims the slot for a new job, or reports the one already running.
    ///
    /// `what` is the user-facing job name ("export", "transcription", ...).
    pub fn begin(self: &Arc<Self>, what: &str) -> Result<Job, String> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| format!("{what} state poisoned"))?;
        if slot.is_some() {
            return Err(format!(
                "a {what} is already running - wait for it or cancel it first"
            ));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Arc::clone(&cancel));
        Ok(Job {
            owner: Arc::clone(self),
            cancel,
        })
    }

    /// Whether a job currently holds the slot.
    pub fn is_busy(&self) -> bool {
        self.slot.lock().map(|slot| slot.is_some()).unwrap_or(false)
    }

    /// Cancels the running job. Idle is a harmless no-op.
    pub fn cancel(&self) {
        if let Ok(slot) = self.slot.lock()
            && let Some(cancel) = slot.as_ref()
        {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// A claim on the slot, released on drop - however the job ended, panic
/// included, the slot frees and the next `begin` succeeds.
pub struct Job {
    owner: Arc<SingleFlight>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// This run's stop flag, for the worker to poll.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// An owned handle on the same flag, for callbacks that outlive borrows
    /// of the job (an FFI progress callback must be `'static`).
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Whether this run has been asked to stop.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.owner.slot.lock() {
            // Release only our own claim. Guards are one-per-begin so the
            // check is belt and braces, but it makes a stale drop harmless.
            if slot
                .as_ref()
                .is_some_and(|flag| Arc::ptr_eq(flag, &self.cancel))
            {
                *slot = None;
            }
        }
    }
}

/// A file that says a job is running on the folder it names, removed
/// when the job ends by any road the process survives: done, failed,
/// cancelled. Left behind, it says the process died inside the job.
///
/// A job the window starts by itself whenever its work is outstanding -
/// a smart stroke's regions - would otherwise start again on the next
/// open and take the app down with it again: a project that closed the
/// app every time it was opened, with the person's edits shut inside it.
/// The window skips a tripped job and says so; the work is redone only
/// when the person asks for it anew.
pub struct Tripwire {
    path: std::path::PathBuf,
}

impl Tripwire {
    const FILE: &'static str = "running";

    /// Marks `dir` as having a job running on it. A folder that cannot
    /// be written to gets no mark, and the job runs unguarded.
    pub fn arm(dir: &std::path::Path) -> Tripwire {
        let path = dir.join(Self::FILE);
        let _ = std::fs::create_dir_all(dir);
        if let Err(error) = std::fs::write(&path, std::process::id().to_string()) {
            log::warn!("could not mark {} as running: {error}", dir.display());
        }
        Tripwire { path }
    }

    /// The mark's own file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Takes the mark off `dir`, for a job the person has asked to be
    /// tried again.
    pub fn clear(dir: &std::path::Path) {
        let _ = std::fs::remove_file(dir.join(Self::FILE));
    }

    /// Whether a job on `dir` was running when its process died: the mark
    /// is there and the process that left it is not running any more. A
    /// second Concat open beside the first sees the first's mark as a job
    /// running, not as a death. Only meaningful while no job on `dir` is
    /// running in this process.
    pub fn tripped(dir: &std::path::Path) -> bool {
        let Ok(text) = std::fs::read_to_string(dir.join(Self::FILE)) else {
            return false;
        };
        match text.trim().parse::<u32>() {
            Ok(pid) => pid == std::process::id() || !alive(pid),
            // A mark with no process in it was left by a death.
            Err(_) => true,
        }
    }
}

impl Drop for Tripwire {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether process `pid` is running.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; it only asks whether `pid` could be
    // signalled, and touches no memory of ours.
    let answer = unsafe { libc::kill(pid, 0) };
    answer == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether process `pid` is running.
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: a handle opened here, read once and closed once; a null one
    // (no such process, or one we may not open) is never used.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0u32;
        let read = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        read != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(any(unix, windows)))]
fn alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_left_by_a_running_process_is_not_a_death() {
        let dir =
            std::env::temp_dir().join(format!("concat-tripwire-alive-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // pid 1 is always running on a unix.
        if cfg!(unix) {
            std::fs::write(dir.join("running"), "1").expect("writes");
            assert!(!Tripwire::tripped(&dir), "a live process's mark");
        }
        std::fs::write(dir.join("running"), "not a pid").expect("writes");
        assert!(Tripwire::tripped(&dir), "a mark with no process in it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tripwire_is_left_only_by_a_job_that_never_ended() {
        let dir = std::env::temp_dir().join(format!("concat-tripwire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let _wire = Tripwire::arm(&dir);
            assert!(Tripwire::tripped(&dir), "marked while it runs");
        }
        assert!(!Tripwire::tripped(&dir), "cleared when it ends");
        // A process that dies inside the job runs no destructor.
        std::mem::forget(Tripwire::arm(&dir));
        assert!(Tripwire::tripped(&dir), "left by a job that never ended");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_a_second_concurrent_job() {
        let flight = Arc::new(SingleFlight::new());
        let job = flight.begin("export").expect("first begin");
        assert!(flight.is_busy());
        match flight.begin("export") {
            Err(error) => assert!(error.contains("already running")),
            Ok(_) => panic!("second begin must be refused"),
        }
        drop(job);
        assert!(!flight.is_busy());
        assert!(flight.begin("export").is_ok(), "slot frees on drop");
    }

    #[test]
    fn cancel_reaches_only_the_running_job() {
        let flight = Arc::new(SingleFlight::new());
        let first = flight.begin("export").expect("begin");
        flight.cancel();
        assert!(first.cancelled());

        drop(first);
        // A new job starts with a fresh, uncancelled flag: the earlier cancel
        // cannot leak forward, and starting fresh cannot un-cancel anyone.
        let second = flight.begin("export").expect("begin again");
        assert!(!second.cancelled());
    }

    #[test]
    fn cancel_when_idle_is_a_no_op() {
        let flight = Arc::new(SingleFlight::new());
        flight.cancel();
        assert!(flight.begin("export").is_ok());
    }
}
