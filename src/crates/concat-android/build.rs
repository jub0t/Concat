// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Compiles the Java the activity needs - the document picker fragment in
//! java/ - into one dex file the binary carries and loads at run time.
//! The same way Slint's Android backend builds its own helper: the SDK's
//! javac and d8, found through ANDROID_HOME and JAVA_HOME. Nothing to do
//! for any other target.

fn main() {
    println!("cargo:rerun-if-changed=java/ConcatFiles.java");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("android") {
        return;
    }

    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let classes = out_dir.join("java");
    if classes.exists() {
        let _ = std::fs::remove_dir_all(&classes);
    }
    std::fs::create_dir_all(&classes).expect("could not create the class directory");

    let android_jar = android_build::android_jar(None).expect("no Android platform found");
    let compiled = android_build::JavaBuild::new()
        .file("java/ConcatFiles.java")
        .class_path(&android_jar)
        .classes_out_dir(&classes)
        .java_source_version(8)
        .java_target_version(8)
        .debug_info(android_build::DebugInfo {
            line_numbers: !release,
            variables: !release,
            source_files: !release,
        })
        .command()
        .expect("could not build the javac command")
        .args(["-encoding", "UTF-8"])
        .output()
        .expect("could not run javac");
    if !compiled.status.success() {
        panic!(
            "javac failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
    }

    let dexed = android_build::Dexer::new()
        .android_jar(&android_jar)
        .class_path(&classes)
        .collect_classes(&classes)
        .expect("could not collect the classes")
        .release(release)
        .android_min_api(26)
        .out_dir(&out_dir)
        .command()
        .expect("could not build the d8 command")
        .output()
        .expect("could not run d8");
    if !dexed.status.success() {
        panic!("d8 failed: {}", String::from_utf8_lossy(&dexed.stderr));
    }
}
