//! The participant's own self-check over the workspace, and where the daemon put the trace.
//!
//! # The daemon's manifest is the record; this is a mirror held up beside it
//!
//! The seal is the **daemon's**: `POST /v0/entries/{id}/seal` finishes the entry and
//! terminates the harness tree per `kill.strategy`. Nothing in this module can change what
//! the daemon records, override it, or stand in for it. What it can do is take the same walk
//! with the same exclusions, from this side, and **print the differences** against a sealed
//! manifest. A difference is information — a file written between the two walks, a symlink
//! the two sides read differently — and never a refusal.
//!
//! # One exclusion function, and this module does not define a second
//!
//! [`goalcup_protocol::seal::is_excluded`] is the single definition, and the verifier, the
//! daemon and this file all call it: separate definitions would come apart, and a
//! participant would see a file in the verdict that their own tools had never mentioned. The
//! semantics are **workspace-root-anchored**: a nested `.git` is the agent's work unless a
//! connector writes `**/.git` and means it.
//!
//! # The trace directory is read here and written nowhere
//!
//! `<artifact_root>/trace` is the daemon's `TraceExport`'s, start to finish. [`trace_dir`]
//! names it so `goalcup seal` can tell the participant where their entry is and what to run
//! over it, and that is the whole of this CLI's half of exporting. There is no `goalcup
//! export` for the same reason: a second writer into that directory is a second author of a
//! document whose whole value is that one process wrote it.
//!
//! # Two of the five fields are arguments rather than inventions
//!
//! A [`SealManifest`] has five fields and a walk can discover three of them. `entry_id` comes
//! from the entry cache — the record `goalcup start` wrote — and `sealed_at` is this
//! machine's clock at the moment of **this** scan, deliberately not the daemon's `sealed_at`.
//! The two documents are put side by side, and a local scan that copied the daemon's
//! timestamp would be claiming to have been taken at a moment it was not.

use crate::CliError;
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::seal::{SealManifest, SealedFile, is_excluded};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The directory `TraceExport` owns under an artifact root.
///
/// The daemon spells it `artifact_dir.join("trace")`; this is the reading on this side and it
/// is read-only.
pub const TRACE_DIR_NAME: &str = "trace";

/// The digest recorded for an entry whose bytes could not be read: a broken symlink, a
/// socket, a file whose permissions changed under the walk.
///
/// Sixty-four zeros, the same sentinel `goalcup_daemon::workspace::ABSENT` uses. Spelled
/// again rather than imported because this crate does not depend on the daemon and must not
/// start; the two are compared only as strings, and a manifest that dropped an unreadable
/// entry instead would be quieter than the workspace it describes.
pub const ABSENT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// How many lines of one kind a report prints before it says how many more there were.
///
/// A self-check over a workspace with five thousand files in it is still a self-check, and a
/// terminal that scrolled all five thousand of them past the participant has told them
/// nothing. The count is always exact; the listing is what is bounded.
///
/// Per kind, and not over the report as a whole: a difference report holds exclusion lines
/// and file lines, one connector that changed its `workspace_exclusions[]` can produce more
/// exclusion lines than the bound, and a single bound over the concatenation would spend the
/// whole listing on them and print not one of the files. Each kind therefore gets its own
/// bound and its own "… and N more".
pub const MAX_LISTED: usize = 20;

/// How much of a file is held in memory while its digest is computed.
///
/// The same 64 KiB `goalcup_daemon::workspace` reads in, because the two walks must agree
/// on the digest and the only way to be sure of that is to compute it the same way.
const HASH_CHUNK: usize = 64 * 1024;

/// Where the daemon's `TraceExport` put this entry's trace: `<artifact_root>/trace`.
///
/// Read and printed. Nothing in this crate creates it, adds to it or removes from it.
pub fn trace_dir(artifact_root: &Path) -> PathBuf {
    artifact_root.join(TRACE_DIR_NAME)
}

