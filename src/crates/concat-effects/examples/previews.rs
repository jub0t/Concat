// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Renders every look's preview still: the package's own FFmpeg chain at
//! its defaults, run over one reference picture, so the card in the Library
//! shows the real look and not a guess at it.
//!
//! ```text
//! cargo run -p concat-effects --example previews -- <still> <out dir> [id...]
//! ```
//!
//! With no ids, every filter package is rendered; the file is named after
//! the id without its namespace, `concat.golden-hour` as `golden-hour.jpg`.
//! Needs `ffmpeg` on the path.

use std::path::PathBuf;
use std::process::Command;

use concat_effects::{At, Catalogue, Kind};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(still), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: previews <still> <out dir> [id...]");
        std::process::exit(2);
    };
    let only: Vec<String> = args.collect();
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("out dir");

    let mut failed = 0;
    for package in Catalogue::builtin().of_kind(Kind::Filter) {
        let id = package.id();
        if !only.is_empty() && !only.iter().any(|want| want == id) {
            continue;
        }
        let chain = match package.ffmpeg_fragment(&package.params_at(At::Default), 0) {
            Ok(Some(chain)) => chain,
            Ok(None) => {
                eprintln!("{id}: no ffmpeg chain, skipped");
                continue;
            }
            Err(error) => {
                eprintln!("{id}: {error}");
                failed += 1;
                continue;
            }
        };
        let name = id.rsplit('.').next().unwrap_or(id);
        let target = out.join(format!("{name}.jpg"));
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-i",
                &still,
                "-vf",
                &chain,
                "-q:v",
                "3",
            ])
            .arg(&target)
            .status();
        match status {
            Ok(status) if status.success() => println!("{id} -> {}", target.display()),
            Ok(status) => {
                eprintln!("{id}: ffmpeg exited with {status}\n  chain: {chain}");
                failed += 1;
            }
            Err(error) => {
                eprintln!("{id}: could not run ffmpeg: {error}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
