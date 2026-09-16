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

/// The system's document picker, reached through the Java in java/.
///
/// The window is a NativeActivity with no Java of its own, and a picker's
/// answer comes back only through Java; so the Java is a fragment
/// compiled by build.rs into a dex the binary carries, loaded here through
/// an in-memory class loader, and told - by registering a native method on
/// it - where to bring the answer. The picked files are copied into the
/// app's own storage by the Java, and their paths are what comes back.
#[cfg(target_os = "android")]
mod picker {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    use jni::objects::{Global, JClass, JObject, JObjectArray, JString};
    use jni::{Env, JavaVM, jni_sig, jni_str, sys};
    use slint::android::AndroidApp;

    /// The classes build.rs compiled: app.concat.editor.ConcatFiles.
    const DEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex"));
    const CLASS_NAME: &str = "app.concat.editor.ConcatFiles";

    type Picked = Box<dyn FnOnce(Vec<PathBuf>) + Send>;

    /// The pick in flight, waiting for Java to answer.
    static PENDING: Mutex<Option<Picked>> = Mutex::new(None);
    /// The fragment class, loaded once and kept.
    static CLASS: OnceLock<Global<JClass<'static>>> = OnceLock::new();

    /// Hands the window's crate a picker that runs on this activity.
    pub fn install(app: &AndroidApp) {
        let app = app.clone();
        concat::install_file_picker(Box::new(move |on_picked| {
            let previous = PENDING
                .lock()
                .map(|mut slot| slot.replace(on_picked))
                .ok()
                .flatten();
            if let Some(previous) = previous {
                // A pick was already up; the one that asked first is told
                // it got nothing rather than left waiting for ever.
                previous(Vec::new());
            }
            if let Err(error) = pick(&app) {
                log::error!("could not show the document picker: {error}");
                if let Some(pending) = PENDING.lock().ok().and_then(|mut slot| slot.take()) {
                    pending(Vec::new());
                }
            }
        }));
    }

    fn pick(app: &AndroidApp) -> jni::errors::Result<()> {
        // SAFETY: the pointer is the activity's JavaVM, live for the
        // process; `from_raw` also seeds `JavaVM::singleton`, which the
        // native callback below reaches for.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| {
            // SAFETY: the activity pointer is a live reference the app
            // holds for as long as it runs; it is only read here.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            let class = match CLASS.get() {
                Some(class) => class,
                None => {
                    let loaded = load_class(env, &activity)?;
                    let _ = CLASS.set(loaded);
                    CLASS.get().expect("set just above")
                }
            };
            env.call_static_method(
                class,
                jni_str!("pick"),
                jni_sig!("(Landroid/app/Activity;)V"),
                &[(&activity).into()],
            )?;
            Ok(())
        })
    }

    /// Loads the fragment class from the dex through the activity's own
    /// class loader, and registers `filesPicked` on it.
    fn load_class(
        env: &mut Env,
        activity: &JObject,
    ) -> jni::errors::Result<Global<JClass<'static>>> {
        let parent = env
            .call_method(
                activity,
                jni_str!("getClassLoader"),
                jni_sig!("()Ljava/lang/ClassLoader;"),
                &[],
            )?
            .l()?;
        // SAFETY: DEX is 'static and the loader never writes to it.
        let buffer = unsafe { env.new_direct_byte_buffer(DEX.as_ptr().cast_mut(), DEX.len()) }?;
        let loader = env.new_object(
            jni_str!("dalvik/system/InMemoryDexClassLoader"),
            jni_sig!("(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V"),
            &[(&buffer).into(), (&parent).into()],
        )?;
        let name = env.new_string(CLASS_NAME)?;
        let class = env
            .call_method(
                &loader,
                jni_str!("loadClass"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
                &[(&name).into()],
            )?
            .l()?;
        let class = JClass::cast_local(env, class)?;
        // SAFETY: the signature is the Java declaration's, and the
        // function below takes exactly what it names.
        let method = unsafe {
            jni::NativeMethod::from_raw_parts(
                jni_str!("filesPicked"),
                jni_str!("([Ljava/lang/String;)V"),
                files_picked as *mut c_void,
            )
        };
        // SAFETY: as above.
        unsafe { env.register_native_methods(&class, &[method]) }?;
        env.new_global_ref(class)
    }

    /// `ConcatFiles.filesPicked`, on whichever thread the Java copied on.
    unsafe extern "system" fn files_picked(
        _env: *mut sys::JNIEnv,
        _class: sys::jclass,
        paths: sys::jobjectArray,
    ) {
        let read = JavaVM::singleton().and_then(|vm| {
            vm.attach_current_thread(|env| {
                // SAFETY: `paths` is the argument Java handed this frame.
                let array = unsafe { JObjectArray::<JString>::from_raw(env, paths) };
                let mut out = Vec::new();
                for index in 0..array.len(env)? {
                    let item = array.get_element(env, index)?;
                    out.push(PathBuf::from(item.mutf8_chars(env)?.to_string()));
                }
                Ok::<_, jni::errors::Error>(out)
            })
        });
        let paths = match read {
            Ok(paths) => paths,
            Err(error) => {
                log::error!("could not read the picked files: {error}");
                Vec::new()
            }
        };
        if let Some(pending) = PENDING.lock().ok().and_then(|mut slot| slot.take()) {
            pending(paths);
        }
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
    if let Err(error) = slint::android::init(app.clone()) {
        log::error!("could not start the Android backend: {error}");
        return;
    }
    // After the backend, which seeds the JavaVM the picker reaches for.
    picker::install(&app);
    if let Err(error) = concat::run() {
        log::error!("{error}");
    }
}
