//! Which harness build a launch ran (D53): the daemon's own observation, recorded in `Launch`.
//!
//! The launch post comes after the parked child was released and `exec`ed, so the registered
//! process is the harness. Two digests are taken, because neither alone names every harness:
//!
//! * the **program** the launcher resolved and registered (`ProcessHandle::program`), the file a
//!   vendor publishes, and for a script or wrapper the only digest that names the harness;
//! * the running **image**: on Linux the file `/proc/<pid>/exe` opens, which is the executed
//!   file even if its path was since replaced; on macOS the path `proc_pidpath` names, so a
//!   replacement between `exec` and this read is a stated residual. For an interpreted harness
//!   it is the interpreter.
//!
//! The image is read only while the pid is still the registered instance, checked before and
//! after the read. A digest that cannot be taken is absent, never guessed; nothing here refuses
//! a launch.

use goalcup_protocol::capture::HarnessBuildV1;
use goalcup_protocol::process::{ProcessHandle, ProcessInstanceStatus, process_instance_status};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The largest file hashed: a harness binary is 100 to 200 MB today.
const MAX_HASHED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Observe the build `handle` names. Slow (it reads whole binaries), so callers hold no lock.
pub(super) fn observe(handle: &ProcessHandle) -> HarnessBuildV1 {
    let program = Path::new(&handle.program);
    let program_sha256 = program
        .is_absolute()
        .then(|| sha256_file(program))
        .flatten();
    HarnessBuildV1 {
        program_sha256,
        image_sha256: image_sha256(handle),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

fn image_sha256(handle: &ProcessHandle) -> Option<String> {
    let same = || {
        process_instance_status(handle.pid, handle.process_start.as_deref())
            == ProcessInstanceStatus::Same
    };
    if !same() {
        return None;
    }
    let digest = sha256_file(&image_path(handle.pid)?)?;
    same().then_some(digest)
}

#[cfg(target_os = "linux")]
fn image_path(pid: u32) -> Option<PathBuf> {
    Some(PathBuf::from(format!("/proc/{pid}/exe")))
}

#[cfg(target_os = "macos")]
fn image_path(pid: u32) -> Option<PathBuf> {
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn image_path(_pid: u32) -> Option<PathBuf> {
    None
}

/// The SHA-256 of a regular file, read to its end, or `None` when it is not one, cannot be
/// read, or is larger than [`MAX_HASHED_BYTES`].
fn sha256_file(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_HASHED_BYTES {
        return None;
    }
    let mut reader = file.take(MAX_HASHED_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_HASHED_BYTES {
            return None;
        }
        hasher.update(&buffer[..n]);
    }
    Some(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(pid: u32, program: &str) -> ProcessHandle {
        ProcessHandle {
            pid,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: goalcup_protocol::process::process_start_identity(pid),
            strategy: goalcup_protocol::manifest::KillStrategy::ProcessGroup,
            grace_seconds: 1,
            launched_at: "2026-10-14T00:00:00Z".into(),
            program: program.into(),
            argv: Vec::new(),
            env_keys: Vec::new(),
        }
    }

    #[test]
    fn a_running_child_has_both_digests_and_a_relative_program_has_none() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let expected = sha256_file(Path::new("/bin/sleep")).unwrap();
        let build = observe(&handle(child.id(), "/bin/sleep"));
        assert_eq!(build.program_sha256.as_deref(), Some(expected.as_str()));
        assert_eq!(build.image_sha256.as_deref(), Some(expected.as_str()));
        assert_eq!(
            build.platform,
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
        );
        let relative = observe(&handle(child.id(), "sleep"));
        assert_eq!(relative.program_sha256, None);
        assert_eq!(relative.image_sha256.as_deref(), Some(expected.as_str()));
        child.kill().unwrap();
        child.wait().unwrap();
        // Gone: no image digest, and no guess.
        assert_eq!(observe(&handle(child.id(), "sleep")).image_sha256, None);
    }

    #[test]
    fn another_instance_on_the_same_pid_gets_no_image_digest() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut stale = handle(child.id(), "sleep");
        stale.process_start = Some("not-this-instance".into());
        assert_eq!(observe(&stale).image_sha256, None);
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
