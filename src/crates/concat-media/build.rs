//! What FFmpeg's static archives need from a phone.
//!
//! On the desktop FFmpeg arrives as shared libraries that carry their own
//! dependencies. A phone links the archives that
//! `scripts/ffmpeg-mobile.sh` builds, and an archive brings nothing with
//! it: the platform libraries its code calls into - zlib, the hardware
//! codec bridges, and on Android the JNI shim - have to be named here.
//! The list is the `Libs:` line of the pkg-config files that build writes.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match os.as_str() {
        // MediaCodec through the NDK, the camera device, and the JNI it all
        // rides on. libatomic backs the lock-free counters FFmpeg uses on
        // AArch64.
        "android" => {
            for lib in ["z", "m", "atomic", "android", "mediandk", "camera2ndk"] {
                println!("cargo:rustc-link-lib={lib}");
            }
            // ffmpeg-sys-the-third runs bindgen at build time, which fails on
            // Android because the host's libclang expects glibc headers, not
            // bionic. Pre-generate the bindings with bindgen CLI using the
            // NDK's clang in ffmpeg-mobile.sh, then skip bindgen here.
            println!("cargo:rustc-env=FFMPEG_SYS_NO_BINDGEN=1");
            println!("cargo:rerun-if-env-changed=FFMPEG_SYS_NO_BINDGEN");
            println!("cargo:rerun-if-env-changed=FFMPEG_SYS_BINDINGS_DIR");
            let bindings_dir = std::env::var("FFMPEG_SYS_BINDINGS_DIR").unwrap_or_else(|_| {
                let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
                let vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();
                let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
                let env_val = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
                let target = if env_val.is_empty() {
                    format!("{arch}-{vendor}-{os}")
                } else {
                    format!("{arch}-{vendor}-{os}-{env_val}")
                };
                let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
                let ws_root = Path::new(&manifest_dir)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                format!("{ws_root}/vendor/ffmpeg-bindings/{target}")
            });
            println!("cargo:rustc-env=FFMPEG_SYS_BINDINGS_DIR={bindings_dir}");
        }
        // VideoToolbox and AudioToolbox, the hardware codecs, the
        // frameworks they hand buffers through, and Security for the TLS
        // behind https.
        "ios" => {
            for lib in ["z", "m"] {
                println!("cargo:rustc-link-lib={lib}");
            }
            for framework in [
                "VideoToolbox",
                "AudioToolbox",
                "CoreMedia",
                "CoreVideo",
                "CoreFoundation",
                "Foundation",
                "Security",
            ] {
                println!("cargo:rustc-link-lib=framework={framework}");
            }
        }
        _ => {}
    }
}
