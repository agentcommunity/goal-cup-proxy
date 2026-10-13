//! The calling thread's `errno`, for the `readdir` end-of-directory idiom.
//!
//! `readdir` returns NULL both at the end of a directory and on error, and only `errno`
//! distinguishes them: clear it, call `readdir`, then a NULL return with `get() == 0` is the
//! end and a non-zero value is the error. The three openat walkers (trace capture, source
//! capture and the verifier's trace reader) share this one accessor.
//!
//! Only the targets the crate's native layers are built and tested on are supported. The
//! verifier reaches this module through `goalcup-protocol -> goalcup-platform`, so it compiles
//! only where the platform crate does; the extra BSD and Solaris arms it once carried were dead.

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn location() -> *mut libc::c_int {
    // SAFETY: the platform errno accessor takes no arguments and returns the calling thread's errno location, which is valid for the thread's lifetime.
    unsafe { libc::__error() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn location() -> *mut libc::c_int {
    // SAFETY: the platform errno accessor takes no arguments and returns the calling thread's errno location, which is valid for the thread's lifetime.
    unsafe { libc::__errno_location() }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
compile_error!("directory enumeration needs a target errno accessor");

/// Set the calling thread's `errno` to zero.
pub fn clear() {
    // SAFETY: `location` returns a valid, writable pointer to this thread's errno, and no reference to it outlives the write.
    unsafe { *location() = 0 };
}

/// Read the calling thread's `errno`.
pub fn get() -> i32 {
    // SAFETY: `location` returns a valid pointer to this thread's errno, read immediately on the same thread.
    unsafe { *location() }
}
