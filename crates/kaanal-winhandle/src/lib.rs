//! Keeps a spawned daemon from inheriting the caller's stdio pipes on Windows (RULES R1.46).
//!
//! Rust's `Command` always spawns with `bInheritHandles = TRUE`, so a daemon started by
//! `kaanal up -d` would hold the pipe behind `$(kaanal up -d)` or a CI step open for ever. The
//! fix is one Win32 call, `SetHandleInformation`, which needs `unsafe`; the workspace forbids
//! `unsafe` everywhere else, so it lives alone in this crate. On other platforms the function
//! just runs the closure.
//!
//! Only the three std handles are made non-inheritable; a `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
//! would cover every handle, but needs far more `unsafe` (out of scope under R1.46).
//!
//! The lint table in this crate's `Cargo.toml` is a copy of the workspace one with only
//! `unsafe_code` different (`deny`, lifted by one `expect` on the Windows module): keep it in sync
//! with `[workspace.lints]`.

#![warn(missing_docs)]

/// Runs `f` with this process's stdin, stdout and stderr handles marked non-inheritable, then
/// restores their flags (also if `f` panics). Handles that are missing, invalid or not
/// inheritable are left alone; any failure is ignored (best effort). A child spawned inside `f`
/// still gets its own stdio through explicit `Stdio` values, which std duplicates for it.
///
/// Not safe to call from two threads at once (the flags are process-wide and the second call
/// would restore the first one's cleared state); the only caller is single-threaded.
pub fn without_inheritable_stdio<T>(f: impl FnOnce() -> T) -> T {
    imp::without_inheritable_stdio(f)
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "R1.46: the one SetHandleInformation call")]
mod imp {
    use windows_sys::Win32::Foundation::GetHandleInformation;
    use windows_sys::Win32::Foundation::{
        HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    /// Handles whose inherit flag we cleared; set again on drop.
    struct Restore(Vec<HANDLE>);

    impl Drop for Restore {
        fn drop(&mut self) {
            for &h in &self.0 {
                // SAFETY: `h` was a valid std handle when recorded and `f` does not close std
                // handles; if it did, the value could be reused by another handle and we would
                // set the flag on that one, which is harmless (only the inherit flag is set).
                unsafe {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
                }
            }
        }
    }

    pub fn without_inheritable_stdio<T>(f: impl FnOnce() -> T) -> T {
        let mut cleared = Vec::new();
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: GetStdHandle has no preconditions; it returns a handle or null/invalid.
            let h = unsafe { GetStdHandle(which) };
            if h.is_null() || h == INVALID_HANDLE_VALUE || cleared.contains(&h) {
                continue;
            }
            let mut flags = 0u32;
            // SAFETY: `h` is not null or invalid and `flags` is a valid out pointer.
            let known = unsafe { GetHandleInformation(h, &raw mut flags) } != 0;
            if known && flags & HANDLE_FLAG_INHERIT != 0 {
                // SAFETY: as above; only the inherit flag is changed.
                if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) } != 0 {
                    cleared.push(h);
                }
            }
        }
        let _restore = Restore(cleared);
        f()
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn without_inheritable_stdio<T>(f: impl FnOnce() -> T) -> T {
        f()
    }
}
