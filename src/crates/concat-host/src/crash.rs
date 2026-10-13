// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What a native crash leaves behind: one line in the log saying what it
//! was and where.
//!
//! A panic has a hook (see `logs::catch_panics`), but a crash is not a
//! panic. An access violation in a DLL, an instruction the CPU does not
//! have, a segmentation fault in a driver: each ended the program with a
//! status code and nothing else, and the report that came back was "it
//! closes" (#313, #318, #326). The handlers here write one line before the
//! crash goes on to wherever it was going:
//!
//! ```text
//! FATAL [concat-worker-1] crash: SIGSEGV (bad memory access) at 0x10 in libonnxruntime.so.1.28.0+0x1a2b3c
//! FATAL [main] crash: 0xC0000005 EXCEPTION_ACCESS_VIOLATION, reading 0x10, in onnxruntime.dll+0x1a2b3c
//! ```
//!
//! The module and the offset into it are what a shipped build can say: it
//! is stripped, so there are no function names to give, but the offset
//! against the same release's library is an exact place. The line goes to
//! standard error and to this run's log file.
//!
//! Nothing is handled. After the line is written the crash carries on as
//! it would have: on Windows the previous filter and Windows Error
//! Reporting see it, and on Unix the previous handler (Rust's own, which
//! names a stack overflow) is put back and the fault happens again under
//! it, so the process dies of the same signal with the same core dump.
//!
//! A crash handler runs in a broken process, so this one allocates
//! nothing and takes no lock: the line is formatted into a buffer on the
//! stack and written with one system call per destination, to a file
//! handle duplicated for it when the log opened.

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the first crash, so a crash inside the handler, or on a second
/// thread at the same moment, does not write over the first one's line.
static CRASHING: AtomicBool = AtomicBool::new(false);

static INSTALL: Once = Once::new();

/// Installs the handlers. Call as early as a run can - before anything
/// loads a library that might crash. Safe to call more than once.
pub fn catch() {
    INSTALL.call_once(|| {
        // SAFETY: called once, before any crash can reach the handlers,
        // which only read what is set here.
        unsafe { platform::install() }
    });
}

/// Gives the handlers this run's log file to write to as well as standard
/// error. The file is duplicated and the duplicate kept open for the rest
/// of the run, so the handlers never touch the logger's own copy, or its
/// lock.
pub(crate) fn write_to(file: &std::fs::File) {
    if let Ok(file) = file.try_clone() {
        platform::keep(file);
    }
}

/// One line, built on the stack. Formatting into it allocates nothing;
/// what does not fit is cut off.
struct Line {
    buf: [u8; 1024],
    len: usize,
}

