// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The machine a run is on, as the first lines of its log say it.
//!
//! The questions every crash report starts with - which Windows, which
//! CPU, does it have AVX2, which build is this - each took a round trip
//! with the reporter (#313, #318). The log now answers them before it is
//! asked. Nothing here is sent anywhere; it is written into the run's own
//! log file like every other line.

/// One line about the build: the version, the target, and the commit CI
/// built it from - "local" for a build from a working tree.
pub fn build() -> String {
    format!(
        "Concat {} · {}-{} · commit {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        option_env!("GITHUB_SHA").map_or("local", |sha| &sha[..sha.len().min(12)]),
    )
}

/// One line about the system: its name and version, and the device where
/// a phone says which one it is.
pub fn system() -> String {
    os::describe().unwrap_or_else(|| std::env::consts::OS.to_owned())
}

/// One line about the CPU: its name where the system says it, how many
/// threads it runs, and which of the instruction sets the libraries Concat
/// links care about it has - the ones a build for a newer CPU than the
/// one it is on dies over.
pub fn cpu() -> String {
    let mut line = cpu_name().unwrap_or_else(|| std::env::consts::ARCH.to_owned());
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    line.push_str(&format!(" · {threads} threads"));
    let (has, lacks) = features();
    if !has.is_empty() {
        line.push_str(&format!(" · has {}", has.join(" ")));
    }
    if !lacks.is_empty() {
        line.push_str(&format!(" · lacks {}", lacks.join(" ")));
    }
    line
}

/// The instruction sets worth naming, split into those the CPU has and
/// those it does not.
fn features() -> (Vec<&'static str>, Vec<&'static str>) {
    #[allow(unused_mut)]
    let mut found: Vec<(&'static str, bool)> = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        found.extend([
            ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
            ("avx", std::arch::is_x86_feature_detected!("avx")),
            ("avx2", std::arch::is_x86_feature_detected!("avx2")),
            ("fma", std::arch::is_x86_feature_detected!("fma")),
            ("bmi2", std::arch::is_x86_feature_detected!("bmi2")),
            ("f16c", std::arch::is_x86_feature_detected!("f16c")),
            ("avx512f", std::arch::is_x86_feature_detected!("avx512f")),
        ]);
    }
    #[cfg(target_arch = "aarch64")]
    {
        found.extend([
            ("neon", std::arch::is_aarch64_feature_detected!("neon")),
            ("fp16", std::arch::is_aarch64_feature_detected!("fp16")),
            (
                "dotprod",
                std::arch::is_aarch64_feature_detected!("dotprod"),
            ),
            ("i8mm", std::arch::is_aarch64_feature_detected!("i8mm")),
            ("sve", std::arch::is_aarch64_feature_detected!("sve")),
        ]);
    }
    let has = found
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect();
    let lacks = found
        .iter()
        .filter(|(_, on)| !*on)
        .map(|(n, _)| *n)
        .collect();
    (has, lacks)
}

/// The CPU's marketing name.
fn cpu_name() -> Option<String> {
    #[cfg(windows)]
    {
        os::registry(
            r"HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
        )
    }
    #[cfg(target_vendor = "apple")]
    {
        os::sysctl("machdep.cpu.brand_string")
    }
    #[cfg(all(not(windows), not(target_vendor = "apple"), target_arch = "x86_64"))]
    {
        x86_brand()
    }
    #[cfg(all(
        not(windows),
        not(target_vendor = "apple"),
        not(target_arch = "x86_64")
    ))]
    {
        None
    }
}

