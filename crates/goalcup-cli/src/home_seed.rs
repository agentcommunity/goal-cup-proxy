//! Seed a per-run harness home from the participant's own configuration, so a fresh home is
//! not a first-run one.
//!
//! # The problem this exists for
//!
//! `connectors/claude-code-cli/manifest.toml` pins `CLAUDE_CONFIG_DIR = {harness_home}`, and
//! the daemon mints that directory per entry. Claude Code keeps its first-run state in
//! `<config dir>/.claude.json`, so without seeding every start would meet a theme picker and
//! then "Select login method" — on a machine whose participant has been signed in for months.
//! The first minute of the trace would be onboarding UI, and the run would never begin.
//!
//! # What it does, and what it will never do
//!
//! It **reads** the file the connector's `[home_seed] from` names under the participant's
//! real home, copies the keys `keys` names out of it, and merges them into the `to` path
//! inside the per-run harness home. Nothing here opens a path under `{home}` for writing,
//! and the manifest has no field that could ask it to: D21 is the list of app-wide
//! **writes** the tracker permits, and this is not one of them. The participant is told, on
//! one line, which file was read and which keys were copied.
//!
//! # Merged, never overwritten
//!
//! The destination may already exist — on a resume it holds everything Claude Code wrote
//! during the first half of the run, its project entry and its history among it. So the
//! seeded keys are merged into the document that is there rather than replacing it. A
//! destination that is not a JSON object at all is replaced, because it is not a document
//! anything here can merge into and the alternative is refusing a run over a file the
//! tracker itself created.
//!
//! # Where `{home}` comes from
//!
//! From the caller, which resolves it out of the environment it was handed. It is a
//! parameter and not a `std::env::var` for the reason everything else in this crate takes
//! its environment as an argument: a rule nothing can call with a made-up home is a rule
//! nothing tests, and this one reads a file that holds an account.

use crate::CliError;
use goalcup_platform::handle_budget::NativeHandleBudgetV1;
use goalcup_platform::sourcecap::{
    SourceCapError, SourceDir, SourceFile, SourceKind, SourceLookup, SourceName,
};
use goalcup_platform::tracecap::{HomeSeedRoot, TraceCapError};
use goalcup_protocol::events::{HomeSeedRecord, SeededFile, SeededKey};
use goalcup_protocol::manifest::HomeSeed;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, OnceLock};

/// Test-only handoff point immediately after a declared whole-file source directory has been
/// acquired. The regression test exchanges that pathname for a symlink here: production code
/// must keep reading through its retained directory capability, not reopen the spelling.
#[cfg(any(test, feature = "test-support"))]
struct SourceDirectoryHook {
    path: PathBuf,
    action: Box<dyn FnOnce() + Send + 'static>,
}

#[cfg(any(test, feature = "test-support"))]
static SOURCE_DIRECTORY_HOOK: OnceLock<Mutex<Option<SourceDirectoryHook>>> = OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
struct DestinationHomeHook {
    path: PathBuf,
    action: Box<dyn FnOnce() + Send + 'static>,
}

#[cfg(any(test, feature = "test-support"))]
static DESTINATION_HOME_HOOK: OnceLock<Mutex<Option<DestinationHomeHook>>> = OnceLock::new();

/// Install the one deterministic source-directory replacement used by the integration test.
///
/// This is compiled only for the crate's documented `test-support` feature and is never in a
/// shipped binary. The returned guard clears an unused hook when a test exits early.
#[cfg(any(test, feature = "test-support"))]
pub struct SeedHookGuard {
    path: PathBuf,
}

