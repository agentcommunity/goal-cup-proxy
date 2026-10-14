//! The file a running process was executed from.

use std::path::PathBuf;

/// The path that names the executable image of `pid`, or `None` when the platform cannot say.
///
/// On Linux it is `/proc/<pid>/exe`, which opens the executed file even if its path was since
/// replaced. On macOS it is the path `proc_pidpath` names, so a replacement after `exec` is a
/// stated residual. Elsewhere there is no answer.
#[cfg(target_os = "linux")]
pub fn executable_path(pid: u32) -> Option<PathBuf> {
    Some(PathBuf::from(format!("/proc/{pid}/exe")))
}

/// See the Linux arm.
#[cfg(target_os = "macos")]
pub fn executable_path(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is writable for the length passed, and `proc_pidpath` writes at most
    // that many bytes and returns how many it wrote.
    let written = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast::<libc::c_void>(),
            buffer.len() as u32,
        )
    };
    let written = usize::try_from(written).ok().filter(|n| *n > 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(
        &buffer[..written],
    )))
}

/// See the Linux arm.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn executable_path(_pid: u32) -> Option<PathBuf> {
    None
}
