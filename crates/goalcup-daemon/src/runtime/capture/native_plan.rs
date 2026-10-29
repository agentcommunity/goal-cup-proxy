//! The entry's native-store plan: one source for each schema-declared `[[capture.native]]` row,
//! the roots they resolve to and the isolation they are held to. Chat mode names no session
//! (`identity.source = "none"`), so there is no scope to bind: the stores are read from the
//! entry's create on, and a `since_run_start` source reads only files changed at or after the
//! run's start (D57).

use super::*;
use goalcup_protocol::harness::{NativeSpec, RunHome, glob_matches};

/// Raw native bytes one poll may read: about an eighth of the queue once encoded.
pub(super) const NATIVE_POLL_BYTES: u64 = 4 * MAX_SEGMENT_BYTES as u64;

/// How an entry reads its harness's native stores: one source per schema-declared row, in row
/// order, or nothing for a harness that declares none.
/// `since` is the run's start in Unix seconds, from which a `since_run_start` source reads
/// (D57) and which any other ignores: the request's `native_since`, or else the entry's own
/// create time ([`create_time_floor`]).
pub(super) fn native_plan(
    capture: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
    since: u64,
) -> Result<Vec<NativeSourceSpec>, DaemonError> {
    // A harness that keeps no conversation file of its own declares none: the entry records
    // the wire alone.
    capture
        .native
        .iter()
        .map(|native| native_source(native, run_home, config_home, since))
        .collect()
}

/// One `[[capture.native]]` row, planned.
fn native_source(
    native: &NativeSpec,
    run_home: &Path,
    config_home: &Path,
    since: u64,
) -> Result<NativeSourceSpec, DaemonError> {
    let modified_since = if native.since_run_start {
        Some(
            std::time::UNIX_EPOCH
                .checked_add(Duration::from_secs(since))
                .ok_or_else(|| {
                    DaemonError::Config(format!(
                        "`native_since` {since} is not a time this machine can represent"
                    ))
                })?,
        )
    } else {
        None
    };
    let root = native_root(&native.root, run_home, config_home)?;
    let form = match native.form {
        goalcup_protocol::harness::NativeForm::AppendOnly => NativeStoreForm::AppendOnly,
        goalcup_protocol::harness::NativeForm::Snapshot => NativeStoreForm::Snapshot,
        goalcup_protocol::harness::NativeForm::FinalSnapshot => NativeStoreForm::FinalSnapshot,
        _ => {
            return Err(DaemonError::Config(
                "unsupported schema-1 native form".into(),
            ));
        }
    };
    let hint = native
        .protocol_hint
        .as_deref()
        .map(NativeHint::new)
        .transpose()
        .map_err(|error| DaemonError::Config(format!("native capture: {error}")))?;
    Ok(NativeSourceSpec {
        name: native.name.clone(),
        root,
        pattern: native.pattern.clone(),
        form,
        hint,
        modified_since,
    })
}

/// The floor a `since_run_start` source reads from when the request names no run start: the
/// entry's create time as its registry row records it (`created_at`, whole seconds), so the
/// row a reopen reads holds the same floor the create used (D57).
pub(super) fn create_time_floor(created_at: &str) -> Result<u64, DaemonError> {
    goalcup_protocol::control::parse_z(created_at).ok_or_else(|| {
        DaemonError::Config(format!(
            "the entry's create time {created_at:?} is not RFC 3339"
        ))
    })
}

/// Create the missing directories of every `{config_home}` native root (D57), and of every
/// `{run_home}` root when the run `HOME` is shared (D59), the home itself included: one
/// component at a time from its parent, mode 0700, never following a link. An existing component
/// that is a symlink or not a directory refuses the create. A per-run `{run_home}` root is
/// untouched.
pub(super) fn create_home_roots(
    capture: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
) -> Result<(), DaemonError> {
    use goalcup_platform::tracecap::TraceDir;
    let run_shared = capture.home.run_home == RunHome::Shared;
    for native in &capture.native {
        let (home, rest) = match (
            native.root.strip_prefix("{config_home}"),
            native.root.strip_prefix("{run_home}"),
        ) {
            (Some(rest), _) => (config_home, rest),
            (None, Some(rest)) if run_shared => (run_home, rest),
            _ => continue,
        };
        let refused = |error: goalcup_platform::tracecap::TraceCapError| {
            DaemonError::Config(format!(
                "the native root {:?} under its home cannot be prepared: {error}",
                native.root
            ))
        };
        let (Some(parent), Some(name)) = (
            home.parent(),
            home.file_name().and_then(|name| name.to_str()),
        ) else {
            return Err(DaemonError::Config(
                "the home has no parent directory to create it in".into(),
            ));
        };
        let mut dir = TraceDir::open(parent)
            .and_then(|parent| parent.open_or_create_child_dir(name))
            .map_err(refused)?;
        for component in rest.split('/').filter(|component| !component.is_empty()) {
            dir = dir.open_or_create_child_dir(component).map_err(refused)?;
        }
    }
    Ok(())
}

/// A native source's `root` template, resolved.
fn native_root(root: &str, run_home: &Path, config_home: &Path) -> Result<PathBuf, DaemonError> {
    if root == "{config_home}" {
        Ok(config_home.to_path_buf())
    } else if let Some(rest) = root.strip_prefix("{config_home}/") {
        Ok(config_home.join(rest))
    } else if root == "{run_home}" {
        Ok(run_home.to_path_buf())
    } else if let Some(rest) = root.strip_prefix("{run_home}/") {
        Ok(run_home.join(rest))
    } else {
        Err(DaemonError::Config(format!(
            "native root {root:?} is not rooted in config_home or run_home"
        )))
    }
}

