// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The two things UniFFI cannot carry: the app's `Context` (which cpal's
//! AAudio host reads through ndk-context to size its buffers) and a
//! `Surface` (whose ANativeWindow the render thread draws on). Both are
//! plain JNI functions on `app.concat.editor.ConcatNative`, which the
//! Kotlin side reaches after `System.loadLibrary("concat_mobile")`.

use jni_sys::{JNIEnv, jclass, jlong, jobject, jstring};

/// `ConcatNative.init(context, files)`: the log - a file in the app's
/// folder, which Settings shares, and logcat - and the JavaVM and Context
/// for ndk-context. `files` is the app's private files folder. Once per
/// process; later calls do nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_concat_editor_ConcatNative_init(
    env: *mut JNIEnv,
    _class: jclass,
    context: jobject,
    files: jstring,
) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: env is the calling thread's JNIEnv, valid for this call,
        // and `files` a live java.lang.String from it.
        let files = unsafe { string_of(env, files) };
        open_log(files.as_deref());
        // SAFETY: env is the calling thread's JNIEnv, valid for this call.
        unsafe {
            let functions = &**env;
            let mut vm = std::ptr::null_mut();
            let get_vm = functions.GetJavaVM.expect("GetJavaVM");
            if get_vm(env, &mut vm) != 0 || vm.is_null() {
                log::error!("no JavaVM");
                return;
            }
            let global = functions.NewGlobalRef.expect("NewGlobalRef")(env, context);
            ndk_context::initialize_android_context(vm.cast(), global.cast());
        }
        log::info!("Concat {} starting", env!("CARGO_PKG_VERSION"));
    });
}

/// The log, as the desktop and the Slint app before this one kept it: a
/// file under the app's data folder - `files/app.concat.editor/logs`, the
/// folder the engine's settings and models are in - with every line also
/// in logcat, and panics and native crashes written into it.
fn open_log(files: Option<&str>) {
    let logcat = android_logger::AndroidLogger::new(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("concat"),
    );
    if let Some(files) = files {
        // The bases the engine finds its folders under, as the Slint app
        // named them: the app's private files folder.
        // SAFETY: called once, from the first native call, before the
        // engine starts a thread; the JVM's threads do not read the C
        // environment.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", files);
            std::env::set_var("XDG_DATA_HOME", files);
        }
    }
    concat_host::crash::catch();
    concat_host::logs::catch_panics();
    let opened = match concat_host::AppDirs::locate() {
        Ok(dirs) => concat_host::logs::open(&dirs, Some(Box::new(logcat))).map(|_| ()),
        Err(error) => concat_host::logs::open_console(Some(Box::new(logcat))).and(Err(error)),
    };
    if let Err(error) = opened {
        log::warn!("log: no file for this run's log ({error}); it goes to logcat only");
    }
}

/// A Java string's characters, or None for null.
///
/// # Safety
/// `env` is the calling thread's JNIEnv and `string` a reference from it.
unsafe fn string_of(env: *mut JNIEnv, string: jstring) -> Option<String> {
    if string.is_null() {
        return None;
    }
    // SAFETY: as the function's contract says; the characters are let go
    // before returning.
    unsafe {
        let functions = &**env;
        let chars = functions.GetStringUTFChars?(env, string, std::ptr::null_mut());
        if chars.is_null() {
            return None;
        }
        let owned = std::ffi::CStr::from_ptr(chars)
            .to_string_lossy()
            .into_owned();
        functions.ReleaseStringUTFChars?(env, string, chars);
        Some(owned)
    }
}

/// `ConcatNative.windowFromSurface(surface)`: the Surface's ANativeWindow
/// with one reference acquired, as a long, or 0. `Engine.attachSurface`
/// takes that reference over.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_concat_editor_ConcatNative_windowFromSurface(
    env: *mut JNIEnv,
    _class: jclass,
    surface: jobject,
) -> jlong {
    // SAFETY: env and surface are this call's own arguments.
    let window = unsafe { ndk_sys::ANativeWindow_fromSurface(env.cast(), surface.cast()) };
    window as jlong
}
