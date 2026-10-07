//! Sessions that are not this run's: the bounded sweep (D10) and the pre-arm scan that
//! refuses to arm a harness home already occupied.
//!
//! **Cadenced calls, never a watcher.** A background task walking a participant's home for
//! 168 hours would be an unbounded read of a machine the tracker was given one workspace
//! on. [`EntryRuntime`](crate::runtime::EntryRuntime) instead calls
//! [`scan_once`] on the heartbeat cadence and once at seal. Its immutable trusted plan and
//! remembered coverage/sighting identities make outcomes initial-or-transition evidence;
//! this module spawns no task and performs no scan except on those explicit calls.
//!
//! **Identifiers and paths, never content (D20).** A sighting is somebody else's session.
//! A line declaration reads one bounded first record and extracts only its two declared
//! identity fields. A SQLite declaration selects only type/length metadata and its two
//! declared identity columns. [`ForeignSighting::to_event`] publishes less again: the
//! matched `directory` stays out of the wire event, because it is the participant's own
//! path and naming the session and the store it lives in is what a reviewer needs.
//!
//! **Two kinds of store, because harnesses keep sessions in two kinds of file.** A line
//! store's first record is its first line, parsed as JSON. A SQLite store declared by a
//! [`ForeignScanReader::SqliteSession`] is read through
//! [`crate::tailer::sqlite::open_read_only`], the same `file:…?mode=ro` connection the
//! tailer opens on the store it owns. The pre-arm scan reads line stores only; no shipped
//! connector keeps its own sessions in SQLite.
//!
//! **The compare is byte-exact, so the seeding site owes a canonical workspace.** The same
//! contract [`crate::partition`] states, inherited here for the same reason: normalising a
//! path would be interpretation and this module does none. A registry entry holding
//! `/var/folders/…` where the harness wrote `/private/var/folders/…` in its own `cwd` makes
//! this module blind to a session sitting in the participant's own workspace — the
//! pre-arm scan arms a home that is occupied, and the sweep reports no sighting. The
//! workspace must be what [`std::fs::canonicalize`] returns, with no trailing slash, and
//! `an_uncanonicalised_registry_workspace_makes_a_private_var_occupant_invisible` pins that
//! outcome so the contract cannot be read as advice.

use crate::arming::RefusalReason;
use crate::manifests::LoadedManifest;
/// Re-exported at its old path: the V1 scan planning below hangs off this type.
pub use crate::manifests::TrustedManifests;
use crate::paths::{
    ForeignGlobResult, PathVars, expand_foreign_glob, expand_globs, resolve_template,
};
use crate::registry::{PendingForeignSighting, Registry};
use crate::tailer::dotted_json;
use crate::{DaemonError, MAX_LINE_BYTES, now_rfc3339};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::events::{ForeignScanOutcome, ForeignScanUnavailableReason};
use goalcup_protocol::manifest::{
    ConnectorManifest, ForeignScan, ForeignScanReader, ForeignScanRoot, PartitionSpec,
};
use goalcup_protocol::pins::connector_dir_is_pinned;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;

/// The most session rows one read takes. Bounded memory is a constraint on the whole
/// daemon and this is somebody else's database; the question either caller is asking —
/// "is anyone working in a directory I track" — is answered by the rows, not by all of
/// them, and a home holding more sessions than this has been in use for years.
pub const MAX_FOREIGN_SESSIONS: usize = 4_096;
pub const MAX_FOREIGN_SCAN_PATHS: usize = 4_096;
pub const MAX_FOREIGN_SIGHTINGS: usize = 4_096;

/// Claude Code may write a short run of bookkeeping before its first record with both
/// declared identity fields.  This reader never turns that into an unbounded transcript
/// walk: it examines at most this many JSONL records from each matched source.
const MAX_FOREIGN_IDENTITY_PREFIX_RECORDS: usize = 64;
/// The total input budget for one [`ForeignScanReader::FileFirstIdentity`] source.
///
/// This bounds its line buffer and retained partial identifiers independently of the
/// declaration-wide identity projection budget below.  A candidate at this exact boundary
/// is usable; otherwise the reader reports `record_limit` without probing into the suffix.
const MAX_FOREIGN_IDENTITY_PREFIX_BYTES: usize = MAX_LINE_BYTES as usize;

const MAX_DECLARATION_IDENTITY_BYTES: usize = 8 * MAX_LINE_BYTES as usize;
const MAX_SWEEP_IDENTITY_BYTES: usize = 16 * MAX_LINE_BYTES as usize;

#[derive(Debug, Clone)]
pub(crate) struct TrustedForeignScan {
    declaring_connector: String,
    declaring_manifest_sha256: String,
    scan_index: u64,
    subject_family: String,
    subject_harness: String,
    scan: ForeignScan,
}