/// This machine's clock, in the one spelling this tree writes a time in.
///
/// The CLI's own scan time and never the daemon's — see the module header. A clock somehow
/// before the epoch reads as the epoch rather than panicking: nothing downstream would
/// believe either number, and a self-check is not worth a panic.
pub fn scanned_at_now() -> String {
    goalcup_protocol::control::format_z(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// The exclusion union the seal applies: the connector's `workspace_exclusions[]` plus every
/// workspace-scoped file the tracker itself installed.
///
/// The mirror of the daemon's V1 `exclusion_union` (removed by OS-31; a capture entry's seal
/// walk excludes nothing). `tracker_files` is
/// the daemon's own list, carried to this process on the entry cache's `workspace_relative`
/// — not a second derivation from anything: a list derived twice is a list that comes apart.
///
/// Sorted and deduplicated, so two connectors that name `.git` twice produce one pattern and
/// `SealManifest.excluded` reads the same on two machines.
pub fn exclusion_set(m: &ConnectorManifest, tracker_files: &[String]) -> Vec<String> {
    let mut out: Vec<String> = m.workspace_exclusions.clone();
    out.extend(tracker_files.iter().cloned());
    out.sort();
    out.dedup();
    out
}

/// Walk `ws` under `exclusions` and answer the manifest this machine would have sealed.
///
/// `entry_id` and `sealed_at` are given rather than discovered — the module header says why.
/// The walk mirrors `goalcup_daemon::workspace::list_workspace` so the two documents are
/// comparable at all: an excluded name is **pruned** (its subtree is never entered),
/// `SealManifest.excluded` carries the patterns that were applied rather than a subset of the
/// paths they matched, and the result is sorted so a diff is stable.
///
/// # A symlink is read exactly the way the daemon reads one
///
/// `file_type()` comes from the directory read and follows nothing, so a link pointing at an
/// ancestor cannot make this walk loop and a link is never mistaken for a directory to
/// descend into. What is then done with it is [`measure`], the same as for any other
/// non-directory: the target is opened and its bytes are hashed, and a link that dangles is
/// listed with [`ABSENT`] — which is what `goalcup_daemon::workspace::measure` does, clause
/// for clause.
///
/// An empty workspace is a manifest with no files in it, not an error.
pub fn scan_workspace(
    ws: &Path,
    exclusions: &[String],
    entry_id: &str,
    sealed_at: &str,
) -> Result<SealManifest, CliError> {
    let mut files = Vec::new();
    let mut stack = vec![ws.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| CliError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| CliError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            let rel = relative_to(ws, &path);
            if is_excluded(&rel, exclusions) {
                continue;
            }
            let kind = entry.file_type().map_err(|source| CliError::Io {
                path: path.display().to_string(),
                source,
            })?;
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let (bytes, sha256) = measure(&path);
            files.push(SealedFile {
                path: rel,
                bytes,
                sha256,
            });
        }
    }
    Ok(SealManifest {
        entry_id: entry_id.to_string(),
        sealed_at: sealed_at.to_string(),
        workspace: ws.display().to_string(),
        files,
        excluded: exclusions.to_vec(),
    }
    .sorted())
}

/// A workspace-relative path with forward slashes — the spelling `is_excluded` is written
/// against and the one `SealedFile.path` carries.
fn relative_to(ws: &Path, path: &Path) -> String {
    path.strip_prefix(ws)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Length and digest of one file, streamed in [`HASH_CHUNK`] blocks.
///
/// The mirror of `goalcup_daemon::workspace::measure`, clause for clause, because the two
/// numbers are compared: the same chunk size, the same `File::open` (which follows a
/// symlink to its target), the same [`ABSENT`] on a failed open, and the same
/// bytes-so-far-with-[`ABSENT`] on a read that fails part way through.
///
/// A workspace is a participant's own directory and may hold a multi-gigabyte artefact, so
/// the file is streamed: memory here is bounded by this constant no matter what the walk
/// finds.
///
/// A file that cannot be read is still listed, with [`ABSENT`]: dropping it would make this
/// scan quieter than the workspace it describes, which is the one thing a self-check must
/// never be.
fn measure(path: &Path) -> (u64, String) {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return (0, ABSENT.to_string());
    };
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; HASH_CHUNK];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                total += n as u64;
            }
            Err(_) => return (total, ABSENT.to_string()),
        }
    }
    (total, hex_of(hasher.finalize().as_slice()))
}

