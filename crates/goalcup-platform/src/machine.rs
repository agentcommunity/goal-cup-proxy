//! Facts about the machine and small host helpers.
//!
//! Everything here is gathered with the standard library and `libc` alone. The facts name no
//! host, user, serial number, address or other identity.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

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
    /// True only on WSL2 (WSL1 cannot run the tracker, so a run never reports it).
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
        wsl: Some(wsl_version() == Some(2)),
        container: in_container(),
        ac_power: on_ac_power(),
    }
}

/// Whether this is Linux running under the Windows Subsystem for Linux, of either version.
pub fn is_wsl() -> bool {
    wsl_version().is_some()
}

/// The WSL test over a kernel release string.
#[doc(hidden)]
pub fn is_wsl_from(osrelease: &str) -> bool {
    wsl_version_from(osrelease, "").is_some()
}

/// Which Windows Subsystem for Linux this is: `Some(1)`, `Some(2)`, or `None` on a machine that
/// is not WSL. Read from `/proc/sys/kernel/osrelease` and `/proc/version`.
pub fn wsl_version() -> Option<u8> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    wsl_version_from(&read("/proc/sys/kernel/osrelease"), &read("/proc/version"))
}

/// The WSL version named by the kernel release (`/proc/sys/kernel/osrelease`) and the kernel
/// version line (`/proc/version`); either may be empty.
///
/// Only the kernel *release* is read, in the release file and in the `Linux version <release>`
/// part of the version line: the rest of that line names whoever built the kernel, and an address
/// such as `builder@microsoft.com` is not WSL. Both versions carry `-microsoft` in the release.
/// WSL2 kernels say `microsoft-standard-WSL2` (the earliest 4.19 builds say `microsoft-standard`);
/// WSL1's kernel is the fixed `4.4.0-<build>-Microsoft`. A Microsoft kernel that is neither is a
/// custom WSL2 kernel, since WSL1 has no other release.
#[doc(hidden)]
pub fn wsl_version_from(osrelease: &str, version: &str) -> Option<u8> {
    let releases = [
        osrelease.trim().to_ascii_lowercase(),
        release_of_version_line(version),
    ];
    let named = |r: &String| r.contains("-microsoft") || r.contains("wsl2");
    if !releases.iter().any(named) {
        return None;
    }
    if releases
        .iter()
        .any(|r| r.contains("wsl2") || r.contains("microsoft-standard"))
    {
        return Some(2);
    }
    if releases.iter().any(|r| r.starts_with("4.4.")) {
        return Some(1);
    }
    Some(2)
}

/// The release token of a `/proc/version` line (`Linux version <release> (...)`), lowercased;
/// empty when the line is not in that form.
fn release_of_version_line(line: &str) -> String {
    let line = line.trim().to_ascii_lowercase();
    line.strip_prefix("linux version ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("")
        .to_string()
}

/// Whether a filesystem type name (the type column of `/proc/self/mountinfo`) is a Windows-host
/// share: the Windows drives (`drvfs`, or `9p` on current WSL2 kernels, or `virtiofs` where WSL
/// is set up with it) and network shares (`cifs`, `smb3`) and NTFS volumes.
#[doc(hidden)]
pub fn windows_host_fstype(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "9p" | "drvfs" | "virtiofs" | "cifs" | "smb3" | "smbfs" | "ntfs" | "ntfs3" | "fuse.ntfs-3g"
    )
}

/// The filesystem type name `statfs` reports as `f_type`, for the magic numbers that name a
/// Windows-host share: `V9FS_MAGIC` (the 9p mount WSL2 uses for a Windows drive), `CIFS_MAGIC`,
/// `SMB2_MAGIC` and `NTFS_SB_MAGIC`.
#[doc(hidden)]
pub fn windows_host_magic(f_type: u64) -> Option<&'static str> {
    match f_type {
        0x0102_1997 => Some("9p"),
        0xFF53_4D42 => Some("cifs"),
        0xFE53_4D42 => Some("smb3"),
        0x5346_544E => Some("ntfs"),
        _ => None,
    }
}

/// The filesystem type of the mount that holds `path`, from the text of
/// `/proc/self/mountinfo`: the mount whose mount point is the longest prefix of `path` (the later
/// line, when two share one, because it is mounted on top). `None` when no line covers `path`.
#[doc(hidden)]
pub fn mountinfo_fstype(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(dash) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        if dash < 6 {
            continue;
        }
        let Some(fstype) = fields.get(dash + 1) else {
            continue;
        };
        let point = PathBuf::from(unescape_mountinfo(fields[4]));
        if !path.starts_with(&point) {
            continue;
        }
        let len = point.components().count();
        if best.as_ref().is_none_or(|(l, _)| len >= *l) {
            best = Some((len, (*fstype).to_string()));
        }
    }
    best.map(|(_, fstype)| fstype)
}

