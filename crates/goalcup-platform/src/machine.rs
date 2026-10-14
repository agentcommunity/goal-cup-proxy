//! Facts about the machine and small host helpers.
//!
//! Everything here is gathered with the standard library and `libc` alone. The facts name no
//! host, user, serial number, address or other identity.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::awake::find_on_path;

/// What a run can say about the machine it ran on. Every field is optional: a fact that could
/// not be read is left out rather than guessed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MachineFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_free_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wsl: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ac_power: Option<bool>,
}

/// Gather the machine facts; `artifact_root` picks the volume whose space is reported.
pub fn facts(artifact_root: &Path) -> MachineFacts {
    let disk = disk_space(artifact_root);
    MachineFacts {
        os: Some(std::env::consts::OS.to_string()),
        os_version: os_version(),
        arch: Some(std::env::consts::ARCH.to_string()),
        cpu_count: std::thread::available_parallelism()
            .ok()
            .map(|n| n.get() as u64),
        memory_bytes: memory_bytes(),
        disk_total_bytes: disk.map(|d| d.0),
        disk_free_bytes: disk.map(|d| d.1),
        wsl: Some(is_wsl()),
        container: in_container(),
        ac_power: on_ac_power(),
    }
}

/// Whether this is Linux running under the Windows Subsystem for Linux.
pub fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").is_ok_and(|s| is_wsl_from(&s))
}

/// The WSL test over a kernel release string.
#[doc(hidden)]
pub fn is_wsl_from(osrelease: &str) -> bool {
    osrelease.to_ascii_lowercase().contains("microsoft")
}

/// Whether the machine runs on mains power; `None` when that cannot be told (a desktop with
/// no battery, or no readable source).
pub fn on_ac_power() -> Option<bool> {
    if cfg!(target_os = "macos") {
        let out = Command::new("pmset")
            .args(["-g", "batt"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        ac_from_pmset(&String::from_utf8_lossy(&out.stdout))
    } else {
        ac_from_sysfs(Path::new("/sys/class/power_supply"))
    }
}

/// The power source named by the first line of `pmset -g batt`.
#[doc(hidden)]
pub fn ac_from_pmset(text: &str) -> Option<bool> {
    if text.contains("'AC Power'") {
        Some(true)
    } else if text.contains("'Battery Power'") {
        Some(false)
    } else {
        None
    }
}

fn ac_from_sysfs(dir: &Path) -> Option<bool> {
    let mut answer = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = std::fs::read_to_string(path.join("type")) else {
            continue;
        };
        if kind.trim() != "Mains" {
            continue;
        }
        let Ok(online) = std::fs::read_to_string(path.join("online")) else {
            continue;
        };
        match online.trim() {
            "1" => return Some(true),
            "0" => answer = Some(false),
            _ => {}
        }
    }
    answer
}

/// Whether `p` lives on a Windows drive mounted into WSL (`/mnt/<letter>/...`).
pub fn on_windows_drive(p: &Path) -> bool {
    on_windows_drive_with(p, is_wsl())
}

/// [`on_windows_drive`] with the WSL answer supplied.
#[doc(hidden)]
pub fn on_windows_drive_with(p: &Path, wsl: bool) -> bool {
    if !wsl {
        return false;
    }
    let mut parts = p.components();
    if parts.next() != Some(Component::RootDir) {
        return false;
    }
    if parts.next().map(Component::as_os_str) != Some("mnt".as_ref()) {
        return false;
    }
    match parts.next() {
        Some(Component::Normal(drive)) => {
            let b = drive.as_bytes();
            b.len() == 1 && b[0].is_ascii_alphabetic()
        }
        _ => false,
    }
}

/// Open `url` in the user's browser. Returns whether the opener started; it is never waited on.
pub fn open_url(url: &str) -> bool {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if let Some(wslview) = find_on_path("wslview") {
        let mut c = Command::new(wslview);
        c.arg(url);
        c
    } else if is_wsl() && find_on_path("rundll32.exe").is_some() {
        // Not `cmd.exe /c start`: cmd would interpret `&`, `|`, `^` and `%` inside the URL.
        let mut c = Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => {
            // Reap the opener off-thread so it never lingers as a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// Whether the content of `/run/systemd/container` names a container. systemd also writes
/// `wsl` there on WSL2, which is a virtual machine, not a container.
#[doc(hidden)]
pub fn container_from_systemd(content: &str) -> bool {
    let kind = content.trim();
    !kind.is_empty() && !kind.eq_ignore_ascii_case("wsl")
}

/// `Some(true)` when a container marker is found. Absence proves nothing (cgroup v2 hides the
/// answer), so the fact is left out rather than reported as `false`.
fn in_container() -> Option<bool> {
    let marked = ["/.dockerenv", "/run/.containerenv"]
        .iter()
        .any(|p| Path::new(p).exists())
        || std::fs::read_to_string("/run/systemd/container")
            .is_ok_and(|s| container_from_systemd(&s));
    let cgroup = std::fs::read_to_string("/proc/1/cgroup").is_ok_and(|s| {
        ["docker", "kubepods", "containerd", "libpod"]
            .iter()
            .any(|marker| s.contains(marker))
    });
    (marked || cgroup).then_some(true)
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "macos") {
        sysctl_string("kern.osproductversion")
    } else {
        let text = std::fs::read_to_string("/etc/os-release").ok()?;
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').to_string())
                .filter(|v| !v.is_empty())
        };
        match (field("ID"), field("VERSION_ID")) {
            (Some(id), Some(ver)) => Some(format!("{id} {ver}")),
            (None, Some(ver)) => Some(ver),
            _ => None,
        }
    }
}

fn memory_bytes() -> Option<u64> {
    if cfg!(target_os = "macos") {
        sysctl_u64("hw.memsize")
    } else {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        kib.checked_mul(1024)
    }
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    // SAFETY: a null buffer asks only for the value's size, written to `len`.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` is valid for `len` bytes, which the call updates to the bytes written.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    String::from_utf8(buf).ok().filter(|s| !s.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn sysctl_string(_name: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    let name = CString::new(name).ok()?;
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>() as libc::size_t;
    // SAFETY: `value` is valid for `len` bytes, the size of a `u64`.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(value)
}

#[cfg(not(target_os = "macos"))]
fn sysctl_u64(_name: &str) -> Option<u64> {
    None
}

/// Total and available bytes of the volume holding `path`.
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: an all-zero `statvfs` is a valid value for the call to overwrite.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path and `st` is a valid out pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    // The field widths differ per target; widening is a no-op on some and needed on others.
    #[allow(clippy::useless_conversion)]
    let unit = u64::from(st.f_frsize);
    let total = (st.f_blocks as u64).checked_mul(unit)?;
    let free = (st.f_bavail as u64).checked_mul(unit)?;
    Some((total, free))
}