/// Compile every installed `foreign_scan` declaration from the loaded, pinned connectors.
///
/// The manifest map itself is loaded by [`crate::manifests::TrustedManifests`], which a
/// local capture daemon needs too; the scan declarations are the V1 path's alone.
pub(crate) fn compile_declarations(
    loaded: &BTreeMap<String, LoadedManifest>,
) -> Result<Vec<TrustedForeignScan>, DaemonError> {
    let mut declarations = Vec::new();
    for owner in loaded
        .values()
        .filter(|row| !row.manifest.foreign_scan.is_empty())
    {
        for (scan_index, scan) in owner.manifest.foreign_scan.iter().enumerate() {
            trusted_scan_root(owner, scan_index, scan)?;
        }
        trusted_installed(owner)?;
        for (scan_index, scan) in owner.manifest.foreign_scan.iter().enumerate() {
            let target = loaded.get(&scan.harness).ok_or_else(|| {
                DaemonError::Config(format!(
                    "connector `{}` foreign_scan[{scan_index}] names uninstalled target `{}`",
                    owner.manifest.id, scan.harness
                ))
            })?;
            trusted_installed(target)?;
            declarations.push(TrustedForeignScan {
                declaring_connector: owner.manifest.id.clone(),
                declaring_manifest_sha256: owner.manifest_sha256.clone(),
                scan_index: scan_index as u64,
                subject_family: target.manifest.family.clone(),
                subject_harness: target.manifest.id.clone(),
                scan: scan.clone(),
            });
        }
    }
    Ok(declarations)
}

impl TrustedManifests {
    /// Freeze only the roots applicable to this entry's exact connector identity.
    pub fn plan_for(&self, active: &ConnectorManifest) -> ForeignScanPlan {
        ForeignScanPlan {
            declarations: self
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.scan.root == ForeignScanRoot::Home
                        || (declaration.subject_family == active.family
                            && declaration.subject_harness == active.id)
                })
                .cloned()
                .collect(),
        }
    }
}

fn trusted_scan_root(
    owner: &LoadedManifest,
    scan_index: usize,
    scan: &ForeignScan,
) -> Result<(), DaemonError> {
    let valid = match scan.root {
        ForeignScanRoot::Home => scan.path_glob.starts_with("{home}/"),
        ForeignScanRoot::OwnConfigDir => {
            scan.harness == owner.manifest.id && scan.path_glob.starts_with("{config_dir}/")
        }
    };
    if valid {
        Ok(())
    } else {
        Err(DaemonError::Config(format!(
            "connector `{}` foreign_scan[{scan_index}] has a root that does not authorize `{}`",
            owner.manifest.id, scan.path_glob
        )))
    }
}

fn trusted_installed(row: &LoadedManifest) -> Result<(), DaemonError> {
    row.manifest.validate().map_err(|error| {
        DaemonError::Config(format!(
            "connector `{}` is not a valid scan authority: {error}",
            row.manifest.id
        ))
    })?;
    if row.directory_name != row.manifest.id {
        return Err(DaemonError::Config(format!(
            "connector directory `{}` declares id `{}`",
            row.directory_name, row.manifest.id
        )));
    }
    if !connector_dir_is_pinned(&row.manifest.id, &row.directory_sha256) {
        return Err(DaemonError::Config(format!(
            "connector `{}` contributes a foreign scan but its directory digest is not pinned",
            row.manifest.id
        )));
    }
    Ok(())
}

/// Immutable declaration snapshot held by one entry for its whole runtime.
#[derive(Debug, Clone, Default)]
pub struct ForeignScanPlan {
    declarations: Vec<TrustedForeignScan>,
}

/// One session belonging to somebody else, working in a directory this daemon tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignSighting {
    /// The harness whose store this is — the connector's own, or another one it declares
    /// a glob for.
    pub harness: String,
    pub family: String,
    pub session_id: String,
    /// The file the record was read from.
    pub store_path: String,
    /// What the sweep matched: the working directory the foreign session declared. Held so
    /// the caller can say why this is a sighting, and deliberately not on the wire event
    /// (D20) — see [`ForeignSighting::to_event`].
    pub directory: String,
    pub first_seen: String,
    /// The trusted root that produced this identity. This stays internal to the daemon's
    /// settlement logic and is never serialized on the wire event.
    pub(crate) scan_root: ForeignScanRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignCoverage {
    pub declaring_connector: String,
    pub declaring_manifest_sha256: String,
    pub scan_index: u64,
    pub outcome: ForeignScanOutcome,
}

impl ForeignCoverage {
    pub(crate) fn declaration_key(&self) -> ForeignDeclarationKey {
        (
            self.declaring_connector.clone(),
            self.declaring_manifest_sha256.clone(),
            self.scan_index,
        )
    }

    pub fn to_event(&self, at: &str) -> EventBody {
        EventBody::ForeignScanCoverage {
            declaring_connector: self.declaring_connector.clone(),
            declaring_manifest_sha256: self.declaring_manifest_sha256.clone(),
            scan_index: self.scan_index,
            outcome: self.outcome.clone(),
            at: at.to_string(),
        }
    }
}