/// `/proc/self/mountinfo` writes a space, tab, newline or backslash in a path as `\040`, `\011`,
/// `\012` or `\134`.
fn unescape_mountinfo(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 4 <= bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 4]).ok())
            .flatten()
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The nearest part of `path` that exists, by its real path: a relative path is joined onto the
/// current directory first, and a link met on the way is followed even when it is dangling (a
/// link to a folder not made yet), so a link cannot hide where a new folder would land.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    nearest_existing_from(&absolute, 0)
}

fn nearest_existing_from(path: &Path, links_followed: u32) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            return Some(real);
        }
        if links_followed < 40
            && let Ok(target) = std::fs::read_link(ancestor)
        {
            let target = ancestor.parent()?.join(target);
            return nearest_existing_from(&target, links_followed + 1);
        }
    }
    None
}

/// [`windows_host_filesystem`] over the text of `/proc/self/mountinfo` given, and the mount table
/// alone (no `statfs`): the name of the Windows-host filesystem type holding `path`, or `None`.
#[doc(hidden)]
pub fn windows_host_filesystem_with(path: &Path, mountinfo: &str) -> Option<String> {
    let existing = nearest_existing(path)?;
    mountinfo_fstype(mountinfo, &existing).filter(|name| windows_host_fstype(name))
}

/// The name of the Windows-host filesystem `path` is on, or `None` when it is on a Linux one (or
/// the filesystem cannot be told). Linux only; elsewhere always `None`.
///
/// `path` need not exist yet: the nearest existing ancestor is looked at, by its real path
/// (a relative path is taken from the current directory, and a link, even a dangling one, is
/// followed), so a link cannot hide a Windows drive. Two readings are combined, because neither is
/// enough alone: `statfs` gives the filesystem's magic number, which names 9p (the mount WSL2
/// makes of a Windows drive, which `statfs` cannot tell from any other 9p) but sees a FUSE mount
/// only as FUSE; `/proc/self/mountinfo` gives the type name (`drvfs`, `9p` or `virtiofs`) of the
/// mount holding the path.
pub fn windows_host_filesystem(path: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let existing = nearest_existing(path)?;
        let by_name = std::fs::read_to_string("/proc/self/mountinfo")
            .ok()
            .and_then(|text| windows_host_filesystem_with(&existing, &text));
        let by_magic = statfs_type(&existing).and_then(windows_host_magic);
        by_name.or_else(|| by_magic.map(str::to_string))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "linux")]
