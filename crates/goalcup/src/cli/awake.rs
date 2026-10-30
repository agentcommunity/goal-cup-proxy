//! The sleep lock (PRD-10 section 9). This slice holds the probe only; the lock itself comes with
//! `start`, in this file. The tool facts are 0.7's (`goalcup-platform/src/awake.rs`): macOS
//! `/usr/bin/caffeinate`, Linux `systemd-inhibit` with a running systemd, none on WSL. The macOS
//! branch has not been run in a box.

use std::collections::BTreeMap;
use std::path::Path;

use crate::connector::launch::resolve_program;

/// None when this computer has a sleep inhibitor goalcup can hold, else why not, in a few words.
/// `vars` is the environment whose `PATH` is searched, as for the harness program.
pub fn missing(vars: &BTreeMap<String, String>) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        let found = Path::new("/usr/bin/caffeinate").exists();
        return (!found).then_some("caffeinate is missing");
    }
    if super::folder::wsl_version().is_some() {
        return Some("set Windows sleep to Never");
    }
    let tool = resolve_program("systemd-inhibit", vars).is_absolute();
    let systemd = Path::new("/run/systemd/system").is_dir();
    (!(tool && systemd)).then_some("no systemd-inhibit with a running systemd")
}
