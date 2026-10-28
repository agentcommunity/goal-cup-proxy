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
//! a launch. Stated residuals: the image is whatever the registered pid runs at the launch
//! post, so a wrapper that `exec`s again (`#!/usr/bin/env node`) may be seen before or after
//! that second `exec`, and a native binary a wrapper spawns as a child is not hashed; on Linux a
//! process that made itself non-dumpable leaves `/proc/<pid>/exe` unreadable to the daemon.

use goalcup_protocol::capture::HarnessBuildV1;
use goalcup_protocol::process::{ProcessHandle, ProcessInstanceStatus, process_instance_status};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// The largest file hashed: a harness binary is 100 to 200 MB today, and a larger file the
/// launcher registered would only keep the launch post waiting.
const MAX_HASHED_BYTES: u64 = 512 * 1024 * 1024;

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
    let digest = sha256_file(&goalcup_platform::process_image::executable_path(
        handle.pid,
    )?)?;
    same().then_some(digest)
}

/// The SHA-256 of a regular file, read to its end, or `None` when it is not one, cannot be
/// read, or is larger than [`MAX_HASHED_BYTES`]. Opened non-blocking, so a path the launcher
/// registered that names a FIFO or a device cannot stall the daemon before the check.
fn sha256_file(path: &Path) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
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

    /// Wait until `pid` runs `program`. A plain `Command::spawn` (std uses `posix_spawn`, a vfork)
    /// returns when the child lets go of this process's memory, which on Linux is a moment before
    /// the kernel installs the new image, so `/proc/<pid>/exe` can still name this test binary.
    /// The daemon never sees that window: the launcher's gated spawn (`launch::spawn_gated` in
    /// goalcup-cli) forks, and returns only after `exec` has closed std's close-on-exec pipe, which
    /// is after the new image is in place, and only then posts the launch.
    fn wait_for_exec(pid: u32, program: &str) {
        let want = std::fs::canonicalize(program).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while goalcup_platform::process_image::executable_path(pid)
            .and_then(|path| std::fs::canonicalize(path).ok())
            != Some(want.clone())
        {
            assert!(
                std::time::Instant::now() < until,
                "pid {pid} did not exec {program}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_running_child_has_both_digests_and_a_relative_program_has_none() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        wait_for_exec(child.id(), "/bin/sleep");
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
    fn a_fifo_or_a_directory_is_no_digest_and_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert_eq!(sha256_file(&fifo), None);
        assert_eq!(sha256_file(dir.path()), None);
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
