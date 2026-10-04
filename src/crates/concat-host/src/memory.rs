// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! How much memory the machine has, for sizing the caches to it.
//!
//! One question, asked once: the frame cache is a share of it (see
//! `concat_media::ReaderPool::frame_budget`). `None` where the platform
//! will not say, and the caches fall back to the sizes they always had.

/// The machine's physical memory, in bytes.
pub fn total() -> Option<u64> {
    static TOTAL: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *TOTAL.get_or_init(read)
}

/// Linux, Android, macOS and the other Unixes: the count of physical pages
/// times their size, which every one of them answers through `sysconf`.
#[cfg(unix)]
fn read() -> Option<u64> {
    // SAFETY: `sysconf` reads a system constant and has no preconditions;
    // a negative answer means "not known" and is handled below.
    let (pages, size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    (pages > 0 && size > 0).then(|| pages as u64 * size as u64)
}

/// Windows: `GlobalMemoryStatusEx`, the total physical memory it reports.
#[cfg(windows)]
fn read() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: the struct is plain data, zeroed and then given its own
    // length as the call requires; the call only writes into it.
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (GlobalMemoryStatusEx(&mut status) != 0 && status.ullTotalPhys > 0)
            .then_some(status.ullTotalPhys)
    }
}

#[cfg(not(any(unix, windows)))]
fn read() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    /// Every machine the tests run on has some memory, and says how much.
    #[test]
    fn the_machine_says_how_much_memory_it_has() {
        let total = super::total().expect("this platform answers");
        assert!(total >= 512 * 1024 * 1024, "{total} bytes");
    }
}