fn statfs_type(path: &Path) -> Option<u64> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c` is NUL-terminated and `stat` is a writable statfs for the call.
    if unsafe { libc::statfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs returned 0, so it filled the struct.
    let stat = unsafe { stat.assume_init() };
    // `f_type` is signed or unsigned and 32 or 64 bits wide by target; the magic numbers fit 32.
    Some(u64::from(stat.f_type as u32))
}

/// The refusal for a machine that WSL cannot run the tracker on.
const WSL1_REFUSAL: &str = "WSL1 cannot run Goal Cup: convert this distribution to WSL2 with \
                            `wsl --set-version <distro> 2`";

/// The WSL checks over named folders: `Ok` when `version` is not WSL, or is WSL2 and no folder
/// is on a Windows-host filesystem (`filesystem_of` names the filesystem a folder is on); an
/// `Err` of one or two sentences otherwise (WSL1 is refused whatever the folders are). Each
/// folder is `(what it is, path)`.
#[doc(hidden)]
pub fn wsl_check_with(
    version: Option<u8>,
    folders: &[(&str, &Path)],
    filesystem_of: &dyn Fn(&Path) -> Option<String>,
) -> Result<(), String> {
    match version {
        None => Ok(()),
        Some(1) => Err(WSL1_REFUSAL.to_string()),
        Some(_) => {
            let refused: Vec<String> = folders
                .iter()
                .filter_map(|(what, path)| {
                    let name = filesystem_of(path).filter(|name| windows_host_fstype(name))?;
                    Some(format!(
                        "the {what} {} is on a Windows-host filesystem ({name}), which WSL2 \
                         reads slowly and cannot keep file modes on; use a folder in the Linux \
                         home (for example under ~)",
                        path.display()
                    ))
                })
                .collect();
            if refused.is_empty() {
                Ok(())
            } else {
                Err(refused.join(". "))
            }
        }
    }
}

/// [`wsl_check_with`] on this machine: its WSL version, and the filesystems it reads.
pub fn wsl_check(folders: &[(&str, &Path)]) -> Result<(), String> {
    wsl_check_with(wsl_version(), folders, &windows_host_filesystem)
}

/// Whether the machine runs on mains power; `None` when that cannot be told (a desktop with
/// no battery, or no readable source).
pub fn on_ac_power() -> Option<bool> {
    if cfg!(target_os = "macos") {
        let out = Command::new("/usr/bin/pmset")
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

/// Whether `url` may be handed to an opener: it must start with `https://`, and hold no
/// whitespace or control characters. That also keeps an argument beginning with `-` away from
/// the opener.
#[doc(hidden)]
pub fn url_allowed(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty())
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Open `url` in the user's browser. Only `https://` URLs are accepted (see [`url_allowed`]);
/// anything else returns false without starting a process. Otherwise returns whether the opener
/// started; it is never waited on.
pub fn open_url(url: &str) -> bool {
    if !url_allowed(url) {
        return false;
    }
    let (program, args) = opener(cfg!(target_os = "macos"), is_wsl(), &find_on_path);
    let mut command = Command::new(program);
    command.args(args).arg(url);
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

/// The program that opens a URL on this machine, and the arguments that go before the URL:
/// `/usr/bin/open` on macOS; else `wslview` when it is on `PATH`; else, on WSL, Windows's
/// `rundll32.exe url.dll,FileProtocolHandler` when it is on `PATH` (not `cmd.exe /c start`:
/// cmd would interpret `&`, `|`, `^` and `%` inside the URL); else `xdg-open`. `on_path`
/// finds a program on `PATH`.
fn opener(
    macos: bool,
    wsl: bool,
    on_path: &dyn Fn(&str) -> Option<PathBuf>,
) -> (PathBuf, &'static [&'static str]) {
    if macos {
        (PathBuf::from("/usr/bin/open"), &[])
    } else if let Some(wslview) = on_path("wslview") {
        (wslview, &[])
    } else if wsl && on_path("rundll32.exe").is_some() {
        (
            PathBuf::from("rundll32.exe"),
            &["url.dll,FileProtocolHandler"],
        )
    } else {
        (PathBuf::from("xdg-open"), &[])
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

/// The first executable called `name` on `PATH`.
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

pub(crate) fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One machine in the opener table: what it is, what is on its `PATH`, and the opener it
    /// must get, with the arguments before the URL.
    struct Machine {
        name: &'static str,
        macos: bool,
        wsl: bool,
        on_path: &'static [&'static str],
        program: &'static str,
        before_url: &'static [&'static str],
    }

    /// Each machine gets the opener it can run, with the arguments that go before the URL
    /// (`open_url` appends the URL after them) (T2-F14).
    #[test]
    fn the_browser_opener_is_chosen_by_platform_and_path() {
        let machine = |name, macos, wsl, on_path, program, before_url| Machine {
            name,
            macos,
            wsl,
            on_path,
            program,
            before_url,
        };
        let nothing: &[&str] = &[];
        let rows = [
            machine(
                "macOS",
                true,
                false,
                &["wslview", "xdg-open"],
                "/usr/bin/open",
                nothing,
            ),
            machine(
                "WSL, wslview",
                false,
                true,
                &["wslview", "rundll32.exe"],
                "/bin/wslview",
                nothing,
            ),
            machine(
                "WSL, rundll32.exe only",
                false,
                true,
                &["rundll32.exe"],
                "rundll32.exe",
                &["url.dll,FileProtocolHandler"],
            ),
            machine("WSL, neither", false, true, &[], "xdg-open", nothing),
            machine("Linux", false, false, &[], "xdg-open", nothing),
            machine(
                "Linux, rundll32.exe",
                false,
                false,
                &["rundll32.exe"],
                "xdg-open",
                nothing,
            ),
            machine(
                "Linux, wslview",
                false,
                false,
                &["wslview"],
                "/bin/wslview",
                nothing,
            ),
        ];
        for row in rows {
            let on_path = |wanted: &str| {
                row.on_path
                    .contains(&wanted)
                    .then(|| PathBuf::from("/bin").join(wanted))
            };
            let (program, before_url) = opener(row.macos, row.wsl, &on_path);
            assert_eq!(program, PathBuf::from(row.program), "{}", row.name);
            assert_eq!(before_url, row.before_url, "{}", row.name);
        }
    }
}
