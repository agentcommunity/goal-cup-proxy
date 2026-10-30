//! The sleep lock (PRD-10 section 9). This slice holds the probe only; the lock itself comes with
//! `start`, in this file. The tool facts are 0.7's (`goalcup-platform/src/awake.rs`): macOS
//! `/usr/bin/caffeinate`, Linux `systemd-inhibit` with a running systemd, none on WSL. The macOS
//! branch has not been run in a box.

use std::path::Path;

/// None when this computer has a sleep inhibitor goalcup can hold, else why not, in a few words.
pub fn missing() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        let found = Path::new("/usr/bin/caffeinate").exists();
        return (!found).then_some("caffeinate is missing");
    }
    if super::folder::wsl_version().is_some() {
        return Some("set Windows sleep to Never");
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = std::env::split_paths(&path).filter(|d| d.is_absolute());
    let tool = dirs.any(|d| d.join("systemd-inhibit").is_file());
    let systemd = Path::new("/run/systemd/system").is_dir();
    (!(tool && systemd)).then_some("no systemd-inhibit with a running systemd")
}
