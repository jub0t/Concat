// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The machine kept awake while long work runs.
//!
//! An export, a transcription or a model download takes minutes, and the
//! machine went to sleep under it: a phone's screen timed out half a
//! minute in and the export stopped with it (#695), and a laptop left to
//! export slept through the rest. Every job slot ([`crate::jobs`]) holds
//! an [`Awake`] while its job runs; the first one held asks the platform
//! to stay awake, the last one let go lets it rest.
//!
//! A phone keeps its screen on, through a hook its own crate installs
//! ([`install`]), since only it holds the activity. Windows and macOS keep
//! the system awake - the display may still turn off - through a power
//! request and a power assertion, each a handle any thread may take or
//! let go of. Elsewhere nothing is done.

use std::sync::{Mutex, OnceLock};

/// What a platform's own crate does to keep the machine awake (`true`) or
/// let it rest (`false`).
pub type Hook = Box<dyn Fn(bool) + Send + Sync>;

static HOOK: OnceLock<Hook> = OnceLock::new();

/// How many [`Awake`]s are held. Changed under the lock, and the platform
/// told under it too, so a hold and a release on two threads cannot reach
/// it in the wrong order.
static HELD: Mutex<usize> = Mutex::new(0);

/// Installs the platform's own way of staying awake, in place of the
/// built-in one. Once; a second call is ignored.
pub fn install(hook: Hook) {
    let _ = HOOK.set(hook);
}

/// A hold on the machine's wakefulness, let go on drop.
#[must_use = "the machine is kept awake only while the hold is"]
pub struct Awake(());

impl Awake {
    /// Keeps the machine awake until this is dropped.
    pub fn hold() -> Awake {
        let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if *held == 0 {
            stay_awake(true);
        }
        *held += 1;
        Awake(())
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *held = held.saturating_sub(1);
        if *held == 0 {
            stay_awake(false);
        }
    }
}

fn stay_awake(on: bool) {
    match HOOK.get() {
        Some(hook) => hook(on),
        None => system::stay_awake(on),
    }
}

#[cfg(windows)]
mod system {
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
    };
    use windows_sys::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    /// The power request, made on first use and kept: set while work runs,
    /// cleared after. Its address, as a handle is not `Send`.
    static REQUEST: Mutex<Option<usize>> = Mutex::new(None);

    pub fn stay_awake(on: bool) {
        let mut request = REQUEST
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if request.is_none() {
            if !on {
                return;
            }
            let mut reason: Vec<u16> = "Concat is exporting or processing media"
                .encode_utf16()
                .chain([0])
                .collect();
            let context = REASON_CONTEXT {
                Version: 0,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: reason.as_mut_ptr(),
                },
            };
            // SAFETY: `context` and the string it points at outlive the call.
            let made: HANDLE = unsafe { PowerCreateRequest(&context) };
            if made.is_null() || made == INVALID_HANDLE_VALUE {
                log::warn!("could not ask Windows to stay awake");
                return;
            }
            *request = Some(made as usize);
        }
        let Some(handle) = *request else { return };
        let handle = handle as HANDLE;
        // SAFETY: `handle` is the live request made above.
        let done = unsafe {
            if on {
                PowerSetRequest(handle, PowerRequestSystemRequired)
            } else {
                PowerClearRequest(handle, PowerRequestSystemRequired)
            }
        };
        if done == 0 {
            log::warn!("Windows refused the power request ({on})");
            // SAFETY: as above; a request that will not set is closed and
            // made anew next time.
            unsafe { CloseHandle(handle) };
            *request = None;
        }
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::ffi::c_void;
    use std::sync::Mutex;

    type CFStringRef = *const c_void;
    type IOPMAssertionID = u32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            text: *const std::ffi::c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(object: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut IOPMAssertionID,
        ) -> i32;
        fn IOPMAssertionRelease(id: IOPMAssertionID) -> i32;
    }

    /// The assertion while one is held.
    static ASSERTION: Mutex<Option<IOPMAssertionID>> = Mutex::new(None);

    pub fn stay_awake(on: bool) {
        let mut assertion = ASSERTION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match (on, *assertion) {
            (true, None) => {
                // SAFETY: both strings are NUL-terminated literals; the
                // CFStrings made from them are released after the call.
                unsafe {
                    let kind = CFStringCreateWithCString(
                        std::ptr::null(),
                        c"PreventUserIdleSystemSleep".as_ptr(),
                        K_CF_STRING_ENCODING_UTF8,
                    );
                    let name = CFStringCreateWithCString(
                        std::ptr::null(),
                        c"Concat is exporting or processing media".as_ptr(),
                        K_CF_STRING_ENCODING_UTF8,
                    );
                    let mut id: IOPMAssertionID = 0;
                    let status =
                        IOPMAssertionCreateWithName(kind, K_IOPM_ASSERTION_LEVEL_ON, name, &mut id);
                    if !kind.is_null() {
                        CFRelease(kind);
                    }
                    if !name.is_null() {
                        CFRelease(name);
                    }
                    if status == 0 {
                        *assertion = Some(id);
                    } else {
                        log::warn!("could not ask macOS to stay awake ({status})");
                    }
                }
            }
            (false, Some(id)) => {
                // SAFETY: `id` is the assertion made above, released once.
                unsafe { IOPMAssertionRelease(id) };
                *assertion = None;
            }
            _ => {}
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod system {
    pub fn stay_awake(_on: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_counted_until_it_is_let_go() {
        // Other tests' jobs hold too, on other threads: only that this
        // one is counted while it is held can be asserted.
        let hold = Awake::hold();
        assert!(*HELD.lock().unwrap() >= 1);
        drop(hold);
    }
}