/// Lowercase hex, spelled here so this crate declares no `hex` dependency for one line.
fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a `String` cannot fail. The result is dropped rather than unwrapped
        // because nothing in `src/` panics, and there is no error here to report.
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The three kinds of disagreement between two walks, kept apart rather than concatenated.
///
/// Apart, because a report prints a bounded listing of each (see [`MAX_LISTED`]): two
/// hundred exclusion lines from one edited connector manifest would otherwise spend the
/// whole listing and print not one of the files, which is a self-check that reported
/// nothing while looking like it reported everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Differences {
    /// The two documents name different workspaces. At most one line, and it explains every
    /// line below it.
    pub workspace: Option<String>,
    /// A pattern one side applied and the other did not.
    pub exclusions: Vec<String>,
    /// A file one side found, or found differently.
    pub files: Vec<String>,
}

impl Differences {
    /// How many disagreements there are, all kinds counted.
    pub fn len(&self) -> usize {
        usize::from(self.workspace.is_some()) + self.exclusions.len() + self.files.len()
    }

    /// The two walks agree.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every line, in reading order: the workspace, then the exclusions, then the files.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.len());
        out.extend(self.workspace.clone());
        out.extend(self.exclusions.iter().cloned());
        out.extend(self.files.iter().cloned());
        out
    }
}

/// Every way `local` and `sealed` disagree, one line each, naming the path and both sides.
///
/// An empty answer is the two walks agreeing. The order is: the workspace they each say they
/// walked, then the exclusion sets they each applied, then the files — because a disagreement
/// about the first two explains every disagreement below it, and a report that opened with
/// four hundred file lines would bury the one line that said why.
///
/// Each line names which side holds what, so nobody has to remember which document is which:
/// "this scan" is the participant's own, "the tracker's seal" is the record.
///
/// # The join is a map and not a search
///
/// Both file lists are read into one [`BTreeMap`] keyed by path: a `find` inside a loop over
/// each side is quadratic, and a fifty-thousand-file workspace — a `node_modules` no
/// connector excluded is exactly that — would make billions of string comparisons and look
/// like a self-check that had hung. The map also gives the ordering for free, since a
/// `BTreeMap` is walked in key order and the paths are the keys.
pub fn compare(local: &SealManifest, sealed: &SealManifest) -> Vec<String> {
    compare_parts(local, sealed).lines()
}

/// [`compare`] with the kinds kept apart, for a report that bounds each one separately.
pub fn compare_parts(local: &SealManifest, sealed: &SealManifest) -> Differences {
    let mut out = Differences::default();
    if local.workspace != sealed.workspace {
        out.workspace = Some(format!(
            "workspace     this scan walked {}; the tracker's seal names {}",
            local.workspace, sealed.workspace
        ));
    }
    for pattern in &local.excluded {
        if !sealed.excluded.contains(pattern) {
            out.exclusions.push(format!(
                "exclusion     `{pattern}` was applied by this scan and is not in the \
                 tracker's seal"
            ));
        }
    }
    for pattern in &sealed.excluded {
        if !local.excluded.contains(pattern) {
            out.exclusions.push(format!(
                "exclusion     `{pattern}` is in the tracker's seal and was not applied by \
                 this scan"
            ));
        }
    }
    // path -> (this scan's reading, the seal's reading). One pass over each list, and the
    // walk below is in path order because the keys are the paths.
    let mut both: BTreeMap<&str, (Option<&SealedFile>, Option<&SealedFile>)> = BTreeMap::new();
    for file in &local.files {
        both.entry(file.path.as_str()).or_default().0 = Some(file);
    }
    for file in &sealed.files {
        both.entry(file.path.as_str()).or_default().1 = Some(file);
    }
    for (path, sides) in both {
        match sides {
            (Some(here), None) => out.files.push(format!(
                "only here     {path} — this scan found it ({}); the tracker's seal does not \
                 list it",
                size(here)
            )),
            (None, Some(there)) => out.files.push(format!(
                "only sealed   {path} — the tracker's seal lists it ({}); this scan did not \
                 find it",
                size(there)
            )),
            (Some(here), Some(there))
                if here.bytes != there.bytes || here.sha256 != there.sha256 =>
            {
                out.files.push(format!(
                    "differs       {path} — this scan {}, the tracker's seal {}",
                    size(here),
                    size(there)
                ))
            }
            _ => {}
        }
    }
    out
}

/// One file's length and the head of its digest, which is what a reader compares by eye.
///
/// Twelve characters: enough that two different files never read the same in one report, and
/// short enough that two of them fit on a line beside the path they are about.
fn size(f: &SealedFile) -> String {
    let head: String = f.sha256.chars().take(12).collect();
    format!("{} bytes, sha {head}", f.bytes)
}