/// The baseline's listing of the harness home (D56): every file but the native roots inside it.
/// The home is the config home, or the run `HOME` when that is shared (D59): the one kept
/// across runs. A native root that is the home itself drops only the files its pattern names;
/// every other file stays listed. `None` when the home cannot be read.
pub(super) fn listed_home(
    capture: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
) -> Option<Vec<goalcup_protocol::seal::SealedFile>> {
    let home = if capture.home.run_home == RunHome::Shared {
        run_home
    } else {
        config_home
    };
    let mut excluded = Vec::new();
    // A row rooted at the home itself names files in it, not a folder. Exclusions are folder
    // prefixes, so those files are dropped by the row's pattern and every other file stays
    // listed (D56).
    let mut home_patterns = Vec::new();
    for native in &capture.native {
        let Ok(root) = native_root(&native.root, run_home, config_home) else {
            continue;
        };
        match root.strip_prefix(home) {
            Ok(rel) if rel.as_os_str().is_empty() => home_patterns.push(native.pattern.as_str()),
            Ok(rel) => excluded.push(rel.to_string_lossy().replace('\\', "/")),
            Err(_) => {}
        }
    }
    let mut files = workspace::list_home(home, &excluded)?;
    files.retain(|file| {
        !home_patterns
            .iter()
            .any(|pattern| pattern_matches(pattern, &file.path))
    });
    Some(files)
}

/// Whether a workspace-relative `path` is named by a native `pattern`: the same number of
/// `/`-separated components, each matching its glob.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(&path)
            .all(|(glob, name)| glob_matches(name, glob))
}

/// A native capture over `sources`, queueing into `sink`.
pub(super) fn native_capture(
    sources: Vec<NativeSourceSpec>,
    sink: Arc<OwnerSink>,
) -> Result<NativeCapture, NativeCaptureError> {
    NativeCapture::new(
        NativeCaptureConfig {
            sources,
            limits: NativeCaptureLimits {
                poll_bytes: NATIVE_POLL_BYTES,
                ..NativeCaptureLimits::default()
            },
        },
        sink,
        Box::new(SystemNativeClock::new()),
    )
}

/// Resolve an isolated root before any artifact creation or source read. Missing trailing
/// directories are allowed, but parent traversal, the participant's `HOME` itself and every
/// loaded connector's `real_dirs` (the harnesses' real homes, as data) are refused.
pub(super) fn isolated_config_home(
    path: &Path,
    declared_real_dirs: &[String],
) -> Result<PathBuf, DaemonError> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        DaemonError::Refused("config_home requires a participant HOME to exclude".into())
    })?;
    isolated_config_home_for(path, declared_real_dirs, &home)
}

/// A shared run `HOME` (D59) persists across runs like a shared config home, so it is resolved
/// and held to the same isolation, and must sit under `<config_dir>/capture-homes`, where the
/// launcher keeps every shared home. A per-run one is the launcher's, fresh inside the run
/// directory, and is returned as given.
pub(super) fn isolated_run_home(
    mode: RunHome,
    run_home: &Path,
    declared_real_dirs: &[String],
    config_dir: &Path,
) -> Result<PathBuf, DaemonError> {
    if mode != RunHome::Shared {
        return Ok(run_home.to_path_buf());
    }
    let refused = |why: String| DaemonError::Refused(format!("the shared run_home: {why}"));
    let resolved = isolated_config_home(run_home, declared_real_dirs)
        .map_err(|error| refused(error.to_string()))?;
    let homes = std::fs::canonicalize(config_dir)
        .map(|dir| dir.join(goalcup_protocol::harness::CAPTURE_HOMES_DIR))
        .map_err(|error| refused(format!("the config_dir cannot be resolved: {error}")))?;
    if resolved.parent() != Some(homes.as_path()) {
        return Err(refused(format!(
            "{} is not a folder directly under {}",
            resolved.display(),
            homes.display()
        )));
    }
    Ok(resolved)
}

pub(super) fn isolated_config_home_for(
    path: &Path,
    declared_real_dirs: &[String],
    home: &Path,
) -> Result<PathBuf, DaemonError> {
    fn resolve(path: &Path) -> Option<PathBuf> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return None;
        }
        let mut ancestor = path;
        let mut tail = Vec::new();
        while std::fs::symlink_metadata(ancestor).is_err() {
            tail.push(ancestor.file_name()?.to_os_string());
            ancestor = ancestor.parent()?;
        }
        let mut resolved = std::fs::canonicalize(ancestor).ok()?;
        for part in tail.into_iter().rev() {
            resolved.push(part);
        }
        Some(resolved)
    }
    let refused = || {
        DaemonError::Refused("config_home must be an absolute isolated directory outside the participant's HOME and every loaded connector's real_dirs".into())
    };
    let root = resolve(path).ok_or_else(refused)?;
    let real_home = resolve(home).ok_or_else(refused)?;
    if root == real_home {
        return Err(refused());
    }
    let mut forbidden = Vec::new();
    for declared in declared_real_dirs {
        let Some(rest) = declared.strip_prefix("{user_home}/") else {
            return Err(refused());
        };
        forbidden.push(home.join(rest));
    }
    for forbidden in forbidden {
        if path.starts_with(&forbidden)
            || root.starts_with(resolve(&forbidden).ok_or_else(refused)?)
        {
            return Err(refused());
        }
    }
    Ok(root)
}
