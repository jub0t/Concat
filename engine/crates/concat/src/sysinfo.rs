// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What Settings > About reports about this machine and this build.

pub fn os_description() -> String {
    #[cfg(target_os = "macos")]
    {
        // Bare `sw_vers` prints all three lines at once, so this is one
        // process rather than the three the flags would cost.
        let fields = std::process::Command::new("sw_vers")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        let field = |key: &str| {
            fields
                .lines()
                .find_map(|line| line.strip_prefix(key)?.split(':').nth(1))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let name = field("ProductName").unwrap_or_else(|| "macOS".into());
        match (field("ProductVersion"), field("BuildVersion")) {
            (Some(version), Some(build)) => format!("{name} {version} ({build})"),
            (Some(version), None) => format!("{name} {version}"),
            _ => name,
        }
    }
    #[cfg(target_os = "linux")]
    {
        // The distribution's own name for itself, which is what a report
        // wants; the kernel version is the next question, not the first.
        if let Ok(release) = std::fs::read_to_string("/etc/os-release") {
            if let Some(pretty) = release
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"'))
                .filter(|value| !value.is_empty())
            {
                return pretty.to_owned();
            }
        }
        return "Linux".into();
    }
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Windows".into());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// What Settings > About shows under "System information", and what its copy
/// button puts on the clipboard.
///
/// One list, read twice: the rows on screen and the block that is copied are
/// built from the same pairs, so a fact cannot be on the page and missing
/// from the report. Gathered once — every line of it is fixed for the life of
/// the process — and handed over as a model that is never replaced.
pub fn system_facts() -> Vec<(&'static str, String)> {
    vec![
        (
            "Application",
            format!("Concat {}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "Build",
            format!("{} · {}", env!("BUILD_PROFILE"), env!("BUILD_TARGET")),
        ),
        // Which of the two renderers in Cargo.toml this binary was built
        // with. The first question to ask about anything that looks wrong on
        // screen, and the one nobody can answer by looking at the window.
        (
            "Renderer",
            if cfg!(feature = "skia") {
                "Skia"
            } else {
                "FemtoVG (wgpu)"
            }
            .into(),
        ),
        (
            "Engine",
            format!("concat-engine · FFmpeg {}", concat_media::linked_version()),
        ),
        ("Operating system", os_description()),
        (
            "Processor",
            format!(
                "{} · {} threads",
                std::env::consts::ARCH,
                std::thread::available_parallelism().map_or(0, |count| count.get())
            ),
        ),
        ("Toolchain", env!("BUILD_RUSTC").into()),
    ]
}
