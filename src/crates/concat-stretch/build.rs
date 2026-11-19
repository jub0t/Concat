// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Compiles Bungee and PFFFT from `third_party/` for the target being
//! built, with the definitions Bungee's own CMake gives them.

use std::path::Path;

fn main() {
    let third_party = Path::new("third_party");
    println!("cargo:rerun-if-changed=third_party");
    println!("cargo:rerun-if-changed=build.rs");

    let mut bungee = cc::Build::new();
    bungee
        .cpp(true)
        .std("c++20")
        .include(third_party.join("bungee"))
        .include(third_party.join("eigen"))
        .include(third_party.join("pffft"))
        // Linked statically into the app: nothing to export.
        .define("BUNGEE_VISIBILITY", Some(""))
        .define("BUNGEE_SELF_TEST", Some("0"))
        .define("eigen_assert", Some("BUNGEE_ASSERT1"))
        .define("EIGEN_DONT_PARALLELIZE", Some("1"))
        .define("BUNGEE_VERSION", Some("\"61f52bf\""))
        // Signed overflow wraps, as Bungee's CMake asks of GCC and Clang.
        .flag_if_supported("-fwrapv")
        // Eigen's headers are not ours to warn about.
        .warnings(false)
        // The stretcher runs in every test and every debug build that
        // plays a sped-up clip; unoptimised Eigen is tens of times slower.
        .opt_level(3);
    let sources = third_party.join("bungee").join("src");
    let mut files: Vec<_> = std::fs::read_dir(&sources)
        .expect("third_party/bungee/src is there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "cpp"))
        .collect();
    files.sort();
    bungee.files(&files);
    bungee.compile("bungee");

    let mut pffft = cc::Build::new();
    pffft
        .file(third_party.join("pffft").join("pffft.c"))
        .warnings(false)
        .opt_level(3)
        // As Bungee's CMake builds it: fast maths, but infinities and NaN
        // kept meaningful.
        .flag_if_supported("-ffast-math")
        .flag_if_supported("-fno-finite-math-only");
    pffft.compile("pffft");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // Bungee's instrumentation logs through Android's log, though
        // Concat never turns it on.
        println!("cargo:rustc-link-lib=log");
    }
}