/// The brand string CPUID leaves 0x80000002-4 spell out.
#[cfg(all(not(windows), not(target_vendor = "apple"), target_arch = "x86_64"))]
fn x86_brand() -> Option<String> {
    use std::arch::x86_64::__cpuid;
    // SAFETY: CPUID is on every x86-64 CPU.
    let top = unsafe { __cpuid(0x8000_0000) }.eax;
    if top < 0x8000_0004 {
        return None;
    }
    let mut bytes = Vec::with_capacity(48);
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        // SAFETY: as above, and the leaf is within the range just read.
        let r = unsafe { __cpuid(leaf) };
        for word in [r.eax, r.ebx, r.ecx, r.edx] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    let name = String::from_utf8_lossy(&bytes);
    let name = name.trim_matches(char::from(0)).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};

    const VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    /// "Windows 11 Pro 24H2 (build 26100.4061)". The product name in the
    /// registry still says Windows 10 on Windows 11, so the build number
    /// decides which it is.
    pub(super) fn describe() -> Option<String> {
        let build: u32 = registry(VERSION_KEY, "CurrentBuildNumber")?.parse().ok()?;
        let product = registry(VERSION_KEY, "ProductName").unwrap_or_default();
        let product = if build >= 22000 {
            product.replace("Windows 10", "Windows 11")
        } else {
            product
        };
        let display = registry(VERSION_KEY, "DisplayVersion").unwrap_or_default();
        let revision =
            registry_dword(VERSION_KEY, "UBR").map_or(String::new(), |ubr| format!(".{ubr}"));
        Some(format!("{product} {display} (build {build}{revision})").replace("  ", " "))
    }

    /// A string value under HKEY_LOCAL_MACHINE.
    pub(super) fn registry(key: &str, value: &str) -> Option<String> {
        let key = wide(key);
        let value = wide(value);
        let mut buf = [0u16; 256];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: the buffers are NUL-terminated and `size` is the
        // output buffer's length in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..len]);
        let text = text.trim_matches(char::from(0)).trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// A number value under HKEY_LOCAL_MACHINE.
    fn registry_dword(key: &str, value: &str) -> Option<u32> {
        use windows_sys::Win32::System::Registry::RRF_RT_REG_DWORD;
        let key = wide(key);
        let value = wide(value);
        let mut out = 0u32;
        let mut size = 4u32;
        // SAFETY: as in `registry`, with a four-byte output.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut out as *mut u32).cast(),
                &mut size,
            )
        };
        (status == 0).then_some(out)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(target_vendor = "apple")]
mod os {
    /// "macOS 15.6.1 · Mac15,6", or "iOS 26.0 · iPhone17,1".
    pub(super) fn describe() -> Option<String> {
        let version = sysctl("kern.osproductversion")?;
        let name = if cfg!(target_os = "ios") {
            "iOS"
        } else {
            "macOS"
        };
        let model = sysctl(if cfg!(target_os = "ios") {
            "hw.machine"
        } else {
            "hw.model"
        });
        Some(match model {
            Some(model) => format!("{name} {version} · {model}"),
            None => format!("{name} {version}"),
        })
    }

    /// A string the kernel answers by name.
    pub(super) fn sysctl(name: &str) -> Option<String> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut buf = [0u8; 256];
        let mut len = buf.len();
        // SAFETY: `len` is the buffer's length, and sysctlbyname writes no
        // more than that.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return None;
        }
        let text = std::ffi::CStr::from_bytes_until_nul(&buf[..len.min(buf.len())])
            .ok()?
            .to_str()
            .ok()?
            .trim();
        (!text.is_empty()).then(|| text.to_owned())
    }
}

#[cfg(target_os = "android")]
mod os {
    /// "Android 11 (API 30) · Google Pixel 4a".
    pub(super) fn describe() -> Option<String> {
        let release = property("ro.build.version.release")?;
        let api = property("ro.build.version.sdk").unwrap_or_default();
        let maker = property("ro.product.manufacturer").unwrap_or_default();
        let model = property("ro.product.model").unwrap_or_default();
        Some(
            format!("Android {release} (API {api}) · {maker} {model}")
                .trim()
                .to_owned(),
        )
    }

    fn property(name: &str) -> Option<String> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut buf = [0u8; 92]; // PROP_VALUE_MAX
        // SAFETY: the buffer is PROP_VALUE_MAX bytes, all a value can be.
        let len = unsafe { libc::__system_property_get(name.as_ptr(), buf.as_mut_ptr().cast()) };
        if len <= 0 {
            return None;
        }
        let text = String::from_utf8_lossy(&buf[..len as usize])
            .trim()
            .to_owned();
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
mod os {
    /// "Linux Mint 22.3 · kernel 7.0.0-10-generic".
    pub(super) fn describe() -> Option<String> {
        let release = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                    .map(|name| name.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| "Linux".to_owned());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok();
        Some(match kernel {
            Some(kernel) => format!("{release} · kernel {}", kernel.trim()),
            None => release,
        })
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub(super) fn describe() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_say_something() {
        assert!(build().starts_with(&format!("Concat {}", env!("CARGO_PKG_VERSION"))));
        assert!(!system().is_empty());
        assert!(cpu().contains("threads"));
    }
}
