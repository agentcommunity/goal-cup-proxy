//! The `PATH` walk a harness program is resolved with, and the sweep of the probe files an
//! older `goalcup doctor` could leave behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The name the V1 writable-directory check gave the file it created to ask its question,
/// and the name [`sweep_stale_probes`] looks for to clean one up.
const PROBE_FILE_PREFIX: &str = ".goalcup-preflight-";

/// Remove every `.goalcup-preflight-<pid>` left in `dir`.
///
/// The V1 `required_paths` check (removed by OS-29) created one and removed it in the next
/// statement, so the only ones that survive are from a process killed in between — and a
/// participant who then finds files with this tracker's name on them in their configuration
/// directory is owed their removal rather than an explanation. `goalcup doctor` sweeps before
/// it checks anything. Best effort throughout.
pub(crate) fn sweep_stale_probes(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(PROBE_FILE_PREFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `binary` resolved against the `PATH` in `env`, by walking it **in this process**.
///
/// Not by shelling out to `which`, which would answer out of a login shell's environment —
/// a different `PATH`, possibly a different harness — and so would disagree with the launch
/// this check is predicting.
///
/// The split is [`std::env::split_paths`] and not `split(':')`: it is the platform's own rule
/// for the variable this build is reading, and a hand-rolled separator is a second answer to
/// a question the standard library already answers. A path component that is empty is skipped
/// rather than read as the working directory, which is the participant's repository during a
/// run.
///
/// A `binary` that already contains a separator is taken as the path it is, so a connector
/// naming an absolute program is not looked up under it.
pub(crate) fn resolve_on_path(binary: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    if binary.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(binary);
        return is_executable_file(&path).then_some(path);
    }
    let path = env.get("PATH")?;
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable_file(candidate))
}

/// A regular file this process could execute. On a platform without Unix modes, existing as
/// a file is the whole of the question this build can ask.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
