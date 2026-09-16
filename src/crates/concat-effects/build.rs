// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Embeds every package under `packages/` into the binary.
//!
//! Each package is a folder named after its id, holding `effect.toml` and
//! optionally `fixtures.toml`. This script lists the folders and writes a
//! table of `include_str!`s, so adding a built-in effect is adding a folder:
//! nothing in Rust names it.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let packages = root.join("packages");
    println!("cargo:rerun-if-changed={}", packages.display());

    let mut ids: Vec<String> = std::fs::read_dir(&packages)
        .expect("packages/ exists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().join("effect.toml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();

    let mut table = String::from(
        "/// Every built-in package: its folder name, its manifest, its \
         fixtures if it has any, and its shader if it has one.\n\
         pub(crate) static BUILTIN_SOURCES: &[(&str, &str, Option<&str>, Option<&str>)] = &[\n",
    );
    for id in &ids {
        let dir = packages.join(id);
        let manifest = dir.join("effect.toml");
        let fixtures = dir.join("fixtures.toml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", fixtures.display());
        let fixtures = if fixtures.is_file() {
            format!("Some(include_str!({:?}))", fixtures.display().to_string())
        } else {
            "None".to_owned()
        };
        let shader = dir.join("effect.wgsl");
        println!("cargo:rerun-if-changed={}", shader.display());
        let shader = if shader.is_file() {
            format!("Some(include_str!({:?}))", shader.display().to_string())
        } else {
            "None".to_owned()
        };
        writeln!(
            table,
            "    ({id:?}, include_str!({:?}), {fixtures}, {shader}),",
            manifest.display().to_string()
        )
        .expect("write");
    }
    table.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("builtins.rs");
    std::fs::write(out, table).expect("write builtins.rs");
}