impl Line {
    const fn new() -> Self {
        Self {
            buf: [0; 1024],
            len: 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl std::fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let room = self.buf.len() - self.len;
        let n = text.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&text.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// The last component of a path, for a module's name: the folder it was
/// loaded from is the install folder, which says nothing.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(unix)]
mod platform {
    use super::{CRASHING, Line, Ordering, file_name};
    use std::cell::UnsafeCell;
    use std::ffi::{CStr, c_int, c_void};
    use std::fmt::Write as _;
    use std::mem::MaybeUninit;
    use std::os::fd::IntoRawFd;
    use std::sync::atomic::AtomicI32;

    /// The log file's duplicate, or -1 before the log opens.
    static FILE: AtomicI32 = AtomicI32::new(-1);

    /// The signals a crash arrives as, and what each one means.
    const SIGNALS: [(c_int, &str, &str); 5] = [
        (libc::SIGSEGV, "SIGSEGV", "bad memory access"),
        (libc::SIGBUS, "SIGBUS", "bad memory access"),
        (
            libc::SIGILL,
            "SIGILL",
            "an instruction this CPU does not have",
        ),
        (libc::SIGFPE, "SIGFPE", "an arithmetic fault"),
        (libc::SIGABRT, "SIGABRT", "aborted"),
    ];

    /// The handler each signal had before this one, put back when it fires.
    struct Previous(UnsafeCell<[MaybeUninit<libc::sigaction>; SIGNALS.len()]>);
    // SAFETY: written once by `install` before any handler is registered,
    // then only read.
    unsafe impl Sync for Previous {}
    static PREVIOUS: Previous = Previous(UnsafeCell::new(
        [const { MaybeUninit::uninit() }; SIGNALS.len()],
    ));

    pub(super) fn keep(file: std::fs::File) {
        let fd = file.into_raw_fd();
        let old = FILE.swap(fd, Ordering::SeqCst);
        if old >= 0 {
            // SAFETY: `old` was a descriptor this module owned.
            unsafe { libc::close(old) };
        }
    }

    pub(super) unsafe fn install() {
        let previous = PREVIOUS.0.get();
        for (index, (signal, _, _)) in SIGNALS.iter().enumerate() {
            // SAFETY: plain sigaction calls on a zeroed struct; PREVIOUS
            // is written before the handler that reads it is installed.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal
                    as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void)
                    as libc::sighandler_t;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                let slot = (*previous)[index].as_mut_ptr();
                if libc::sigaction(*signal, std::ptr::null(), slot) != 0 {
                    continue;
                }
                libc::sigaction(*signal, &action, std::ptr::null_mut());
            }
        }
    }

    extern "C" fn on_signal(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
        let Some(index) = SIGNALS.iter().position(|(s, _, _)| *s == signal) else {
            return;
        };
        if !CRASHING.swap(true, Ordering::SeqCst) {
            // SAFETY: the kernel hands a valid siginfo and context to an
            // SA_SIGINFO handler.
            let line = unsafe { describe(index, info, context) };
            write(&line);
        }
        // SAFETY: the slot was filled by `install`. Putting the old handler
        // back and returning makes a fault happen again under it.
        unsafe {
            let previous = (*PREVIOUS.0.get())[index].as_ptr();
            libc::sigaction(signal, previous, std::ptr::null_mut());
            // Sent by another process (kill) or by raise(): returning would
            // not happen again, so send it again.
            if (*info).si_code <= 0 {
                libc::raise(signal);
            }
        }
    }

    unsafe fn describe(index: usize, info: *mut libc::siginfo_t, context: *mut c_void) -> Line {
        let (_, name, meaning) = SIGNALS[index];
        let mut line = Line::new();
        let _ = write!(line, "FATAL [");
        thread_name(&mut line);
        let _ = write!(line, "] crash: {name} ({meaning})");
        // SAFETY: as `on_signal` says.
        let address = unsafe { fault_address(info) };
        if matches!(name, "SIGSEGV" | "SIGBUS") {
            let _ = write!(line, " at {address:#x}");
        }
        // SAFETY: as `on_signal` says.
        match unsafe { program_counter(context) } {
            Some(pc) => where_(&mut line, pc),
            None => where_(&mut line, address),
        }
        let _ = writeln!(line);
        line
    }

    /// ` in libfoo.so+0x1234`, or the bare address when no module has it.
    fn where_(line: &mut Line, pc: usize) {
        if pc == 0 {
            return;
        }
        // SAFETY: dladdr only reads the loader's tables; the strings it
        // returns live as long as the module.
        unsafe {
            let mut info: libc::Dl_info = std::mem::zeroed();
            if libc::dladdr(pc as *const c_void, &mut info) != 0 && !info.dli_fname.is_null() {
                let path = CStr::from_ptr(info.dli_fname).to_string_lossy();
                let offset = pc.wrapping_sub(info.dli_fbase as usize);
                let _ = write!(line, " in {}+{offset:#x}", file_name(&path));
                return;
            }
        }
        let _ = write!(line, " at pc {pc:#x}");
    }

    unsafe fn fault_address(info: *mut libc::siginfo_t) -> usize {
        // SAFETY: as `on_signal` says.
        unsafe {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                (*info).si_addr() as usize
            }
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            {
                (*info).si_addr as usize
            }
        }
    }

