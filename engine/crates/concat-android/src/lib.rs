// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The activity's entry point. android-activity calls `android_main` on
//! its own thread once the activity is up; Slint's backend takes the
//! activity from there, and the window runs exactly as it does anywhere
//! else.
//!
//! Two things are settled here before the window starts, because only the
//! activity knows them. The app's directories: an Android process has no
//! home directory, so the host's XDG bases are pointed at the app's own
//! files folder, and its external files folder stands in for the home the
//! project and export defaults hang off. And where words go: a process on
//! a phone has no terminal, so the log facade and everything the window
//! prints to stderr are forwarded to logcat under the `concat` tag, which
//! is where `adb logcat -s concat` reads a report from.

#[cfg(target_os = "android")]
mod activity {
    use std::io::BufRead;

    /// Names the app's directories for the host. Set once, before any other
    /// thread exists, which is what makes writing the environment sound.
    pub fn name_directories(app: &slint::android::AndroidApp) {
        let Some(internal) = app.internal_data_path() else {
            return;
        };
        // What the phone keeps for the app: settings, recents, models.
        // SAFETY: called from android_main before the window or any worker
        // thread starts, so no other thread reads the environment.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &internal);
            std::env::set_var("XDG_DATA_HOME", &internal);
            // Projects and exports land under the folder the user can reach
            // through the phone's file manager, falling back to the private
            // one when there is no external storage.
            let home = app.external_data_path().unwrap_or(internal);
            std::env::set_var("HOME", home);
        }
    }

    /// Routes the log facade, panics and stderr to logcat, and to a file.
    ///
    /// logcat is what a phone on a cable gives you, and nothing else is as
    /// good while there is a cable. A phone in somebody's hand has none, so
    /// the same lines are written into the app's own storage as well; the
    /// window's Settings is where they are found. Called after
    /// [`name_directories`], because that is what says where storage is.
    pub fn open_log() {
        // Info is logcat's floor, as it always was; CONCAT_LOG is the
        // ceiling over both sinks, so turning the file up to debug does not
        // also flood logcat, and turning everything down still quiets it.
        let logcat = android_logger::AndroidLogger::new(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("concat"),
        );
        concat::open_logging(Some(Box::new(logcat)));
        forward_stderr();
    }

    /// Everything written to stderr is read back off a pipe and logged a
    /// line at a time, so a stray `eprintln!` in a dependency reaches logcat
    /// without that dependency knowing about phones. The app's own lines do
    /// not come this way - they go through the facade, which on a phone
    /// deliberately leaves stderr alone so a line cannot come back round
    /// this pipe and log itself forever.
    fn forward_stderr() {
        use std::os::fd::FromRawFd;
        let mut ends = [0i32; 2];
        // SAFETY: plain libc calls on descriptors this function owns; the
        // read end is handed to exactly one File.
        let reader = unsafe {
            if libc::pipe(ends.as_mut_ptr()) != 0 {
                return;
            }
            if libc::dup2(ends[1], libc::STDERR_FILENO) < 0 {
                libc::close(ends[0]);
                libc::close(ends[1]);
                return;
            }
            libc::close(ends[1]);
            std::fs::File::from_raw_fd(ends[0])
        };
        std::thread::Builder::new()
            .name("stderr-to-logcat".into())
            .spawn(move || {
                for line in std::io::BufReader::new(reader)
                    .lines()
                    .map_while(Result::ok)
                {
                    log::warn!("{line}");
                }
            })
            .ok();
    }
}

/// Called by the activity's native glue; the name is the contract.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    // Directories first: the log file is written into one of them.
    activity::name_directories(&app);
    activity::open_log();
    log::info!("Concat {} starting", env!("CARGO_PKG_VERSION"));
    if let Err(error) = slint::android::init(app) {
        log::error!("could not start the Android backend: {error}");
        return;
    }
    if let Err(error) = concat::run() {
        log::error!("{error}");
    }
}
