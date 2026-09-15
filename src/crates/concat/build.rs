// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Compiles the `.slint` tree into the binary, embeds the built-in text
//! presets, and records three facts about the build for Settings > About.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    embed_text_presets();
    // Fonts and images are compiled into the binary rather than read off disk
    // at run time.
    //
    // `EmbedFiles` keeps each file exactly as it is — a PNG stays compressed,
    // a TTF stays a TTF — and hands it to the renderer from memory instead of
    // opening it. What that buys is a startup that touches no files and a
    // binary that is the whole application: six font faces, a logo and twenty
    // effect previews travel inside it, so there is no directory to ship
    // beside it and no path to get wrong.
    //
    // Not `EmbedForSoftwareRenderer`, which pre-decodes to raw pixels: that is
    // for MCUs with no filesystem, it is the only kind the software renderer
    // can read, and Skia and FemtoVG cannot use it at all.
    //
    // On its own thread with a deep stack: the Slint compiler recurses over
    // the tree, and the tree has outgrown the megabyte a main thread gets
    // on Windows - the release build died there with a stack overflow and
    // nothing else to say. Half a gigabyte is reserved, not committed.
    let compile = std::thread::Builder::new()
        .name("slint".into())
        .stack_size(512 << 20)
        .spawn(|| {
            let config = slint_build::CompilerConfiguration::new()
                .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
            slint_build::compile_with_config("ui/app.slint", config)
        })
        .expect("could not start the Slint compiler thread");
    compile
        .join()
        .expect("the Slint compiler thread panicked")
        .expect("failed to compile ui/app.slint");

    // Three facts about the build that the built thing cannot ask for at run
    // time. Settings > About shows them in the block a bug report is copied
    // out of: which triple this binary is for, which profile it came out of,
    // and which compiler made it — the three questions every "cannot
    // reproduce" ends up asking.
    //
    // slint_build emits rerun-if-changed for the .slint tree, which turns off
    // cargo's rerun-on-any-change default, so the toolchain is watched by
    // hand: change rustc and this file has to run again or `Toolchain` would
    // name the old one.
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!(
        "cargo:rustc-env=BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
    println!(
        "cargo:rustc-env=BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into())
    );
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let version = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BUILD_RUSTC={version}");
}

/// Embeds every built-in text preset under `text-presets/`, the same way
/// `concat-effects`'s `build.rs` embeds effect packages: each folder holds a
/// `preset.toml`, and this script lists them and writes a table of
/// `include_str!`s, so adding a built-in preset is adding a folder - nothing
/// in Rust names it.
fn embed_text_presets() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let presets = root.join("text-presets");
    println!("cargo:rerun-if-changed={}", presets.display());

    let mut ids: Vec<String> = std::fs::read_dir(&presets)
        .expect("text-presets/ exists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().join("preset.toml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();

    let mut table = String::from(
        "/// Every built-in text preset: its folder name and its TOML.\n\
         pub(crate) static BUILTIN_PRESET_SOURCES: &[(&str, &str)] = &[\n",
    );
    for id in &ids {
        let path = presets.join(id).join("preset.toml");
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(
            table,
            "    ({id:?}, include_str!({:?})),",
            path.display().to_string()
        )
        .expect("write");
    }
    table.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("text_presets.rs");
    std::fs::write(out, table).expect("write text_presets.rs");
}
