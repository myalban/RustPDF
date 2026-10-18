//! macOS isolation, applied by the worker to itself once PDFium is loaded: the system "pure computation" sandbox
//! profile (Seatbelt) denies file access, network access and starting programs.

use std::ffi::{CStr, CString, c_char, c_int};

unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// `SANDBOX_NAMED`: the profile is one of the system's named profiles.
const SANDBOX_NAMED: u64 = 0x0001;

pub fn lockdown() -> String {
    let profile = CString::new("pure-computation").unwrap_or_default();
    let mut error: *mut c_char = std::ptr::null_mut();
    // SAFETY: documented use of sandbox_init with a named profile; the error string is freed with sandbox_free_error.
    unsafe {
        if sandbox_init(profile.as_ptr(), SANDBOX_NAMED, &mut error) == 0 {
            return "macOS sandbox (pure computation): no files, no network, no other programs".into();
        }
        let msg = if error.is_null() { "unknown error".to_string() } else { CStr::from_ptr(error).to_string_lossy().into_owned() };
        if !error.is_null() {
            sandbox_free_error(error);
        }
        format!("macOS sandbox failed: {msg}")
    }
}