/// The immutable identity of a trusted foreign-scan declaration.
///
/// This intentionally carries only declaring-manifest identity and row number: runtime
/// settling keeps no source path, transcript byte, or observed identity.
pub(crate) type ForeignDeclarationKey = (String, String, u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignScanResult {
    pub coverage: Vec<ForeignCoverage>,
    pub sightings: Vec<ForeignSighting>,
    /// Declarations with a clean EOF after at least one valid identity-prefix record and no
    /// hard source result. The runtime decides whether a live observation is settled; the
    /// wire outcome remains the existing `Unavailable(Identity)` shape.
    pub(crate) eligible_identity: BTreeSet<ForeignDeclarationKey>,
}

impl ForeignSighting {
    /// Identifiers and paths only. `directory` is what the sweep matched and is
    /// deliberately not on the wire event (D20).
    ///
    /// The reviewer's question is "was a second session live in this run's workspace", and
    /// the answer is the harness, the session id and the store it was found in. The
    /// directory is the participant's own filesystem path and adds nothing to that answer
    /// that the sighting's existence does not already carry.
    pub fn to_event(&self) -> EventBody {
        EventBody::ForeignSession {
            harness: self.harness.clone(),
            session_id: self.session_id.clone(),
            store_path: self.store_path.clone(),
            first_seen: self.first_seen.clone(),
        }
    }

    pub(crate) fn to_pending(&self) -> PendingForeignSighting {
        PendingForeignSighting {
            family: self.family.clone(),
            harness: self.harness.clone(),
            session_id: self.session_id.clone(),
            store_path: self.store_path.clone(),
            first_seen: self.first_seen.clone(),
            scan_root: self.scan_root,
        }
    }

    pub(crate) fn from_pending(pending: &PendingForeignSighting) -> Self {
        ForeignSighting {
            harness: pending.harness.clone(),
            family: pending.family.clone(),
            session_id: pending.session_id.clone(),
            store_path: pending.store_path.clone(),
            directory: String::new(),
            first_seen: pending.first_seen.clone(),
            scan_root: pending.scan_root,
        }
    }
}

/// Read-only. Inspects only the bounded identity projection declared by each [`ForeignScan`].
///
/// `directories` is the set of working directories this daemon tracks; a session working
/// anywhere else is not this tracker's business and is not returned. Results are sorted by
/// store path, so two runs of the same sweep over the same disk produce the same list.
pub fn scan_once(
    plan: &ForeignScanPlan,
    vars: &PathVars,
    directories: &[PathBuf],
) -> Result<ForeignScanResult, DaemonError> {
    let tracked: Vec<String> = directories
        .iter()
        .map(|d| d.display().to_string())
        .collect();
    let mut state = SweepState::default();
    let mut coverage = Vec::with_capacity(plan.declarations.len());
    let mut eligible_identity = BTreeSet::new();
    for declaration in &plan.declarations {
        let (outcome, eligible) = match state.exhausted {
            Some(reason) => (ForeignScanOutcome::Unavailable(reason), false),
            None => match scan_declaration(declaration, vars, &tracked, &mut state)? {
                DeclarationScanOutcome::Outcome(outcome) => (outcome, false),
                DeclarationScanOutcome::EligibleIdentity => (
                    ForeignScanOutcome::Unavailable(ForeignScanUnavailableReason::Identity),
                    true,
                ),
            },
        };
        let coverage_row = ForeignCoverage {
            declaring_connector: declaration.declaring_connector.clone(),
            declaring_manifest_sha256: declaration.declaring_manifest_sha256.clone(),
            scan_index: declaration.scan_index,
            outcome,
        };
        if eligible {
            eligible_identity.insert(coverage_row.declaration_key());
        }
        coverage.push(coverage_row);
        if state.exhausted.is_none() {
            if state.sightings.len() == MAX_FOREIGN_SIGHTINGS {
                state.exhausted = Some(ForeignScanUnavailableReason::SightingLimit);
            } else if state.identity_bytes == MAX_SWEEP_IDENTITY_BYTES {
                state.exhausted = Some(ForeignScanUnavailableReason::RecordLimit);
            }
        }
    }
    state.sightings.sort_by(|a, b| {
        (&a.family, &a.harness, &a.session_id, &a.store_path).cmp(&(
            &b.family,
            &b.harness,
            &b.session_id,
            &b.store_path,
        ))
    });
    Ok(ForeignScanResult {
        coverage,
        sightings: state.sightings,
        eligible_identity,
    })
}

#[derive(Default)]
struct SweepState {
    identity_bytes: usize,
    sightings: Vec<ForeignSighting>,
    sighting_keys: BTreeSet<(String, String, String)>,
    exhausted: Option<ForeignScanUnavailableReason>,
}

/// One declaration's result before the runtime decides whether a clean EOF is stable.
enum DeclarationScanOutcome {
    Outcome(ForeignScanOutcome),
    EligibleIdentity,
}

fn scan_declaration(
    declaration: &TrustedForeignScan,
    vars: &PathVars,
    tracked: &[String],
    state: &mut SweepState,
) -> Result<DeclarationScanOutcome, DaemonError> {
    let pattern = resolve_template(&declaration.scan.path_glob, vars)?;
    let paths = match expand_foreign_glob(&pattern, MAX_FOREIGN_SCAN_PATHS) {
        ForeignGlobResult::Absent => {
            return Ok(DeclarationScanOutcome::Outcome(ForeignScanOutcome::Absent));
        }
        ForeignGlobResult::Unavailable(reason) => {
            return Ok(DeclarationScanOutcome::Outcome(
                ForeignScanOutcome::Unavailable(reason),
            ));
        }
        ForeignGlobResult::Paths(paths) => paths,
    };
    let mut declaration_bytes = 0usize;
    let mut rows_seen = 0usize;
    let mut eligible_identity = false;
    for path in paths {
        let result = match &declaration.scan.reader {
            ForeignScanReader::FileFirstRecord {} => read_line_source(
                &path,
                declaration,
                tracked,
                &mut rows_seen,
                &mut declaration_bytes,
                state,
            ),
            ForeignScanReader::FileFirstIdentity {} => match read_line_identity_source(
                &path,
                declaration,
                tracked,
                &mut rows_seen,
                &mut declaration_bytes,
                state,
            ) {
                Ok(LineIdentitySourceOutcome::Complete) => Ok(()),
                Ok(LineIdentitySourceOutcome::EligibleIdentity) => {
                    eligible_identity = true;
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            ForeignScanReader::SqliteSession {
                table,
                session_id_column,
                directory_column,
            } => read_sqlite_source(
                &path,
                declaration,
                tracked,
                table,
                session_id_column,
                directory_column,
                &mut rows_seen,
                &mut declaration_bytes,
                state,
            ),
        };
        if let Err(reason) = result {
            return Ok(DeclarationScanOutcome::Outcome(
                ForeignScanOutcome::Unavailable(reason),
            ));
        }
    }
    Ok(if eligible_identity {
        DeclarationScanOutcome::EligibleIdentity
    } else {
        DeclarationScanOutcome::Outcome(ForeignScanOutcome::Complete)
    })
}

/// A file-first-identity source either supplied a candidate or ended cleanly after valid
/// bookkeeping that did not yet contain a joint identity.
enum LineIdentitySourceOutcome {
    Complete,
    EligibleIdentity,
}

#[allow(clippy::too_many_arguments)]
fn read_line_identity_source(
    path: &Path,
    declaration: &TrustedForeignScan,
    tracked: &[String],
    rows_seen: &mut usize,
    declaration_bytes: &mut usize,
    state: &mut SweepState,
) -> Result<LineIdentitySourceOutcome, ForeignScanUnavailableReason> {
    if *rows_seen == MAX_FOREIGN_SESSIONS {
        return Err(ForeignScanUnavailableReason::RowLimit);
    }
    let mut file = open_regular_source(path).map_err(scan_open_reason)?;
    let mut prefix_bytes = 0usize;
    let mut prefix_records = 0usize;
    let mut observed_session = None;
    let mut observed_directory = None;

    loop {
        if prefix_records == MAX_FOREIGN_IDENTITY_PREFIX_RECORDS {
            return Err(ForeignScanUnavailableReason::RowLimit);
        }
        let Some(line) = read_identity_prefix_line(&mut file, &mut prefix_bytes)? else {
            return if prefix_records == 0 {
                Err(ForeignScanUnavailableReason::Identity)
            } else {
                Ok(LineIdentitySourceOutcome::EligibleIdentity)
            };
        };
        prefix_records += 1;
        let record: serde_json::Value =
            serde_json::from_slice(&line).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let session_id = identity_field(&record, &declaration.scan.session_id_field)?;
        let directory = identity_field(&record, &declaration.scan.directory_field)?;
        observe_identity(&mut observed_session, session_id)?;
        observe_identity(&mut observed_directory, directory)?;

        // A pair belongs to one record.  Joining a session from one bookkeeping record to a
        // directory from another could invent an identity no source ever asserted.
        let (Some(session_id), Some(directory)) = (session_id, directory) else {
            continue;
        };
        *rows_seen += 1;
        reserve_identity(session_id.len(), directory.len(), declaration_bytes, state)?;
        if tracked.iter().any(|candidate| candidate == directory) {
            retain_sighting(path, declaration, session_id, directory, state)?;
        }
        return Ok(LineIdentitySourceOutcome::Complete);
    }
}

/// Read one JSONL record without fetching a byte after the bounded identity prefix.
///
/// A one-byte read is intentional.  `BufReader` may prefetch bytes after the deciding
/// record; this scanner is authorized only for the prefix and stops as soon as it finds the
/// first complete identity.
fn read_identity_prefix_line(
    file: &mut File,
    prefix_bytes: &mut usize,
) -> Result<Option<Vec<u8>>, ForeignScanUnavailableReason> {
    let mut line = Vec::new();
    loop {
        if *prefix_bytes == MAX_FOREIGN_IDENTITY_PREFIX_BYTES {
            // The preceding one-byte reads consumed the whole authorized prefix.  The source
            // is already guarded as a regular file, so its length confirms EOF without
            // reading a suffix byte.
            return match file.metadata() {
                Ok(metadata) if metadata.len() == *prefix_bytes as u64 => {
                    Ok((!line.is_empty()).then_some(line))
                }
                Ok(_) => Err(ForeignScanUnavailableReason::RecordLimit),
                Err(_) => Err(ForeignScanUnavailableReason::Open),
            };
        }
        let mut byte = [0_u8; 1];
        match file.read(&mut byte) {
            Ok(0) => return Ok((!line.is_empty()).then_some(line)),
            Ok(_) => {
                *prefix_bytes += 1;
                if byte[0] == b'\n' {
                    return Ok(Some(line));
                }
                line.push(byte[0]);
                if line.len() > MAX_LINE_BYTES as usize {
                    return Err(ForeignScanUnavailableReason::RecordLimit);
                }
            }
            Err(_) => return Err(ForeignScanUnavailableReason::Open),
        }
    }
}

fn identity_field<'a>(
    record: &'a serde_json::Value,
    path: &str,
) -> Result<Option<&'a str>, ForeignScanUnavailableReason> {
    let Some(value) = dotted_json(record, path) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(ForeignScanUnavailableReason::Identity)?;
    Ok(Some(value))
}

fn observe_identity(
    observed: &mut Option<String>,
    value: Option<&str>,
) -> Result<(), ForeignScanUnavailableReason> {
    let Some(value) = value else {
        return Ok(());
    };
    match observed {
        Some(previous) if previous != value => Err(ForeignScanUnavailableReason::Identity),
        Some(_) => Ok(()),
        None => {
            *observed = Some(value.to_owned());
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_line_source(
    path: &Path,
    declaration: &TrustedForeignScan,
    tracked: &[String],
    rows_seen: &mut usize,
    declaration_bytes: &mut usize,
    state: &mut SweepState,
) -> Result<(), ForeignScanUnavailableReason> {
    if *rows_seen == MAX_FOREIGN_SESSIONS {
        return Err(ForeignScanUnavailableReason::RowLimit);
    }
    let file = open_regular_source(path).map_err(scan_open_reason)?;
    let mut head = Vec::new();
    file.take(MAX_LINE_BYTES + 1)
        .read_to_end(&mut head)
        .map_err(|_| ForeignScanUnavailableReason::Open)?;
    let line = match head.iter().position(|byte| *byte == b'\n') {
        Some(end) if end <= MAX_LINE_BYTES as usize => &head[..end],
        Some(_) => return Err(ForeignScanUnavailableReason::RecordLimit),
        None if head.len() <= MAX_LINE_BYTES as usize => &head[..],
        None => return Err(ForeignScanUnavailableReason::RecordLimit),
    };
    *rows_seen += 1;
    let record: serde_json::Value =
        serde_json::from_slice(line).map_err(|_| ForeignScanUnavailableReason::Row)?;
    let directory = str_field_ref(&record, &declaration.scan.directory_field)
        .ok_or(ForeignScanUnavailableReason::Identity)?;
    let session_id = str_field_ref(&record, &declaration.scan.session_id_field)
        .ok_or(ForeignScanUnavailableReason::Identity)?;
    reserve_identity(session_id.len(), directory.len(), declaration_bytes, state)?;
    if tracked.iter().any(|candidate| candidate == directory) {
        retain_sighting(path, declaration, session_id, directory, state)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_sqlite_source(
    path: &Path,
    declaration: &TrustedForeignScan,
    tracked: &[String],
    table: &str,
    session_id_column: &str,
    directory_column: &str,
    rows_seen: &mut usize,
    declaration_bytes: &mut usize,
    state: &mut SweepState,
) -> Result<(), ForeignScanUnavailableReason> {
    // This makes an already-present FIFO/device/directory/symbolic link unavailable before
    // SQLite receives the pathname. It does not turn SQLite's later pathname open into a
    // held-identity operation; that separate race is reported as a platform limitation.
    let _source_kind_guard = open_regular_source(path).map_err(scan_open_reason)?;
    let conn = crate::tailer::sqlite::open_read_only(&path.display().to_string())
        .map_err(|_| ForeignScanUnavailableReason::Open)?;
    let remaining = MAX_FOREIGN_SESSIONS.saturating_sub(*rows_seen);
    let sql = format!(
        "SELECT typeof(\"{session_id_column}\"), \
                 length(CAST(\"{session_id_column}\" AS BLOB)), \
                 typeof(\"{directory_column}\"), \
                 length(CAST(\"{directory_column}\" AS BLOB)), \
                 CASE WHEN typeof(\"{session_id_column}\") = 'text' \
                           AND length(CAST(\"{session_id_column}\" AS BLOB)) <= ?1 \
                      THEN \"{session_id_column}\" ELSE NULL END, \
                 CASE WHEN typeof(\"{directory_column}\") = 'text' \
                           AND length(CAST(\"{directory_column}\" AS BLOB)) <= ?1 \
                      THEN \"{directory_column}\" ELSE NULL END \
           FROM \"{table}\" LIMIT ?2"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|_| ForeignScanUnavailableReason::Schema)?;
    let mut rows = statement
        .query(rusqlite::params![
            MAX_LINE_BYTES as i64,
            (remaining + 1) as i64
        ])
        .map_err(|_| ForeignScanUnavailableReason::Schema)?;
    loop {
        let row = rows.next().map_err(|_| ForeignScanUnavailableReason::Row)?;
        let Some(row) = row else { break };
        if *rows_seen == MAX_FOREIGN_SESSIONS {
            return Err(ForeignScanUnavailableReason::RowLimit);
        }
        *rows_seen += 1;

        // Both fields' type and byte length are read before either value is requested.
        let session_kind: String = row.get(0).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let session_len: Option<i64> = row.get(1).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let directory_kind: String = row.get(2).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let directory_len: Option<i64> =
            row.get(3).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let (Some(session_len), Some(directory_len)) = (session_len, directory_len) else {
            return Err(ForeignScanUnavailableReason::Identity);
        };
        if session_kind != "text"
            || directory_kind != "text"
            || session_len < 0
            || directory_len < 0
            || session_len as u64 > MAX_LINE_BYTES
            || directory_len as u64 > MAX_LINE_BYTES
        {
            return Err(ForeignScanUnavailableReason::Identity);
        }
        reserve_identity(
            session_len as usize,
            directory_len as usize,
            declaration_bytes,
            state,
        )?;
        let session_id: String = row.get(4).map_err(|_| ForeignScanUnavailableReason::Row)?;
        let directory: String = row.get(5).map_err(|_| ForeignScanUnavailableReason::Row)?;
        if tracked.iter().any(|candidate| candidate == &directory) {
            retain_sighting(path, declaration, &session_id, &directory, state)?;
        }
    }
    Ok(())
}

/// Open one discovered line/source-kind guard without following a final symbolic link or
/// blocking on a FIFO, then validate the type from the opened handle. SQLite still opens
/// its own pathname after this guard; callers must not describe that as stable identity.
fn open_regular_source(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT

    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "foreign source is not a regular file",
        ));
    }
    Ok(file)
}

fn scan_open_reason(error: std::io::Error) -> ForeignScanUnavailableReason {
    if error.kind() == std::io::ErrorKind::NotFound {
        ForeignScanUnavailableReason::Race
    } else {
        ForeignScanUnavailableReason::Open
    }
}

fn reserve_identity(
    session_bytes: usize,
    directory_bytes: usize,
    declaration_bytes: &mut usize,
    state: &mut SweepState,
) -> Result<(), ForeignScanUnavailableReason> {
    if session_bytes > MAX_LINE_BYTES as usize || directory_bytes > MAX_LINE_BYTES as usize {
        return Err(ForeignScanUnavailableReason::Identity);
    }
    let Some(bytes) = session_bytes.checked_add(directory_bytes) else {
        return Err(ForeignScanUnavailableReason::RecordLimit);
    };
    if declaration_bytes.saturating_add(bytes) > MAX_DECLARATION_IDENTITY_BYTES {
        return Err(ForeignScanUnavailableReason::RecordLimit);
    }
    if state.identity_bytes.saturating_add(bytes) > MAX_SWEEP_IDENTITY_BYTES {
        state.exhausted = Some(ForeignScanUnavailableReason::RecordLimit);
        return Err(ForeignScanUnavailableReason::RecordLimit);
    }
    *declaration_bytes += bytes;
    state.identity_bytes += bytes;
    Ok(())
}

fn retain_sighting(
    path: &Path,
    declaration: &TrustedForeignScan,
    session_id: &str,
    directory: &str,
    state: &mut SweepState,
) -> Result<(), ForeignScanUnavailableReason> {
    let key = (
        declaration.subject_family.clone(),
        declaration.subject_harness.clone(),
        session_id.to_string(),
    );
    if state.sighting_keys.contains(&key) {
        // One identity can be covered by both a participant-home and an own-config
        // declaration. Home provenance is the conservative winner: a session visible in the
        // participant's home is immediate evidence even if another declaration found the same
        // id under the per-run config root first.
        if declaration.scan.root == ForeignScanRoot::Home
            && let Some(existing) = state.sightings.iter_mut().find(|sighting| {
                sighting.family == key.0
                    && sighting.harness == key.1
                    && sighting.session_id == key.2
            })
        {
            existing.store_path = path.display().to_string();
            existing.scan_root = ForeignScanRoot::Home;
        }
        return Ok(());
    }
    if state.sightings.len() == MAX_FOREIGN_SIGHTINGS {
        state.exhausted = Some(ForeignScanUnavailableReason::SightingLimit);
        return Err(ForeignScanUnavailableReason::SightingLimit);
    }
    state.sighting_keys.insert(key);
    state.sightings.push(ForeignSighting {
        harness: declaration.subject_harness.clone(),
        family: declaration.subject_family.clone(),
        session_id: session_id.to_string(),
        store_path: path.display().to_string(),
        directory: directory.to_string(),
        first_seen: now_rfc3339(),
        scan_root: declaration.scan.root,
    });
    Ok(())
}

fn str_field_ref<'a>(record: &'a serde_json::Value, path: &str) -> Option<&'a str> {
    dotted_json(record, path).and_then(|value| value.as_str())
}

/// Refuses to arm a harness home whose own store already holds a session whose
/// directory is a tracked workspace: splitting a run across two homes must not cost only a
/// Flag.
///
/// Two questions, cheapest first.
///
/// 1. **Does the registry already have somebody in this home?** [`Registry::in_harness_home`]
///    answers without reading a byte of anyone's transcript. An entry whose `harness_home`
///    is `None` has not been placed and occupies nothing, so it matches no scan.
/// 2. **Does the home's own store already hold a session working in a tracked workspace?**
///    The connector's stores that declare a `file_first_record` partition are exactly the
///    ones whose first record decides which session a file belongs to (Codex's per-thread
///    rollout), so the deciding record is the one to read — and it is read the same way the
///    partitioner reads it, against the same manifest fields, so the scan and the capture
///    cannot disagree about what a file says.
///
/// The date components are rendered as `*` rather than today's: the question is whether the
/// home holds a tracked session at all, not whether it holds one written this morning. The
/// session is named from the connector's own [`ForeignScan`] entry (`session_id_field`),
/// which reads the same `session_meta` record the partition decides on.
///
/// `arming` is the entry being armed, and it is excluded from question 1. A daemon places an
/// entry in its harness home when it creates the workspace and arms it afterwards (control
/// API: `POST /v0/workspaces`, then `POST /v0/entries/{id}/arm`), so by the time this runs
/// the registry already holds the candidate itself in the home it is about to take. Without
/// the exclusion every entry occupies its own home against itself and no entry can ever
/// arm — and the refusal, which is about a run split across two homes, would name the run
/// it is refusing to start.
pub fn pre_arm_scan(
    manifest: &ConnectorManifest,
    vars: &PathVars,
    registry: &Registry,
    harness_home: &Path,
    arming: &str,
) -> Result<(), DaemonError> {
    if let Some(occupant) = registry
        .in_harness_home(harness_home)?
        .iter()
        .find(|e| e.entry_id != arming)
    {
        return Err(DaemonError::Refused(format!(
            "{}: entry {} is already placed in the harness home {}, and a home holds one \
             run: arming a second here would split one participant's session across two \
             homes",
            RefusalReason::HarnessHomeOccupied,
            occupant.entry_id,
            harness_home.display(),
        )));
    }

    let tracked: Vec<String> = registry
        .all()?
        .into_iter()
        .map(|e| e.workspace.display().to_string())
        .collect();
    if tracked.is_empty() {
        return Ok(());
    }
    // The session-id field this connector declares for its own sessions. `None` when the
    // connector declares no scan for itself, in which case the deciding record still names
    // the directory and the file names the session.
    let own_scan = manifest
        .foreign_scan
        .iter()
        .find(|s| s.harness == manifest.id);
    let undated = undated(vars);
    for store in &manifest.store {
        let Some(PartitionSpec::FileFirstRecord {
            select_path,
            select_value,
            value_path,
        }) = store.partition.as_ref()
        else {
            continue;
        };
        let pattern = resolve_template(&store.path_template, &undated)?;
        for path in expand_globs(&pattern) {
            let Some(record) = first_record(&path)? else {
                continue;
            };
            if str_field(&record, select_path).as_deref() != Some(select_value.as_str()) {
                continue;
            }
            let Some(directory) = str_field(&record, value_path) else {
                continue;
            };
            if !tracked.contains(&directory) {
                continue;
            }
            let session = own_scan
                .and_then(|s| str_field(&record, &s.session_id_field))
                .unwrap_or_else(|| file_stem(&path));
            return Err(home_occupied(harness_home, &session, &store.name, &path));
        }
    }
    Ok(())
}

/// One sentence for an occupied home, so the two stores that can raise it raise the same one.
fn home_occupied(harness_home: &Path, session: &str, store: &str, path: &Path) -> DaemonError {
    DaemonError::Refused(format!(
        "{}: the harness home {} already holds session {session}, whose working directory \
         is a workspace this daemon tracks (store `{store}`, {}); a run split across two \
         homes is refused before it is armed, not flagged after it ends",
        RefusalReason::HarnessHomeOccupied,
        harness_home.display(),
        path.display(),
    ))
}

/// The same path values with the three date components wildcarded and the session unbound,
/// so a glob over a date-partitioned tree matches every day rather than today.
fn undated(vars: &PathVars) -> PathVars {
    PathVars {
        session_id: None,
        date: ("*".to_string(), "*".to_string(), "*".to_string()),
        ..vars.clone()
    }
}

/// The first line of a file, parsed as JSON, bounded by [`MAX_LINE_BYTES`].
///
/// Bounded memory is a constraint on the whole daemon and this is somebody else's file: at
/// most one line's worth is read, the file is never held, and a first line that is not JSON
/// — a SQLite header, a partial write — is `None` rather than an error. A file that vanished
/// between the glob and the open is `None` too; a harness rotates its own transcripts.
fn first_record(path: &Path) -> Result<Option<serde_json::Value>, DaemonError> {
    let file = match open_regular_source(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(DaemonError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    };
    let mut head = Vec::new();
    file.take(MAX_LINE_BYTES)
        .read_to_end(&mut head)
        .map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let line = match head.iter().position(|b| *b == b'\n') {
        Some(end) => &head[..end],
        // No newline inside the cap: either the whole (short) file is one record, or the
        // first line is longer than the cap and there is no record here to read.
        None if (head.len() as u64) < MAX_LINE_BYTES => &head[..],
        None => return Ok(None),
    };
    Ok(serde_json::from_slice(line).ok())
}

/// A dotted field as an owned string, through the tailer's own reader so the scan and the
/// capture resolve `payload.cwd` identically. A field that is absent or is not a string is
/// `None`: an identifier that is a number or an object is not one this daemon coerces.
fn str_field(record: &serde_json::Value, path: &str) -> Option<String> {
    dotted_json(record, path)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// The file's name without its extension — the last resort for naming a session, used only
/// when the connector declares no `foreign_scan` entry for its own harness.
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn regular_source_open_is_nonblocking_and_nofollow() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("source.jsonl");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        assert!(open_regular_source(&fifo).is_err());

        std::fs::remove_file(&fifo).unwrap();
        let regular = dir.path().join("regular.jsonl");
        std::fs::write(&regular, b"{}\n").unwrap();
        std::os::unix::fs::symlink(&regular, &fifo).unwrap();
        assert!(open_regular_source(&fifo).is_err());
    }

    fn declaration() -> TrustedForeignScan {
        TrustedForeignScan {
            declaring_connector: "owner".into(),
            declaring_manifest_sha256: "a".repeat(64),
            scan_index: 0,
            subject_family: "cli".into(),
            subject_harness: "target".into(),
            scan: ForeignScan {
                harness: "target".into(),
                path_glob: "{home}/sessions/*.jsonl".into(),
                directory_field: "cwd".into(),
                session_id_field: "id".into(),
                reader: ForeignScanReader::FileFirstRecord {},
                root: ForeignScanRoot::Home,
            },
        }
    }

    #[test]
    fn declaration_and_global_identity_budgets_are_distinct() {
        let mut state = SweepState::default();
        let mut declaration_bytes = MAX_DECLARATION_IDENTITY_BYTES - 1;
        assert_eq!(
            reserve_identity(1, 1, &mut declaration_bytes, &mut state),
            Err(ForeignScanUnavailableReason::RecordLimit)
        );
        assert_eq!(state.exhausted, None, "another declaration may still run");

        let mut state = SweepState {
            identity_bytes: MAX_SWEEP_IDENTITY_BYTES - 1,
            ..SweepState::default()
        };
        let mut declaration_bytes = 0;
        assert_eq!(
            reserve_identity(1, 1, &mut declaration_bytes, &mut state),
            Err(ForeignScanUnavailableReason::RecordLimit)
        );
        assert_eq!(
            state.exhausted,
            Some(ForeignScanUnavailableReason::RecordLimit),
            "the rest of this sweep is explicitly unavailable"
        );
    }

    #[test]
    fn sighting_budget_is_global_and_duplicate_identities_are_free() {
        let declaration = declaration();
        let mut state = SweepState::default();
        retain_sighting(
            Path::new("/store"),
            &declaration,
            "same-session",
            "/workspace",
            &mut state,
        )
        .unwrap();
        retain_sighting(
            Path::new("/another-store"),
            &declaration,
            "same-session",
            "/workspace",
            &mut state,
        )
        .unwrap();
        assert_eq!(state.sightings.len(), 1);

        state
            .sightings
            .resize(MAX_FOREIGN_SIGHTINGS, state.sightings[0].clone());
        assert_eq!(
            retain_sighting(
                Path::new("/store"),
                &declaration,
                "one-too-many",
                "/workspace",
                &mut state,
            ),
            Err(ForeignScanUnavailableReason::SightingLimit)
        );
        assert_eq!(
            state.exhausted,
            Some(ForeignScanUnavailableReason::SightingLimit)
        );
    }

    #[test]
    fn participant_home_duplicate_upgrades_provenance_and_store_path() {
        let mut config_declaration = declaration();
        config_declaration.scan.root = ForeignScanRoot::OwnConfigDir;
        let mut state = SweepState::default();
        retain_sighting(
            Path::new("/config-store/session.jsonl"),
            &config_declaration,
            "same-session",
            "/workspace",
            &mut state,
        )
        .unwrap();

        retain_sighting(
            Path::new("/participant-home/session.jsonl"),
            &declaration(),
            "same-session",
            "/workspace",
            &mut state,
        )
        .unwrap();

        assert_eq!(state.sightings.len(), 1);
        assert_eq!(
            state.sightings[0].scan_root,
            ForeignScanRoot::Home,
            "participant-home provenance wins conservatively"
        );
        assert_eq!(
            state.sightings[0].store_path,
            "/participant-home/session.jsonl"
        );
    }
}
