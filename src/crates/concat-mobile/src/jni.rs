// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The two things UniFFI cannot carry: the app's `Context` (which cpal's
//! AAudio host reads through ndk-context to size its buffers) and a
//! `Surface` (whose ANativeWindow the render thread draws on). Both are
//! plain JNI functions on `org.standalones.concat.spike.ConcatNative`, which the
//! Kotlin side reaches after `System.loadLibrary("concat_mobile")`.

use jni_sys::{JNIEnv, jclass, jlong, jobject};

/// `ConcatNative.init(context)`: logcat, and the JavaVM and Context for
/// ndk-context. Once per process; later calls do nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_standalones_concat_spike_ConcatNative_init(
    env: *mut JNIEnv,
    _class: jclass,
    context: jobject,
) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("concat-spike"),
        );
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
        log::info!("concat-spike {} native side up", env!("CARGO_PKG_VERSION"));
    });
}

/// `ConcatNative.windowFromSurface(surface)`: the Surface's ANativeWindow
/// with one reference acquired, as a long, or 0. `Engine.attachSurface`
/// takes that reference over.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_org_standalones_concat_spike_ConcatNative_windowFromSurface(
    env: *mut JNIEnv,
    _class: jclass,
    surface: jobject,
) -> jlong {
    // SAFETY: env and surface are this call's own arguments.
    let window = unsafe { ndk_sys::ANativeWindow_fromSurface(env.cast(), surface.cast()) };
    window as jlong
}