#[cfg(any(test, feature = "test-support"))]
pub fn replace_declared_source_directory_after_acquire_for_tests(
    path: PathBuf,
    action: impl FnOnce() + Send + 'static,
) -> SeedHookGuard {
    let hooks = SOURCE_DIRECTORY_HOOK.get_or_init(|| Mutex::new(None));
    let mut hook = hooks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        hook.is_none(),
        "only one source-directory hook may be installed"
    );
    *hook = Some(SourceDirectoryHook {
        path: path.clone(),
        action: Box::new(action),
    });
    SeedHookGuard { path }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for SeedHookGuard {
    fn drop(&mut self) {
        if let Some(hooks) = SOURCE_DIRECTORY_HOOK.get() {
            let mut hook = hooks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if hook
                .as_ref()
                .is_some_and(|waiting| waiting.path == self.path)
            {
                *hook = None;
            }
        }
        if let Some(hooks) = DESTINATION_HOME_HOOK.get() {
            let mut hook = hooks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if hook
                .as_ref()
                .is_some_and(|waiting| waiting.path == self.path)
            {
                *hook = None;
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn after_declared_source_directory_acquired(path: &Path) {
    let Some(hooks) = SOURCE_DIRECTORY_HOOK.get() else {
        return;
    };
    let action = {
        let mut hook = hooks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match hook.take() {
            Some(waiting) if waiting.path == path => Some(waiting.action),
            Some(waiting) => {
                *hook = Some(waiting);
                None
            }
            None => None,
        }
    };
    if let Some(action) = action {
        action();
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn after_declared_source_directory_acquired(_path: &Path) {}

/// Install the deterministic destination-home replacement used by the integration test.
#[cfg(any(test, feature = "test-support"))]
pub fn replace_destination_home_after_acquire_for_tests(
    path: PathBuf,
    action: impl FnOnce() + Send + 'static,
) -> SeedHookGuard {
    let hooks = DESTINATION_HOME_HOOK.get_or_init(|| Mutex::new(None));
    let mut hook = hooks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        hook.is_none(),
        "only one destination-home hook may be installed"
    );
    *hook = Some(DestinationHomeHook {
        path: path.clone(),
        action: Box::new(action),
    });
    SeedHookGuard { path }
}

#[cfg(any(test, feature = "test-support"))]
fn after_destination_home_acquired(path: &Path) {
    let Some(hooks) = DESTINATION_HOME_HOOK.get() else {
        return;
    };
    let action = {
        let mut hook = hooks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match hook.take() {
            Some(waiting) if waiting.path == path => Some(waiting.action),
            Some(waiting) => {
                *hook = Some(waiting);
                None
            }
            None => None,
        }
    };
    if let Some(action) = action {
        action();
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn after_destination_home_acquired(_path: &Path) {}

/// What one seeding did, so the caller can say it in one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// The file that was read, as it resolved on this machine.
    pub source: PathBuf,
    /// The file that was written, inside the per-run harness home.
    pub written: PathBuf,
    /// The keys actually copied, in manifest order. A key the source did not carry is not
    /// here: it was skipped rather than written as a null.
    pub keys: Vec<String>,
    /// The workspace spellings this run's trust entry was written under, empty when the
    /// connector declares no `trust_field`. Two of them where the daemon's spelling and its
    /// canonical form differ — see [`seed`].
    pub trusted: Vec<String>,
    /// Whole files copied verbatim, `(read, written)`, in manifest order.
    pub files: Vec<(PathBuf, PathBuf)>,
    /// Declared files the per-run home already held, left exactly as they were: on a resume
    /// the harness may have refreshed its own credential, and a seeding that put an older
    /// one back would sign the run out halfway through.
    pub kept: Vec<PathBuf>,
    /// Declared files this machine does not have. Reported, never refused — a home that was
    /// not seeded meets a sign-in screen a person can still answer.
    pub missing: Vec<PathBuf>,
    /// The same seeding as the chain records it.
    ///
    /// Built here, beside the copies, and by nothing else: a record assembled a second time
    /// from the fields above could disagree with what was actually written, and the raw
    /// values are only in hand at this moment anyway.
    ///
    /// **It holds no resolved path.** The fields above name files on this machine, because
    /// the participant reading the start line is owed the real names; the record names the
    /// manifest's spelling and the relative file names, because it is published.
    pub record: HomeSeedRecord,
}

impl Seeded {
    /// The one line `goalcup start` and `goalcup resume` print.
    ///
    /// It names both files and says which way the bytes went, because a participant reading
    /// "seeded" about their own account file is owed the sentence that says it was only read.
    pub fn line(&self) -> String {
        let mut said = match self.keys.is_empty() {
            false => format!(
                "seeded {} key(s) — {} — READ from {} into {}, so this run does not meet the \
                 harness's first-run onboarding. Nothing was written to {}.",
                self.keys.len(),
                self.keys.join(", "),
                self.source.display(),
                self.written.display(),
                self.source.display(),
            ),
            // A connector that copies whole files and merges no keys — `codex-cli` — says
            // the same thing about the files instead.
            true => format!(
                "seeded this run's harness home from {}: it was READ and nothing was written \
                 to it.",
                self.source.display()
            ),
        };
        if !self.files.is_empty() {
            said.push_str(&format!(
                " {} whole file(s) copied 0600 — {} — so this run is not asked to sign in \
                 again. They are your own credentials, on your own machine, in a directory \
                 the tracker made for this run; nothing uploads them and no trace names them.",
                self.files.len(),
                self.files
                    .iter()
                    .map(|(_, to)| to.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.kept.is_empty() {
            said.push_str(&format!(
                " {} file(s) this run's home already held were left as they are.",
                self.kept.len()
            ));
        }
        if !self.missing.is_empty() {
            said.push_str(&format!(
                " Not on this machine, so not copied: {}. If the harness asks you to sign \
                 in, sign in inside it once, or sign in against this run's own home before \
                 starting — README, \"The per-run home\", names the command for your \
                 harness.",
                self.missing
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.trusted.is_empty() {
            said.push_str(
                " The tracker's own empty workspace for this run is marked trusted in that \
                 same per-run file, so the harness does not stop at its per-project safety \
                 question; no other directory is named there.",
            );
        }
        said
    }
}

/// Copy the connector's declared keys out of the participant's configuration into the
/// per-run harness home.
///
/// `home` is the participant's real home, resolved by the caller; `harness_home` is the
/// directory the daemon created for this entry. Refuses — rung 1, so the participant can
/// act on it — when no candidate carries every `required` key, because the alternative is
/// launching a harness into a login wall no unattended run can answer.
///
/// `connector` is the manifest's id, and it is here for one reason: the record this builds
/// ([`Seeded::record`]) names the connector whose `[home_seed]` was followed, and a record
/// that named it from anywhere but the manifest that was read could name a different one.
pub fn seed(
    connector: &str,
    spec: &HomeSeed,
    home: &Path,
    harness_home: &Path,
    workspace: &Path,
) -> Result<Seeded, CliError> {
    let destination = HomeSeedRoot::open(harness_home)
        .map_err(|error| destination_capability_error(harness_home, error))?;
    after_destination_home_acquired(harness_home);

    // The whole-file clause first: it is independent of the key merge, and a
    // connector may declare either or both.
    let mut files = Vec::new();
    let mut kept = Vec::new();
    let mut missing = Vec::new();
    // …and, beside each of the three, what the chain records. Relative names
    // and digests where the lists above hold this machine's absolute paths.
    let mut files_recorded: Vec<SeededFile> = Vec::new();
    let mut files_kept: Vec<String> = Vec::new();
    let mut files_missing: Vec<String> = Vec::new();
    if let Some(dir) = &spec.from_dir {
        // The manifest's validator has already refused anything that does not begin
        // `{home}/`, so this is a substitution and not a second place that decides the root.
        let from_dir = home.join(dir.trim_start_matches("{home}/"));
        let source_dir = match open_home_source_root(home)? {
            Some(home_root) => open_declared_source_directory(
                &home_root,
                declared_source_components(dir)?,
                &from_dir,
            )?,
            None => None,
        };
        if source_dir.is_some() {
            after_declared_source_directory_acquired(&from_dir);
        }
        for name in &spec.files {
            let from = from_dir.join(name);
            let to = harness_home.join(name);
            let file = match source_dir.as_ref() {
                Some(root) => {
                    open_declared_source_file(root, relative_source_components(name)?, &from)?
                }
                // A missing or otherwise unreadable configuration directory is
                // reported as a missing optional credential.  A directory that is a
                // symlink is different: the retained-source traversal refuses it before this
                // branch, without resolving its target.
                None => None,
            };
            let Some(file) = file else {
                missing.push(from);
                files_missing.push(name.clone());
                continue;
            };
            // Never over an existing one: on a resume the harness has been writing into
            // this home for the length of the run's first half, and a credential it
            // refreshed there is newer than the one on this side.
            if destination
                .entry_exists(name)
                .map_err(|error| destination_capability_error(&to, error))?
            {
                kept.push(to);
                files_kept.push(name.clone());
                continue;
            }
            // Read whole, then atomically replaced through the retained destination
            // capability rather than copied in place: the temporary is private from
            // creation, and a killed process leaves either the old or complete new file.
            let bytes = read_declared_source_file(file, &from)?;
            destination
                .replace_private(name, &bytes)
                .map_err(|error| destination_capability_error(&to, error))?;
            // Name and length of the bytes already in hand — never a second read, never the
            // bytes themselves, and no digest of them either: a digest of a
            // credential is a stable identifier for it in a public trace. See `SeededFile`.
            files_recorded.push(SeededFile {
                name: name.clone(),
                bytes: bytes.len() as u64,
            });
            files.push((from, to));
        }
    }

    let mut tried: Vec<String> = Vec::new();
    // The matched candidate's own spelling travels with the path it resolved to, so the
    // record can name `{home}/.claude.json` where the start line names the resolved file.
    let mut source: Option<(String, PathBuf, Map<String, Value>)> = None;
    let home_root = open_home_source_root(home)?;
    for candidate in &spec.from {
        // The manifest's own validator has already refused anything that does not begin
        // `{home}/`, so this is a substitution and not a second place that decides the root.
        let path = home.join(candidate.trim_start_matches("{home}/"));
        tried.push(path.display().to_string());
        let object = match read_declared_source_object(
            home_root.as_ref(),
            declared_source_components(candidate)?,
            &path,
        )? {
            Some(object) => object,
            None => continue,
        };
        if spec.required.iter().all(|k| object.contains_key(k)) {
            source = Some((candidate.clone(), path, object));
            break;
        }
    }
    // A connector that merges no keys is finished: what it had to do was the files above.
    // `source` is the directory they were read from, so the line still names what was read.
    if spec.keys.is_empty() {
        return Ok(Seeded {
            source: spec
                .from_dir
                .as_ref()
                .map(|d| home.join(d.trim_start_matches("{home}/")))
                .unwrap_or_else(|| home.to_path_buf()),
            written: harness_home.to_path_buf(),
            keys: Vec::new(),
            trusted: Vec::new(),
            files,
            kept,
            missing,
            record: HomeSeedRecord {
                handoff_generation: None,
                handoff_request_digest: None,
                connector: connector.to_string(),
                // The file clause's own source, spelled as the manifest spells it: this
                // connector — Codex — read a directory and merged no keys. The default is
                // unreachable through the manifest validator, which refuses a `[home_seed]`
                // that declares neither `keys` nor `from_dir`.
                from: spec.from_dir.clone().unwrap_or_default(),
                keys: Vec::new(),
                keys_unchanged: Vec::new(),
                // No key merge, so no `trust_field` write either: this connector answers its
                // own trust question elsewhere, and the tracker decided nothing here.
                written: Vec::new(),
                files: files_recorded,
                files_kept,
                files_missing,
                wire_layout: Default::default(),
            },
        });
    }
    let (from_spelled, source_path, object) = source.ok_or_else(|| {
        CliError::Refused(format!(
            "this machine's harness configuration does not carry {}, so a run started here \
             would meet the harness's first-run onboarding — a theme picker and a login \
             screen — instead of your goal.\n  Looked in: {}\n  Run the harness once \
             outside the tracker and log in, then start the run again. `goalcup` reads that \
             file and never writes it.",
            spec.required
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(" and "),
            tried.join(", "),
        ))
    })?;

    let written = harness_home.join(&spec.to);
    // Merged into whatever is already there: on a resume that is everything the harness
    // wrote during the first half of the run.
    let mut document = match destination
        .read_merge_target(&spec.to)
        .map_err(|error| destination_capability_error(&written, error))?
    {
        Some(file) => read_object_from_file(file, &written)?.unwrap_or_default(),
        None => Map::new(),
    };
    let mut copied = Vec::new();
    let mut recorded: Vec<SeededKey> = Vec::new();
    let mut keys_unchanged: Vec<String> = Vec::new();
    for key in &spec.keys {
        if let Some(value) = object.get(key) {
            // Read before the insert: on a resume the destination is what the first launch
            // seeded and the harness has been writing into since, and a key already carrying
            // the value about to be written is written again to no effect. Unchanged and not
            // skipped — the merge has no skipped case, which is why the record does not
            // spell this list like `files_kept`. On a resume it is normally every key, and
            // it is a subset of `keys` rather than a list of its own. (A manifest that named
            // one key twice would see its own first write here and call the second
            // unchanged; the manifest validator is what keeps that out.)
            if document.get(key) == Some(value) {
                keys_unchanged.push(key.clone());
            }
            document.insert(key.clone(), value.clone());
            copied.push(key.clone());
            // The value rule is the protocol's and is applied in exactly one place
            // (`SeededKey::new`): booleans, numbers and strings of at most
            // `HOME_SEED_VALUE_MAX_BYTES` (32) bytes are carried; an object, an array,
            // `null` and a longer string are named only. 32 is what makes `userID` — a
            // 64-hex account identifier, stable across every entry a participant runs —
            // name-only.
            recorded.push(SeededKey::new(key, value));
        }
    }
    // The one thing here that is not a copy: this run's own workspace, marked trusted.
    //
    // Claude Code's per-project safety question is asked about a path that did not exist
    // when the participant's file was written, so no copied key can answer it, and its
    // default answer is `No, exit` — which is where a run would stop once the
    // onboarding keys had done their job. The directory is one the tracker created, empty,
    // for this entry alone.
    //
    // **Both spellings, where they differ.** The daemon answers a canonicalised workspace
    // and the harness keys this table on its own `getcwd`; under a symlinked parent — `/var`
    // → `/private/var`, which is where a macOS temporary directory lands — those are two
    // names for one directory, and an entry under only one of them leaves the question
    // asked. This is the same trap `handlers.rs` documents for the project slug.
    let mut trusted = Vec::new();
    // …and the record's own view of it: the trust keys with the value the
    // tracker gives them, and never the workspace path, which names a directory on this
    // machine. `Seeded::trusted` above is the participant's line and holds the real paths.
    let mut written_keys: Vec<SeededKey> = Vec::new();
    if let Some(field) = &spec.trust_field {
        let mut entry = Map::new();
        for key in &spec.trust_keys {
            entry.insert(key.clone(), Value::Bool(true));
        }
        let mut spellings = vec![workspace.display().to_string()];
        if let Ok(canonical) = std::fs::canonicalize(workspace) {
            let canonical = canonical.display().to_string();
            if !spellings.contains(&canonical) {
                spellings.push(canonical);
            }
        }
        let projects = document
            .entry(field.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(projects) = projects {
            for spelling in &spellings {
                // Merged into whatever the harness already recorded for this directory on an
                // earlier launch of the same run, for the same reason the document itself is.
                match projects.get_mut(spelling) {
                    Some(Value::Object(existing)) => existing.extend(entry.clone()),
                    _ => {
                        projects.insert(spelling.clone(), Value::Object(entry.clone()));
                    }
                }
            }
            trusted = spellings;
            // Recorded only where the write actually landed, for the same reason a seeding
            // that failed chains nothing: a record of a write that did not happen is a claim.
            for key in &spec.trust_keys {
                written_keys.push(SeededKey::new(key, &Value::Bool(true)));
            }
        }
    }
    let text = serde_json::to_string_pretty(&Value::Object(document))?;
    // It carries an account identity, so it is the participant's to read and nobody else's
    // — the retained capability's private replacement means it never exists under any other
    // mode, not even for the instant between an `open()` and a later `chmod`, and an
    // interrupted write on a resume leaves the harness's own live document exactly as it
    // was rather than truncated.
    destination
        .replace_private(&spec.to, text.as_bytes())
        .map_err(|error| destination_capability_error(&written, error))?;
    Ok(Seeded {
        source: source_path,
        written,
        keys: copied,
        trusted,
        files,
        kept,
        missing,
        record: HomeSeedRecord {
            handoff_generation: None,
            handoff_request_digest: None,
            connector: connector.to_string(),
            from: from_spelled,
            keys: recorded,
            keys_unchanged,
            written: written_keys,
            files: files_recorded,
            files_kept,
            files_missing,
            wire_layout: Default::default(),
        },
    })
}

/// The largest configuration document [`read_object`] reads whole into memory.
///
/// The participant's own `~/.claude.json` accumulates project
/// history for as long as they keep using the harness outside this tracker, and a machine
/// that has been signed in for months can carry one in the tens of megabytes. Copying two
/// keys out of it has no business loading whatever happens to be there first. The number is
/// the same cap `config.rs`'s `read_control_file` uses for the daemon's own document, for
/// the same reason: generous for anything this tool has ever written or read, and small
/// next to an account file nobody has ever bounded.
const HOME_SEED_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Retain a native, no-follow capability for the participant home before looking below it up.
///
/// `$HOME` itself may be a macOS system alias, so its physical directory is selected once and
/// then kept open. Every declared component is subsequently acquired relative to that retained
/// handle: Unix uses `openat` with `O_NOFOLLOW`, and Windows uses a rooted NT handle with
/// reparse-point rejection. Replacing an ancestor pathname later cannot retarget either read.
fn open_home_source_root(home: &Path) -> Result<Option<SourceDir>, CliError> {
    let physical = match std::fs::canonicalize(home) {
        Ok(path) if path.is_dir() => path,
        _ => return Ok(None),
    };
    SourceDir::open_selected(&physical, &NativeHandleBudgetV1::new())
        .map(Some)
        .map_err(|error| refused_source_capability(home, error))
}

/// Convert a validated manifest spelling into native single-component acquisitions.
///
/// Manifest validation has already ruled out `{home}` escapes. The second validation is needed
/// because `SourceDir` is the layer that reaches the filesystem and Windows has more native
/// aliases than the portable manifest spelling alone can describe.
fn relative_source_components(value: &str) -> Result<Vec<SourceName>, CliError> {
    value
        .split('/')
        .map(|component| {
            SourceName::from_component(component).map_err(|error| {
                CliError::Refused(format!(
                    "{value:?} is not a safe native [home_seed] source component: {error}"
                ))
            })
        })
        .collect()
}

fn declared_source_components(value: &str) -> Result<Vec<SourceName>, CliError> {
    let relative = value.strip_prefix("{home}/").ok_or_else(|| {
        CliError::Refused(format!(
            "{value:?} is not a [home_seed] source declared below {{home}}"
        ))
    })?;
    relative_source_components(relative)
}

/// Descend through a manifest-declared directory below a retained source root.
fn open_declared_source_directory(
    root: &SourceDir,
    names: Vec<SourceName>,
    display: &Path,
) -> Result<Option<SourceDir>, CliError> {
    open_declared_source_directory_from(root, &names, display)
}

fn open_declared_source_directory_from(
    root: &SourceDir,
    names: &[SourceName],
    display: &Path,
) -> Result<Option<SourceDir>, CliError> {
    let Some((name, remaining)) = names.split_first() else {
        return Err(CliError::Refused(format!(
            "{} has an empty [home_seed] source directory",
            display.display()
        )));
    };
    let observation = match root
        .observe(name)
        .map_err(|error| source_capability_error(display, error))?
    {
        SourceLookup::Missing => return Ok(None),
        SourceLookup::Present(observation) => observation,
    };
    if observation.kind() == SourceKind::SymbolicLink {
        return Err(refused_source_symlink(display));
    }
    if observation.kind() != SourceKind::Directory {
        return Ok(None);
    }
    let directory = root
        .open_dir_matching(name, &observation)
        .map_err(|error| source_capability_error(display, error))?;
    if remaining.is_empty() {
        Ok(Some(directory))
    } else {
        open_declared_source_directory_from(&directory, remaining, display)
    }
}

/// Open a regular declared source relative to an already retained directory capability.
fn open_declared_source_file(
    root: &SourceDir,
    names: Vec<SourceName>,
    display: &Path,
) -> Result<Option<SourceFile>, CliError> {
    open_declared_source_file_from(root, &names, display, HOME_SEED_MAX_BYTES)
}

fn open_declared_source_object_file(
    root: &SourceDir,
    names: Vec<SourceName>,
    display: &Path,
) -> Result<Option<SourceFile>, CliError> {
    open_declared_source_file_from(root, &names, display, HOME_SEED_MAX_BYTES)
}

fn open_declared_source_file_from(
    root: &SourceDir,
    names: &[SourceName],
    display: &Path,
    max_len: u64,
) -> Result<Option<SourceFile>, CliError> {
    let Some((name, remaining)) = names.split_first() else {
        return Err(CliError::Refused(format!(
            "{} has an empty [home_seed] source file",
            display.display()
        )));
    };
    let observation = match root
        .observe(name)
        .map_err(|error| source_capability_error(display, error))?
    {
        SourceLookup::Missing => return Ok(None),
        SourceLookup::Present(observation) => observation,
    };
    if observation.kind() == SourceKind::SymbolicLink {
        return Err(refused_source_symlink(display));
    }
    if !remaining.is_empty() {
        if observation.kind() != SourceKind::Directory {
            return Ok(None);
        }
        let directory = root
            .open_dir_matching(name, &observation)
            .map_err(|error| source_capability_error(display, error))?;
        return open_declared_source_file_from(&directory, remaining, display, max_len);
    }
    if observation.kind() != SourceKind::Regular {
        return Ok(None);
    }
    if observation.length() > max_len {
        return Err(oversized_source_refusal(display, observation.length()));
    }
    root.open_regular_matching(name, &observation, max_len)
        .map(Some)
        .map_err(|error| source_capability_error(display, error))
}

fn read_declared_source_file(mut file: SourceFile, path: &Path) -> Result<Vec<u8>, CliError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    (&mut file)
        .take(HOME_SEED_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| CliError::Io {
            path: path.display().to_string(),
            source,
        })?;
    file.finish()
        .map_err(|error| source_capability_error(path, error))?;
    if bytes.len() as u64 > HOME_SEED_MAX_BYTES {
        return Err(oversized_source_refusal(path, bytes.len() as u64));
    }
    Ok(bytes)
}

fn source_capability_error(path: &Path, error: SourceCapError) -> CliError {
    match error {
        SourceCapError::Io { source, .. } => CliError::Io {
            path: path.display().to_string(),
            source,
        },
        other => refused_source_capability(path, other),
    }
}

fn refused_source_capability(path: &Path, error: SourceCapError) -> CliError {
    CliError::Refused(format!(
        "{} cannot be safely acquired as a declared [home_seed] source: {error}",
        path.display()
    ))
}

fn refused_source_symlink(path: &Path) -> CliError {
    CliError::Refused(format!(
        "{} is a symbolic link at a declared [home_seed] source. Refusing to follow it; \
         nothing was read from it.",
        path.display()
    ))
}

fn destination_capability_error(path: &Path, error: TraceCapError) -> CliError {
    CliError::Refused(format!(
        "{} cannot be safely acquired as a [home_seed] destination: {error}",
        path.display()
    ))
}

/// Read the participant's declared source JSON object, never a symlink target.
///
/// A corrupt or unreadable normal configuration remains a candidate miss. The explicit
/// symlink/containment refusals remain
/// actionable instead of being turned into an apparent first-run configuration.
fn read_declared_source_object(
    home_root: Option<&SourceDir>,
    names: Vec<SourceName>,
    path: &Path,
) -> Result<Option<Map<String, Value>>, CliError> {
    let Some(home_root) = home_root else {
        return Ok(None);
    };
    let file = match open_declared_source_object_file(home_root, names, path) {
        Ok(file) => file,
        Err(CliError::Io { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(mut file) = file else {
        return Ok(None);
    };
    let object = read_object_from_file(&mut file, path)?;
    file.finish()
        .map_err(|error| source_capability_error(path, error))?;
    Ok(object)
}

/// The file's top-level JSON object, `None` when it is missing, unreadable, not JSON, or
/// JSON that is not an object — and `Err` when it is there and over
/// [`HOME_SEED_MAX_BYTES`], naming the size, because that is a different fact from "there is
/// nothing here to read" and the participant is owed the distinction.
///
/// The same bounded decoder is used by the retained source and destination file handles.
/// Every `None` case means there is no object here to take keys out of or merge into. A
/// participant's own configuration that has been corrupted is not something this command
/// can fix, and the refusal it produces — through the `required` check in [`seed`] — already
/// says what to do about it.
fn read_object_from_file(
    file: impl std::io::Read,
    path: &Path,
) -> Result<Option<Map<String, Value>>, CliError> {
    use std::io::Read;
    let mut text = String::new();
    let read = match file.take(HOME_SEED_MAX_BYTES + 1).read_to_string(&mut text) {
        Ok(read) => read,
        // Unreadable — permission denied, not UTF-8 — is the same "nothing to read" answer
        // a missing file gets; the refusal below is reserved for the one case that is a
        // different fact rather than another shape of the same one.
        Err(_) => return Ok(None),
    };
    if read as u64 > HOME_SEED_MAX_BYTES {
        return Err(oversized_source_refusal(path, read as u64));
    }
    Ok(match serde_json::from_str::<Value>(&text).ok() {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    })
}

fn oversized_source_refusal(path: &Path, _length: u64) -> CliError {
    CliError::Refused(format!(
        "{} is larger than {HOME_SEED_MAX_BYTES} bytes, which is more than any \
         configuration file this seed reads. Nothing was read from it.",
        path.display()
    ))
}
