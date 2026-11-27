//! What FFmpeg's static archives need from a phone.
//!
//! On the desktop FFmpeg arrives as shared libraries that carry their own
//! dependencies. A phone links the archives that
//! `scripts/ffmpeg-mobile.sh` builds, and an archive brings nothing with
//! it: the platform libraries its code calls into - zlib, the hardware
//! codec bridges and the JNI shim - have to be named here.
//! The list is the `Libs:` line of the pkg-config files that build writes.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // MediaCodec through the NDK, the camera device, and the JNI it all
    // rides on. libatomic backs the lock-free counters FFmpeg uses on
    // AArch64.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        for lib in ["z", "m", "atomic", "android", "mediandk", "camera2ndk"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}
