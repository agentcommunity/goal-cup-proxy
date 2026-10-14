//! The entry's schema-driven native source and binding state.

use super::*;

/// Raw native bytes one poll may read: about an eighth of the queue once encoded.
pub(super) const NATIVE_POLL_BYTES: u64 = 4 * MAX_SEGMENT_BYTES as u64;

/// How an entry reads its harness's one schema-1 native store.
pub(super) struct NativePlan {
    /// The schema-declared source when its path contains `{id}`.
    pub(super) scoped: Option<NativeSourceSpec>,
    /// Where that source stands before the launch.
    pub(super) initial_binding: NativeBinding,
    /// The schema-declared source when its path has no `{id}`; empty when `scoped` is set.
    pub(super) controlled: Vec<NativeSourceSpec>,
    /// Stores the profile names that this entry does not read; each is marked by a gap at the
    /// launch.
    pub(super) unread_stores: usize,
}

/// `native_since` is the run's start in Unix seconds, which a `since_run_start` source needs
/// (D57) and any other ignores.
pub(super) fn native_plan(
    capture: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
    native_since: Option<u64>,
) -> Result<NativePlan, DaemonError> {
    let native = capture
        .native
        .first()
        .ok_or_else(|| DaemonError::Config("a schema-1 capture has no native source".into()))?;
    let modified_since = match (native.since_run_start, native_since) {
        (false, _) => None,
        (true, Some(secs)) => Some(
            std::time::UNIX_EPOCH
                .checked_add(Duration::from_secs(secs))
                .ok_or_else(|| {
                    DaemonError::Config(format!(
                        "`native_since` {secs} is not a time this machine can represent"
                    ))
                })?,
        ),
        (true, None) => {
            return Err(DaemonError::Config(
                "the profile reads the session files written since the run started, and the \
                 request names no `native_since`: the launcher must send the run's start time"
                    .into(),
            ));
        }
    };
    let root = native_root(&native.root, run_home, config_home)?;
    let form = match native.form {
        goalcup_protocol::harness::NativeForm::AppendOnly => NativeStoreForm::AppendOnly,
        goalcup_protocol::harness::NativeForm::Snapshot => NativeStoreForm::Snapshot,
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
    let source = NativeSourceSpec {
        name: native.name.clone(),
        root,
        pattern: native.pattern.clone(),
        form,
        hint,
        scope_id: None,
        modified_since,
    };
    if native.pattern.contains("{id}") {
        let initial_binding = match capture.identity.binding {
            Some(goalcup_protocol::harness::Binding::AtLaunch) => NativeBinding::AtLaunch,
            Some(goalcup_protocol::harness::Binding::Deferred) => NativeBinding::Deferred,
            None => {
                return Err(DaemonError::Config(
                    "scoped native source has no binding".into(),
                ));
            }
            _ => {
                return Err(DaemonError::Config(
                    "unsupported schema-1 identity binding".into(),
                ));
            }
        };
        Ok(NativePlan {
            scoped: Some(source),
            initial_binding,
            controlled: Vec::new(),
            unread_stores: 0,
        })
    } else {
        Ok(NativePlan {
            scoped: None,
            initial_binding: NativeBinding::NotRequired,
            controlled: vec![source],
            unread_stores: 0,
        })
    }
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

/// The baseline's listing of the config home (D56): every file but the native roots inside it.
/// Empty when a native root is the home itself; `None` when the home cannot be read.
pub(super) fn listed_home(
    capture: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
) -> Option<Vec<goalcup_protocol::seal::SealedFile>> {
    let mut excluded = Vec::new();
    for native in &capture.native {
        let Ok(root) = native_root(&native.root, run_home, config_home) else {
            continue;
        };
        match root.strip_prefix(config_home) {
            Ok(rel) if rel.as_os_str().is_empty() => return Some(Vec::new()),
            Ok(rel) => excluded.push(rel.to_string_lossy().replace('\\', "/")),
            Err(_) => {}
        }
    }
    workspace::list_home(config_home, &excluded)
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

/// Where an entry's native source stands. In memory only: a daemon restart forgets it, and a
/// reopened Hermes entry starts at [`NativeBinding::Deferred`] with no source to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeBinding {
    /// The profile reads no scoped native source, or this is a reopened entry.
    NotRequired,
    /// Due at the launch and not yet dealt with. The launch leaves it as [`Self::Bound`] or
    /// [`Self::Unbound`].
    AtLaunch,
    /// The scope is due at the finish (Hermes). The launch recorded no gap for it.
    Deferred,
    /// A source was handed to the owner, at the launch or at finish stage 4.
    Bound,
    /// No usable scope: the one gap was recorded, at the launch or at finish stage 5. Terminal: a
    /// later finish of an expired one never records it twice.
    Unbound,
}

impl Daemon {
    /// Classify a reopened row only when today's connector bytes match its recorded bytes.
    pub(super) fn recorded_harness_defers_scope(
        &self,
        connector_id: &str,
        recorded_digest: Option<&str>,
    ) -> bool {
        let Some(recorded_digest) = recorded_digest else {
            return false;
        };
        let Some(loaded) = recorded_capture_harness(self.config(), connector_id, recorded_digest)
        else {
            return false;
        };
        matches!(
            loaded.manifest.capture.identity.binding,
            Some(goalcup_protocol::harness::Binding::Deferred)
        )
    }
}

impl CaptureEntry {
    /// The entry's one native source over `source`, scoped to `id`; `None` when `id` is not a
    /// scope identity the collector accepts.
    pub(super) fn native_for(&self, source: &NativeSourceSpec, id: &str) -> Option<NativeCapture> {
        let mut source = source.clone();
        source.scope_id = Some(id.to_string());
        native_capture(vec![source], self.sink.clone()).ok()
    }

    /// Finish stage 4, under the run lock and before the owner stops: a deferred entry given a
    /// scope identity installs its native source, which the owner takes up when it stops and
    /// finishes within the drain share. Anything else leaves the binding as it was, so a finish
    /// that expired before this stage is retried with the same chance, and a source already
    /// installed is never installed twice.
    pub(super) fn install_deferred_scope(&self, scope: Option<&str>, launched: bool) {
        // An owner already told to stop has passed (or will not make) its final take of the
        // pending source: installing one now would be `Bound` for a source nothing finishes, and
        // no gap. The binding stays `Deferred`, and stage 5 records the gap.
        if !launched || self.owner.stop.load(Ordering::Acquire) {
            return;
        }
        let mut binding = lock(&self.native_binding);
        if *binding != NativeBinding::Deferred {
            return;
        }
        if let Some(id) = scope {
            self.bind_scope(&mut binding, id);
        }
    }

    /// Hand the owner a native capture over the entry's scoped source for session `id`, and mark
    /// `binding` [`NativeBinding::Bound`]. `false`, with `binding` unchanged, when the profile has
    /// no scoped source or `id` is not a scope identity the collector accepts. The one place a
    /// source is bound, at the launch or at finish stage 4.
    pub(super) fn bind_scope(&self, binding: &mut NativeBinding, id: &str) -> bool {
        let Some(native) = self
            .scoped_native
            .as_ref()
            .and_then(|source| self.native_for(source, id))
        else {
            return false;
        };
        *lock(&self.owner.pending_native) = Some(native);
        *binding = NativeBinding::Bound;
        true
    }

    /// The launch's part of the native binding. An entry whose scope is due at the launch binds
    /// it from the request, or records its one gap and is [`NativeBinding::Unbound`]. A
    /// [`NativeBinding::Deferred`] entry with no scope in the request owes nothing yet: the finish
    /// carries it (CONTRACTS.md, C6 local finish budget amendment, ¶9).
    pub(super) fn bind_launch_scope(
        &self,
        scope: Option<&str>,
        writer: &mut ChainWriter,
        now: &CaptureTime,
    ) {
        if self.scoped_native.is_none() {
            return;
        }
        let mut binding = lock(&self.native_binding);
        if *binding == NativeBinding::Deferred && scope.is_none() {
            return;
        }
        if scope.is_some_and(|id| self.bind_scope(&mut binding, id)) {
            return;
        }
        writer.streamless_gap(GapReason::SourceChanged, now);
        *binding = NativeBinding::Unbound;
    }

    /// Finish stage 5: whether a launched entry still owes the one deferred-scope gap. Taken
    /// once: the binding becomes terminal either way.
    pub(super) fn take_deferred_gap(&self, launched: bool) -> bool {
        let mut binding = lock(&self.native_binding);
        if launched && *binding == NativeBinding::Deferred {
            *binding = NativeBinding::Unbound;
            return true;
        }
        false
    }
}