    /// Where the faulting instruction is, on the targets Concat ships to.
    #[allow(unused_variables)]
    unsafe fn program_counter(context: *mut c_void) -> Option<usize> {
        if context.is_null() {
            return None;
        }
        // SAFETY: an SA_SIGINFO handler's third argument is a ucontext_t.
        unsafe {
            let context = context as *const libc::ucontext_t;
            #[cfg(all(
                any(target_os = "linux", target_os = "android"),
                target_arch = "x86_64"
            ))]
            return Some((*context).uc_mcontext.gregs[libc::REG_RIP as usize] as usize);
            #[cfg(all(
                any(target_os = "linux", target_os = "android"),
                target_arch = "aarch64"
            ))]
            return Some((*context).uc_mcontext.pc as usize);
            #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
            return (*context)
                .uc_mcontext
                .as_ref()
                .map(|m| m.__ss.__pc as usize);
            #[cfg(all(target_vendor = "apple", target_arch = "x86_64"))]
            return (*context)
                .uc_mcontext
                .as_ref()
                .map(|m| m.__ss.__rip as usize);
            #[allow(unreachable_code)]
            None
        }
    }

    /// The thread's name as the kernel has it - what Rust gave the threads
    /// it started, cut to 15 bytes on Linux - or its id.
    fn thread_name(line: &mut Line) {
        let mut name = [0u8; 64];
        // SAFETY: each call writes at most the buffer's length, NUL
        // included.
        let named = unsafe {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                libc::prctl(libc::PR_GET_NAME, name.as_mut_ptr()) == 0
            }
            #[cfg(target_vendor = "apple")]
            {
                libc::pthread_getname_np(libc::pthread_self(), name.as_mut_ptr().cast(), name.len())
                    == 0
            }
            #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
            {
                false
            }
        };
        let name = CStr::from_bytes_until_nul(&name)
            .ok()
            .and_then(|name| name.to_str().ok())
            .filter(|name| named && !name.is_empty());
        match name {
            Some(name) => {
                let _ = write!(line, "{name}");
            }
            None => {
                let _ = write!(line, "?");
            }
        }
    }

    fn write(line: &Line) {
        let bytes = line.bytes();
        // SAFETY: write(2) on descriptors this process holds; a failed
        // write is ignored, there being nowhere left to say so.
        unsafe {
            libc::write(libc::STDERR_FILENO, bytes.as_ptr().cast(), bytes.len());
            let fd = FILE.load(Ordering::SeqCst);
            if fd >= 0 {
                libc::write(fd, bytes.as_ptr().cast(), bytes.len());
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{CRASHING, Line, Ordering, file_name};
    use std::fmt::Write as _;
    use std::os::windows::io::IntoRawHandle;
    use std::sync::atomic::AtomicPtr;
    use windows_sys::Win32::Foundation::{HANDLE, HMODULE};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };

    /// The log file's duplicate, or null before the log opens.
    static FILE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    /// The filter that was there before, which is called after this one.
    static mut PREVIOUS: LPTOP_LEVEL_EXCEPTION_FILTER = None;

    /// What the codes a crash arrives as mean.
    const CODES: [(u32, &str); 9] = [
        (0xC000_0005, "EXCEPTION_ACCESS_VIOLATION"),
        (0xC000_0006, "EXCEPTION_IN_PAGE_ERROR"),
        (
            0xC000_001D,
            "EXCEPTION_ILLEGAL_INSTRUCTION: an instruction this CPU does not have",
        ),
        (0xC000_0094, "EXCEPTION_INT_DIVIDE_BY_ZERO"),
        (0xC000_0096, "EXCEPTION_PRIV_INSTRUCTION"),
        (0xC000_00FD, "EXCEPTION_STACK_OVERFLOW"),
        (0xC000_0409, "STATUS_STACK_BUFFER_OVERRUN"),
        (0xC000_0374, "STATUS_HEAP_CORRUPTION"),
        (0xE06D_7363, "an uncaught C++ exception"),
    ];

    pub(super) fn keep(file: std::fs::File) {
        let handle = file.into_raw_handle();
        let old = FILE.swap(handle, Ordering::SeqCst);
        if !old.is_null() {
            // SAFETY: `old` was a handle this module owned.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(old) };
        }
    }

    pub(super) unsafe fn install() {
        // SAFETY: PREVIOUS is written here, once, before the filter that
        // reads it can run.
        unsafe { PREVIOUS = SetUnhandledExceptionFilter(Some(on_exception)) };
    }

    unsafe extern "system" fn on_exception(pointers: *const EXCEPTION_POINTERS) -> i32 {
        if !CRASHING.swap(true, Ordering::SeqCst) {
            // SAFETY: Windows hands the filter a valid record.
            let line = unsafe { describe(pointers) };
            write(&line);
        }
        // SAFETY: as `install` says.
        match unsafe { PREVIOUS } {
            // SAFETY: the filter that was installed before this one.
            Some(previous) => unsafe { previous(pointers) },
            // EXCEPTION_CONTINUE_SEARCH: on to Windows Error Reporting.
            None => 0,
        }
    }

    unsafe fn describe(pointers: *const EXCEPTION_POINTERS) -> Line {
        let mut line = Line::new();
        let _ = write!(line, "FATAL [");
        // A thread Rust started has its name already, so this is a read;
        // on one a library started it is "?".
        let _ = write!(line, "{}", std::thread::current().name().unwrap_or("?"));
        let _ = write!(line, "] crash");
        // SAFETY: as `on_exception` says.
        let record = unsafe { (*pointers).ExceptionRecord.as_ref() };
        let Some(record) = record else {
            let _ = writeln!(line);
            return line;
        };
        let code = record.ExceptionCode as u32;
        let _ = write!(line, ": {code:#010X}");
        if let Some((_, name)) = CODES.iter().find(|(c, _)| *c == code) {
            let _ = write!(line, " {name}");
        }
        if matches!(code, 0xC000_0005 | 0xC000_0006) && record.NumberParameters >= 2 {
            let what = match record.ExceptionInformation[0] {
                0 => "reading",
                1 => "writing",
                8 => "executing",
                _ => "at",
            };
            let _ = write!(line, ", {what} {:#x},", record.ExceptionInformation[1]);
        }
        where_(&mut line, record.ExceptionAddress as usize);
        let _ = writeln!(line);
        line
    }

    /// ` in foo.dll+0x1234`, or the bare address when no module has it.
    fn where_(line: &mut Line, address: usize) {
        let mut module: HMODULE = std::ptr::null_mut();
        let mut path = [0u16; 260];
        // SAFETY: the flags make GetModuleHandleExW take an address and
        // leave the module's reference count alone; the path is written
        // within the buffer's length.
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address as *const u16,
                &mut module,
            ) != 0
        };
        let len = if found {
            // SAFETY: as above.
            unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) as usize }
        } else {
            0
        };
        if len == 0 {
            let _ = write!(line, " at {address:#x}");
            return;
        }
        // The name, decoded into a buffer on the stack.
        let mut name = Line::new();
        for c in char::decode_utf16(path[..len].iter().copied()) {
            let _ = name.write_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let name = std::str::from_utf8(name.bytes()).unwrap_or("?");
        let offset = address.wrapping_sub(module as usize);
        let _ = write!(line, " in {}+{offset:#x}", file_name(name));
    }

    fn write(line: &Line) {
        let bytes = line.bytes();
        let put = |handle: HANDLE| {
            if handle.is_null() || handle as isize == -1 {
                return;
            }
            let mut written = 0u32;
            // SAFETY: a handle this process holds; a failed write is
            // ignored, there being nowhere left to say so.
            unsafe {
                WriteFile(
                    handle,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                );
            }
        };
        // SAFETY: GetStdHandle only reads the process's parameters.
        put(unsafe { GetStdHandle(STD_ERROR_HANDLE) });
        put(FILE.load(Ordering::SeqCst));
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) fn keep(_: std::fs::File) {}
    pub(super) unsafe fn install() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn a_line_that_does_not_fit_is_cut_off() {
        let mut line = Line::new();
        for _ in 0..200 {
            let _ = write!(line, "0123456789");
        }
        assert_eq!(line.bytes().len(), 1024);
    }

    /// Crashes a copy of this test binary on purpose and reads what it
    /// left on standard error.
    #[test]
    fn a_crash_says_what_it_was_and_where() {
        const CHILD: &str = "CONCAT_CRASH_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            catch();
            // SAFETY: none; this is the crash.
            unsafe { std::ptr::without_provenance_mut::<u8>(0x10).write_volatile(1) };
            unreachable!("the write above should have crashed");
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash::tests::a_crash_says_what_it_was_and_where",
            ])
            .args(["--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "the child did not crash: {stderr}");
        let line = stderr
            .lines()
            .find(|line| line.starts_with("FATAL ["))
            .unwrap_or_else(|| panic!("no crash line in: {stderr}"));
        #[cfg(unix)]
        assert!(
            line.contains("SIGSEGV") || line.contains("SIGBUS"),
            "{line}"
        );
        #[cfg(windows)]
        assert!(
            line.contains("EXCEPTION_ACCESS_VIOLATION, writing 0x10"),
            "{line}"
        );
        assert!(line.contains(" in "), "no module in: {line}");
    }

    #[test]
    fn a_module_is_named_without_its_folder() {
        assert_eq!(
            file_name(r"C:\Program Files\Concat\onnxruntime.dll"),
            "onnxruntime.dll"
        );
        assert_eq!(
            file_name("/usr/lib/libonnxruntime.so.1.28.0"),
            "libonnxruntime.so.1.28.0"
        );
        assert_eq!(file_name("concat"), "concat");
    }
}
