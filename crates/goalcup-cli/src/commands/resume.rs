//! `goalcup resume` — catch the tail up, check the lock, relaunch with the same pins.
//!
//! A participant who closed their harness resumes the **same** session: the same workspace,
//! the same artifact root, the same session id, the same reserved
//! port. Anything else is a second run wearing the first one's entry id.
//!
//! # Five normal-resume steps, in this order, and the order is the feature
//!
//! 1. **The entry cache** ([`crate::entries::read_entry`]). It holds what `goalcup start`
//!    pinned: the connector, the harness home, the workspace, the session id and the pid of
//!    the last process this CLI registered. A machine with no record of the entry is rung 1
//!    — `read_entry`'s own refusal — and nothing is asked of the daemon.
//! 2. **[`check_locks`], before the POST.** This is the only place the participant is told
//!    *which file to clear and which pid, if any, is holding it*, and it has to run first
//!    because of what the daemon does instead.
//! 3. **`client.resume`** through [`retry_busy`], which is the first call this command makes
//!    on the wire (D9: the gap is closed before anything new is appended, which is what
//!    lets the verifier classify it as clean).
//! 4. **The relaunch**, from the manifest's `[resume]` recipe and
//!    [`crate::vars::from_resume`] — the same eight variables the original launch rendered.
//! 5. **`register_process`**, immediately, and the new pid written back to the cache.
//!
//! # The daemon is the authority and it refuses first; this command translates
//!
//! `resume_entry` refuses a **sealed** entry with `conflict`/409 before anything else, then
//! calls `free_of_held_locks` and returns an **error** rather than data when a lock is held,
//! and `ResumeInfo` carries no lock findings. A CLI that posted first would learn only
//! "conflict" and, per the ladder, would tell the participant nothing about which process is
//! holding their session open.
//!
//! So [`check_locks`] runs against the entry cache before the POST, and against a real
//! daemon it is the only half that can name a pid. Its inputs therefore cannot come from a
//! `ResumeInfo` that has not been fetched yet — which is what [`LockRoots`] is for, and why
//! it can be built from either side. Both readings are kept and both must agree: the record
//! is what `goalcup start` wrote, the answer is what the daemon just said, and a relaunch
//! into directories the lock check never looked at is not a checked relaunch.
//!
//! # A lock that is on disk is a refusal, held or not
//!
//! A `codex resume` that cannot take the thread-writer lock
//! **silently becomes a new thread with no parent reference**. A resume that proceeds into
//! that state costs the participant their continuity and produces a trace the verifier flags
//! as a second root agent.
//!
//! The daemon's `free_of_held_locks` asks only whether anything is at the path and refuses
//! whether or not a holder is alive, so this side refuses the same way: a warning that said
//! the relaunch was proceeding would be followed by a `409` from the other side, under advice
//! ("stop whatever is holding it") about a process that had already died. **Every declared
//! lock path that exists is [`CliError::Gate`], exit 4, before the POST.** The difference a
//! holder makes is only in what the participant is asked to do: stop the process that is
//! holding it, or — when nothing holds it, or nothing in the file says who does — run the
//! `rm` the refusal prints.
//!
//! **That `rm` is printed and never run.** The lock file is the harness's, this tracker did
//! not create it, and a CLI that deleted one would be acting on its own guess that the
//! holder it could not find is really gone.
//!
//! The rule is the manifest's and not one connector's. `codex-cli` declares a lock path and
//! `claude-code-cli` declares none. Nothing in
//! `manifest::ConnectorManifest` grades a `lock_paths[]` entry, so a connector that declares
//! one gets the rule the sentence above states, and the daemon remains the authority.
//!
//! # `check_locks` takes no environment, and that is the point
//!
//! A `lock_paths[]` entry is written in `manifest::PATH_TOKENS` syntax, and the tokens a
//! lock path may use resolve from [`LockRoots`] and from nothing else. Reading an
//! environment variable here would let a participant move the lock directory out from under
//! the check by exporting one — which is the same reason `manifest.rs` says no manifest ever
//! names an environment variable.
//!
//! # `--print-launch` only previews the line
//!
//! It asks the dedicated read-only preview endpoint for the launch pins, then stops.
//! It does not catch up capture, clear a registered handle, seed the run home, spawn or
//! register a harness, rewrite the local cache, or open the status page. The existing entry
//! remains intact for a later normal `goalcup resume`.

use crate::CliError;
use crate::connector::Connector;
use crate::entries::{
    EntryRecord, ProcessCleanupOrigin, ResumeRecoveryIntent, read_entry, write_entry,
};
use crate::launch::{
    LaunchKind, handle, plan, process_env, quote, scrubbed_env, shell_line, spawn,
};
use crate::preflight::{Level, server_port_gate};
use crate::retry::{retry_busy, retry_busy_entry, retry_busy_process_classified};
use goalcup_protocol::control::{
    ControlError, DaemonClient, ProcessHandoffCancelRequest, ResumeInfo, ResumePreviewInfo,
    ResumeRequest,
};
use goalcup_protocol::manifest::{ConnectorManifest, PATH_TOKENS};
use goalcup_protocol::process::{
    ProcessInstanceStatus, ProcessRegistration, is_alive, process_handle_instance_status,
};
use std::path::{Path, PathBuf};

/// D8, to the participant's terminal and never into the session.
///
/// The tracker cannot type into the harness and must not: the next human message is the
/// participant's own, and the rules say what it has to be. Printed on every relaunch,
/// because a resume whose first message was anything else is a resume the verifier reads as
/// a new instruction rather than as a continuation.
pub const KEEP_GOING: &str = "Your next message in the harness must be exactly \
     `keep going` — nothing else, and nothing before it. That is what makes this the same \
     run rather than a new instruction.";

/// The measured sentence, quoted where a participant is being asked to stop something.
///
/// A constant because the refusal below is the only place this tracker ever asks someone to
/// end a process, and the reason has to be the measured one rather than a paraphrase of it.
pub const M0_LOCK_SENTENCE: &str = "a `codex resume` that cannot take the \
     thread-writer lock silently becomes a new thread with no parent reference";

/// Private, per-entry lock files held for the duration of a non-preview `resume`.
///
/// The lock file is intentionally retained after every command: the OS lock, not pathname
/// absence, decides ownership, so a process crash releases it without a stale-file recovery
/// race.  The adjacent cache marker says whether a successor may repair an open daemon
/// permit after that release.
const RESUME_LOCKS_DIR: &str = "resume-locks";

/// An OS-held per-entry lock.  `std::fs::File::try_lock` maps to the platform's advisory
/// locking primitive and releases when this value drops, including after a crash.
struct LocalResumeLock {
    _file: std::fs::File,
    _path: PathBuf,
}

impl LocalResumeLock {
    fn acquire(cfg: &Path, entry_id: &str) -> Result<LocalResumeLock, CliError> {
        // The entry cache has the same path-component rule.  Repeat it at this new path
        // boundary rather than letting a daemon-provided id name a lock outside cfg.
        if entry_id.is_empty()
            || matches!(entry_id, "." | "..")
            || entry_id.contains('/')
            || entry_id.contains('\\')
            || entry_id.contains('\0')
        {
            return Err(CliError::Refused(format!(
                "`{entry_id}` does not name one local resume lock; nothing was resumed"
            )));
        }
        let dir = cfg.join(RESUME_LOCKS_DIR);
        if !dir.exists() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&dir).map_err(|source| CliError::Io {
                path: dir.display().to_string(),
                source,
            })?;
        }
        let path = dir.join(format!("{entry_id}.lock"));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            // A retained lock file deliberately holds no payload: a later resume opens the
            // same inode and asks the OS whether another process still holds it.
            .truncate(false)
            .open(&path)
            .map_err(|source| CliError::Io {
                path: path.display().to_string(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(LocalResumeLock {
                _file: file,
                _path: path,
            }),
            Err(std::fs::TryLockError::WouldBlock) => Err(CliError::Gate(format!(
                "another local `goalcup resume {entry_id}` is already recovering or \
                     relaunching this entry. Wait for that command to finish; do not start a \
                     second resume that could replace its daemon permit."
            ))),
            Err(std::fs::TryLockError::Error(source)) => Err(CliError::Io {
                path: path.display().to_string(),
                source,
            }),
        }
    }
}

/// Rewrite this machine's record after preview proves no permit is open.  The daemon remains
/// the source of truth; this only removes the local permission to cancel an old `open_resume`
/// permit once it cannot identify a live predecessor any longer.
fn clear_resume_recovery(cfg: &Path, record: &EntryRecord) -> Result<EntryRecord, CliError> {
    let repaired = EntryRecord {
        resume_recovery: None,
        ..record.clone()
    };
    write_entry(cfg, &repaired)?;
    Ok(repaired)
}

/// Finish a definitively rejected cached handoff without ever making its body replayable
/// again. Cancellation settles the daemon's issued resume generation. A successor never
/// signals a cached numeric pid: it clears the marker only when the retained OS creation
/// identity proves the original child is gone, and otherwise blocks relaunch.
fn finish_pending_process_cleanup(
    client: &dyn DaemonClient,
    cfg: &Path,
    record: &EntryRecord,
    entry_id: &str,
    registration: &ProcessRegistration,
    out: &mut dyn std::io::Write,
) -> Result<EntryRecord, CliError> {
    let origin = record
        .pending_process_cleanup_origin
        .unwrap_or(ProcessCleanupOrigin::Resume);
    let process_generation = if origin == ProcessCleanupOrigin::Resume {
        let cancel_request = ProcessHandoffCancelRequest {
            expected_generation: registration.generation,
            request_digest: Some(registration.canonical_handle_digest(entry_id)),
        };
        match retry_busy_entry(|| client.cancel_process_handoff(entry_id, &cancel_request)) {
            Ok(answer) => answer.process_generation,
            Err(error) => {
                let _ = say(
                    out,
                    &format!(
                        "  (cleanup of the rejected child and resume generation {} was not confirmed: {error})",
                        registration.generation
                    ),
                );
                return Err(CliError::Control(error));
            }
        }
    } else {
        record.process_generation
    };
    if let Some(unresolved) = &record.pending_unresolved_native_cleanup {
        let progressed = EntryRecord {
            process_generation,
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: Some(registration.clone()),
            pending_process_cleanup_origin: Some(origin),
            resume_recovery: None,
            ..record.clone()
        };
        write_entry(cfg, &progressed)?;
        return Err(CliError::Refused(format!(
            "the rejected group-less process may still contain an undiscovered native process `{}`; its cached registration will not be replayed. Exit that original harness in its terminal, then run `goalcup resume {entry_id}` again",
            unresolved.native_binary
        )));
    }
    let status = process_handle_instance_status(&registration.handle);
    if status != ProcessInstanceStatus::GoneOrReused {
        let progressed = EntryRecord {
            process_generation,
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: Some(registration.clone()),
            pending_process_cleanup_origin: Some(origin),
            resume_recovery: None,
            ..record.clone()
        };
        write_entry(cfg, &progressed)?;
        let why = if status == ProcessInstanceStatus::Same {
            "still contains the same live process authority"
        } else {
            "could not be proven absent because a live process authority's creation identity is unavailable"
        };
        return Err(CliError::Refused(format!(
            "the rejected process tree rooted at pid {} {why}; its cached registration will not be replayed. Exit that original harness in its terminal, then run `goalcup resume {entry_id}` again",
            registration.handle.pid
        )));
    }
    let repaired = EntryRecord {
        process_generation,
        pending_process_registration: None,
        pending_process_registration_origin: None,
        pending_process_cleanup: None,
        pending_process_cleanup_origin: None,
        pending_unresolved_native_cleanup: None,
        resume_recovery: None,
        ..record.clone()
    };
    write_entry(cfg, &repaired)?;
    Ok(repaired)
}

/// Return an issued resume permit after a local failure and repair the cache with the
/// daemon's monotonic generation.  This is best-effort reporting around a durable recovery
/// transition: the original local error remains the command result, while a refusal leaves
/// the entry fail-closed with its prior termination authority intact.
fn cancel_issued_resume(
    client: &dyn DaemonClient,
    cfg: &Path,
    record: &EntryRecord,
    entry_id: &str,
    generation: u64,
    request_digest: Option<String>,
    out: &mut dyn std::io::Write,
) -> bool {
    let request = ProcessHandoffCancelRequest {
        expected_generation: generation,
        request_digest,
    };
    match retry_busy_entry(|| client.cancel_process_handoff(entry_id, &request)) {
        Ok(answer) => {
            let repaired = EntryRecord {
                process_generation: answer.process_generation,
                pending_process_registration: None,
                pending_process_registration_origin: None,
                resume_recovery: None,
                ..record.clone()
            };
            if let Err(error) = write_entry(cfg, &repaired) {
                let _ = say(
                    out,
                    &format!(
                        "  (the resume permit was cancelled but this machine's generation note could not be updated: {error})"
                    ),
                );
            }
            true
        }
        Err(error) => {
            let _ = say(
                out,
                &format!(
                    "  (the daemon could not confirm cancellation of resume generation {generation}: {error}; the daemon's existing handoff state was left unchanged)"
                ),
            );
            false
        }
    }
}

/// One `lock_paths[]` file that is on disk, and what is known about who holds it.
///
/// `holder_pid` is read out of **that file's own content** and is `None` whenever the
/// content is not a pid — see [`pid_in`]. `holder_alive` is [`is_alive`] on a pid that was
/// read, and is false when none was.
///
/// It is not `EntryRecord::last_pid`, the pid this CLI last registered for the entry. A pid
/// the operating system has recycled is *alive* and belongs to something else, so the
/// refusal would tell the participant to kill an unrelated process; and a connector whose
/// `lock_paths[]` names a **directory** would get every file in it attributed to that one
/// pid, including files from other sessions. The cache's `last_pid` is not evidence about
/// who holds a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockFinding {
    pub path: String,
    pub holder_pid: Option<u32>,
    pub holder_alive: bool,
    /// What the process table calls the holder, where a pid was read out of the file and
    /// something is running under it. `None` whenever there is no pid, nothing is running,
    /// or `ps` could not be asked — and a refusal says which of those it is rather than
    /// printing an empty pair of quotation marks. See [`lock_refusal`].
    pub holder_command: Option<String>,
}

/// The most lock files one refusal lists.
///
/// A `lock_paths[]` entry may name a directory, and a harness home holding hundreds of
/// files would otherwise produce a refusal no
/// participant can read. The same bound the seal's difference report uses
/// ([`crate::seal::MAX_LISTED`]), for the same reason, and the count of what was not shown
/// is printed after it: a list that was cut says so.
pub const MAX_LISTED_LOCKS: usize = crate::seal::MAX_LISTED;

/// The tokens a `lock_paths[]` entry may use, plus the session id to render.
///
/// [`check_locks`] runs **before** the resume POST, so its inputs cannot come from a
/// `ResumeInfo` that has not been fetched yet. Both readings are kept and both must agree —
/// the entry cache is what `goalcup start` recorded, the `ResumeInfo` is what the daemon just
/// said.
///
/// The tokens resolve like this and no other way:
///
/// * `{config_dir}` → [`Self::harness_home`] — the tracker-created harness home, the
///   directory the launch pinned through `launch.home_env`, and the only place a tracked
///   session's locks can be. The shipped user of the token is `codex-cli`'s
///   `{config_dir}/thread-writer-locks/{session_id}.lock`.
/// * `{home}` → [`Self::harness_home`] as well, for the same reason: the CLI never looks at
///   the participant's real `$HOME`, and the daemon resolved these roots once.
/// * `{workspace}` → [`Self::workspace`].
/// * `{session_id}` → [`Self::session_id`]: `codex-cli` names the per-thread lock file rather
///   than the directory it sits in, because the directory and the `.coordination.lock` inside
///   it are made at startup and left behind, and reading either as a lock would refuse every
///   resume of a run that had ended properly. Any other name is refused — see
///   [`unresolvable`].
///
/// **The session id is not compared.** The cache holds the id the daemon minted at
/// `create_workspace`; `ResumeInfo.session_id` is the id the arming gate bound — "the session
/// the latest corroborating leg-1 record belongs to". Only `claude-code-cli` pins the minted
/// id at launch (`--session-id`); `codex-cli` pins none, so the harness mints its own and the
/// two ids differ on every Codex run. That is why `[resume]` renders `{session_id}` off
/// `ResumeInfo`, and comparing them would refuse every Codex resume at rung 1 with nothing
/// relaunched. The identity check is the **entry id** — which the daemon answered about, and
/// which `read_entry` already resolved — and the session is the daemon's to say.
///
/// There is **no `last_pid` here**: a pid a lock check has not read out of a lock file is not
/// evidence about that file (see [`LockFinding`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockRoots {
    pub harness_home: String,
    pub workspace: String,
    pub session_id: String,
}

/// Do these two spellings name the same directory?
///
/// String equality first, which is the answer when both readings come from the one canonical
/// spelling (D16), and no file system is touched. Otherwise both are resolved, and they
/// agree only if both resolve and resolve to the same real path — a cache can hold the requested
/// spelling of a workspace whose parent is a symlink (`/var` → `/private/var` is the default
/// shape on macOS), and that is the same directory under two names, not two directories.
///
/// A path that does not resolve — deleted, or never created — is **not** treated as equal to
/// anything. The refusal is the point of the check: a genuinely different path, and a
/// workspace that has gone away underneath a resume, both still stop the relaunch.
fn same_directory(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl LockRoots {
    /// What `goalcup start` wrote down on this machine.
    pub fn from_entry(rec: &EntryRecord) -> LockRoots {
        LockRoots {
            harness_home: rec.harness_home.clone(),
            workspace: rec.workspace.clone(),
            session_id: rec.session_id.clone(),
        }
    }

    /// What the daemon just said, for the agreement check.
    pub fn from_resume(info: &ResumeInfo) -> LockRoots {
        LockRoots {
            harness_home: info.harness_home.clone(),
            workspace: info.workspace.clone(),
            session_id: info.session_id.clone(),
        }
    }

    /// What the daemon's side-effect-free preview said, for the same agreement check.
    pub fn from_resume_preview(info: &ResumePreviewInfo) -> LockRoots {
        LockRoots {
            harness_home: info.harness_home.clone(),
            workspace: info.workspace.clone(),
            session_id: info.session_id.clone(),
        }
    }

    /// The first of the two directories the two readings disagree about, if any.
    ///
    /// The two are the whole of it: they decide *where* the lock check looked and *where*
    /// the relaunch will run, and nothing else here is a place. The session id is carried
    /// beside them and is not compared — [`LockRoots`] says why.
    fn disagreement(&self, other: &LockRoots) -> Option<&'static str> {
        if !same_directory(&self.harness_home, &other.harness_home) {
            return Some("harness home");
        }
        if !same_directory(&self.workspace, &other.workspace) {
            return Some("workspace");
        }
        // No session-id comparison: see [`LockRoots`]. The daemon's bound id is the answer
        // and this side's minted one is a starting point.
        None
    }

    /// One `lock_paths[]` template, rendered.
    ///
    /// **There is no `env` parameter, and there must not be one** — see the module header.
    /// A token outside the three is [`CliError::Internal`] naming the token and the
    /// connector, never a silently non-matching glob: a lock check that quietly matched
    /// nothing would report every run as free of locks, which is the failure the lock
    /// check exists to prevent.
    fn resolve(&self, template: &str, connector_id: &str) -> Result<String, CliError> {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            rest = &rest[open + 1..];
            let Some(close) = rest.find('}') else {
                // An unterminated placeholder is not a name — the same reading
                // `manifest::scan_braces` takes — so the rest of the template is a literal.
                out.push('{');
                break;
            };
            let token = &rest[..close];
            let value = match token {
                // Both mean the tracker-created harness home; the doc above says why.
                "config_dir" | "home" => &self.harness_home,
                "workspace" => &self.workspace,
                // The third of the three values [`LockRoots`] carries, and the one
                // `codex-cli`'s lock path names: the per-thread lock file.
                "session_id" => &self.session_id,
                _ => return Err(unresolvable(token, template, connector_id)),
            };
            out.push_str(value);
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// A `lock_paths[]` token no resume can fill, named with the connector that wrote it.
///
/// Rung 3 and not rung 1: a manifest naming a token this side cannot resolve is a connector
/// disagreeing with the tracker, which is not something a participant does differently next
/// time. The message says whether the token is a `PATH_TOKENS` name at all, because "that is
/// a typo" and "that one has no source in a resume" are two different fixes.
fn unresolvable(token: &str, template: &str, connector_id: &str) -> CliError {
    let known = match PATH_TOKENS.contains(&token) {
        true => {
            "it is a path token with no source in a resume: a resume knows the harness home, \
             the workspace and the session, and nothing else"
        }
        false => "it is not one of `manifest::PATH_TOKENS` at all",
    };
    CliError::Internal(format!(
        "`{connector_id}`'s lock path `{template}` names `{{{token}}}`, and {known}. Nothing \
         was checked and nothing was relaunched: a lock path this side cannot resolve would \
         match nothing and report every run as free of locks."
    ))
}

/// Every `lock_paths[]` file that is on disk right now, and who holds it.
///
/// A template ending in `/`, or naming a directory, is a directory **of** locks — which is
/// what both shipped users of the token are — and every name inside it is one finding. Any
/// other template is one path, and one finding if anything is at it.
///
/// `symlink_metadata` and not `exists`, for the reason the daemon's own `free_of_held_locks`
/// gives: the question is whether anything is at the path, and a dangling link left by a
/// killed harness is still a name a relaunch will trip over.
///
/// The findings are sorted by path. `read_dir` answers in whatever order the filesystem
/// enumerates, and a refusal that named two locks in a different order on two machines would
/// be a refusal nobody could diff.
///
/// **`Result` and not a bare `Vec`**: an unresolvable token is a `CliError::Internal` naming
/// the token and the connector, and a function that returned only findings would have
/// nowhere to put it but a finding — which is a refusal at exit 4 about a lock nobody is
/// holding.
pub fn check_locks(m: &ConnectorManifest, roots: &LockRoots) -> Result<Vec<LockFinding>, CliError> {
    let mut paths: Vec<String> = Vec::new();
    for template in &m.lock_paths {
        if paths.len() > MAX_LISTED_LOCKS {
            break;
        }
        let rendered = roots.resolve(template, &m.id)?;
        let path = Path::new(&rendered);
        // The one rule, and the daemon applies the same one:
        // [`goalcup_protocol::manifest::lock_path_is_held`]. A directory of locks that is
        // absent or empty holds nothing, so this function then has nothing to enumerate and
        // nothing to refuse.
        if !goalcup_protocol::manifest::lock_path_is_held(&rendered) {
            continue;
        }
        if rendered.ends_with('/') || path.is_dir() {
            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                // The helper answered "held" for a directory this process could not look
                // in, which is not the same answer as "no locks" and must never be
                // reported as one — see [`unreadable`]. (`NotFound` cannot reach here: an
                // absent directory is not held.)
                Err(e) => return Err(unreadable(&rendered, &m.id, &e)),
            };
            for entry in entries {
                match entry {
                    // The same entries the helper counted: anything that is not itself a
                    // directory, so a dangling symlink a killed harness left is listed and
                    // a nested directory is not mistaken for a lock file.
                    Ok(entry) if entry.path().is_dir() => continue,
                    Ok(entry) => paths.push(entry.path().display().to_string()),
                    Err(e) => return Err(unreadable(&rendered, &m.id, &e)),
                }
                // Bounded while enumerating and not while printing: the answer this
                // function gives is "is anything holding a lock", and one file past the
                // bound answers it exactly as ten thousand do. `+ 1` so the caller can say
                // that the list was cut.
                if paths.len() > MAX_LISTED_LOCKS {
                    break;
                }
            }
        } else {
            paths.push(rendered.clone());
        }
    }
    paths.sort();
    paths.dedup();
    // One `ps` snapshot for the whole report, and only where there is something to ask
    // about: the table is the same for every finding, and asking once per lock file would
    // run `ps` as many times as a harness home has files in it.
    let table = match paths.is_empty() {
        true => Vec::new(),
        false => goalcup_protocol::process::snapshot().unwrap_or_default(),
    };
    Ok(paths.iter().map(|p| finding(p, &table)).collect())
}

/// A lock directory this process could not enumerate.
///
/// [`CliError::Internal`], exit 3, and deliberately not a shrug: treating a permission error
/// the same as a directory that is not there would report **every** run as free of locks on a
/// machine where the lock directory had become unreadable — the silent-pass failure the lock
/// check exists to prevent, arrived at from the other direction. The daemon would then refuse
/// the POST on its own reading with nothing from this side to explain it.
fn unreadable(rendered: &str, connector_id: &str, e: &std::io::Error) -> CliError {
    CliError::Internal(format!(
        "`{connector_id}`'s lock directory `{rendered}` is there and could not be read: {e}. \
         Nothing was checked and nothing was relaunched: a lock check that could not look is \
         not a lock check that found nothing."
    ))
}

/// One lock file, and what is known about its holder.
///
/// `table` is one `ps` snapshot, taken once by the caller. The pid in a lock file is a
/// number the file's writer chose — see [`pid_in`] — so what the process table calls it is
/// carried beside it and the refusal prints both. An empty table (a platform with no `ps`,
/// or a `ps` that could not be run) simply yields no name, which is the honest answer.
fn finding(path: &str, table: &[goalcup_protocol::process::ProcInfo]) -> LockFinding {
    let holder_pid = pid_in(Path::new(path));
    let holder_alive = holder_pid.is_some_and(is_alive);
    let holder_command = match (holder_pid, holder_alive) {
        (Some(pid), true) => table
            .iter()
            .find(|row| row.pid == pid)
            .map(|row| row.comm.clone()),
        _ => None,
    };
    LockFinding {
        path: path.to_string(),
        holder_pid,
        holder_alive,
        holder_command,
    }
}

/// The pid a lock file names, when the whole of its content is one, and `None` otherwise.
///
/// **What the spike measured.** `spikes/codex-cli` records the shape of the only lock this
/// tracker has watched a harness take: `$CODEX_HOME/thread-writer-locks/<thread>.lock`, one
/// per thread, plus a directory-wide `.coordination.lock`. It records that orphaned native
/// processes went on **holding** those files after the wrapper was killed, and that session
/// death has to be inferred from "the lock file plus hook silence" — it does not record any
/// content at all, and the tracker never opened one to find out. So this parses the
/// narrowest thing that can be recognised without a contract: a file whose entire content,
/// trimmed, is a decimal pid. `.coordination.lock`, an empty file, JSON, a hostname, a
/// binary `flock` cookie — every one of them is `None`, which is the honest answer "this
/// file does not say who holds it", and the refusal then prints the `rm` instead of a `kill`.
///
/// The length cap is the same honesty: a pid is at most ten digits, so anything longer is
/// not a pid file and is not read into memory to prove it.
fn pid_in(path: &Path) -> Option<u32> {
    use std::io::Read as _;

    const LONGEST_PID_FILE: u64 = 32;
    let file = goalcup_platform::readcap::open_regular(path).ok()?;
    match file.metadata() {
        Ok(m) if m.is_file() && m.len() <= LONGEST_PID_FILE => {}
        _ => return None,
    }
    let mut text = String::new();
    let read = file
        .take(LONGEST_PID_FILE + 1)
        .read_to_string(&mut text)
        .ok()?;
    if read as u64 > LONGEST_PID_FILE {
        return None;
    }
    match text.trim().parse::<u32>() {
        // Pid 0 is no process on either platform this runs on; a file holding it says
        // nothing, so it is read as saying nothing.
        Ok(0) | Err(_) => None,
        Ok(pid) => Some(pid),
    }
}

/// What one `goalcup resume` was asked to do.
pub struct ResumeOptions {
    /// Print the line this relaunch would run, and stop there — nothing is relaunched,
    /// seeded or registered after it. See the module header.
    pub print_launch: bool,
    /// Hand the status URL to the participant's browser. `--no-open` clears it, which is the
    /// headless machine's path: the URL is printed either way.
    pub open_browser: bool,
}

/// The testable half: everything except resolving a real client and a real config dir.
///
/// `connectors`, `cfg` and `out` are parameters and not lookups for the reason the rest of
/// this crate takes them that way — a command that printed with `println!` could not be
/// asserted at all.
///
/// `cfg` carries the record the pre-POST lock check reads and the record the post-relaunch
/// `register_process` writes the new pid back to: the same file `status` and `seal` read,
/// and the same one `goalcup start` wrote.
pub fn resume_with(
    client: &dyn DaemonClient,
    connectors: &Path,
    cfg: &Path,
    entry_id: &str,
    opts: &ResumeOptions,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    // The window a Ctrl-C is this command's to act on is short — between the spawn and the
    // hand-off — and the flag it reads is process-global, so it is put back on the way out
    // whichever way this went. A guard and not a trailing call: see
    // [`crate::arming::clear_interrupt`].
    crate::arming::watch_for_interrupt();
    let _clear = crate::arming::clear_on_drop();

    // One normal resume owns this machine's per-entry critical section from its first cache
    // read until it hands the process over (or repairs a crash).  A preview remains read-only
    // and deliberately does not take it.
    let _local_resume_lock = (!opts.print_launch)
        .then(|| LocalResumeLock::acquire(cfg, entry_id))
        .transpose()?;

    // 1. What this machine pinned when the run was created.
    let mut record = read_entry(cfg, entry_id)?;
    if !opts.print_launch
        && record.pending_process_registration.is_none()
        && record.pending_process_cleanup.is_none()
        && let Some(unresolved) = &record.pending_unresolved_native_cleanup
    {
        return Err(CliError::Refused(format!(
            "a prior group-less launch may still contain an undiscovered native process `{}`; no exact process handle was persisted, so automatic cleanup or relaunch is unsafe. Exit that original harness in its terminal before manually repairing this entry",
            unresolved.native_binary
        )));
    }
    if !opts.print_launch
        && let Some(registration) = record.pending_process_cleanup.clone()
    {
        record =
            finish_pending_process_cleanup(client, cfg, &record, entry_id, &registration, out)?;
    }
    // A process registration is written to this cache before its first request byte.  Replay
    // it before *every* other non-print-resume action: the accepted child may legitimately
    // own a connector lock, and minting another resume permit before repairing this exact
    // request would strand its daemon authority.  This also recovers an initial launch whose
    // response was lost — receipt binding continues autonomously after the acknowledgement.
    if !opts.print_launch
        && let Some(registration) = record.pending_process_registration.clone()
    {
        // This is written explicitly. The fallback is for the short pre-release cache shape:
        // a resumed handoff always had a recovery intent, while an initial launch did not.
        let registration_origin = record.pending_process_registration_origin.unwrap_or(
            if record.resume_recovery.is_some() {
                ProcessCleanupOrigin::Resume
            } else {
                ProcessCleanupOrigin::Initial
            },
        );
        match retry_busy_process_classified(|| {
            DaemonClient::register_process_with_admission(client, entry_id, &registration)
        }) {
            Ok(()) => {
                let repaired = EntryRecord {
                    process_generation: registration.generation,
                    last_pid: Some(registration.handle.pid),
                    pending_process_registration: None,
                    pending_process_registration_origin: None,
                    pending_unresolved_native_cleanup: None,
                    resume_recovery: None,
                    ..record.clone()
                };
                write_entry(cfg, &repaired)?;
                say(
                    out,
                    &format!(
                        "entry {entry_id}: recovered the previously issued process handoff. \
                         The tracker owns that harness; inspect its status before requesting \
                         another resume."
                    ),
                )?;
                return Ok(());
            }
            Err(failure) if failure.may_have_been_admitted => {
                return Err(if failure.interrupted {
                    CliError::Refused(format!(
                        "`goalcup resume` was interrupted while the pending process handoff may already have been admitted: {}. The exact handoff remains pending; run resume again to recover it.",
                        failure.error
                    ))
                } else {
                    CliError::Control(failure.error)
                });
            }
            Err(failure) => {
                // Retire replay authority before any cleanup side effect. If this process
                // dies on the next line, its successor resumes cleanup and never reposts a
                // body the daemon has definitively rejected.
                let cleaning = EntryRecord {
                    pending_process_registration: None,
                    pending_process_registration_origin: None,
                    pending_process_cleanup: Some(registration.clone()),
                    pending_process_cleanup_origin: Some(registration_origin),
                    // A persisted unresolved-native latch is historical authority: a
                    // later snapshot of a cached pid cannot safely weaken it. Recompute
                    // only for the older cache shape that had no explicit latch.
                    pending_unresolved_native_cleanup: record
                        .pending_unresolved_native_cleanup
                        .clone()
                        .or_else(|| {
                            crate::commands::start::unresolved_native_cleanup(
                                &registration.handle,
                                registration_origin,
                            )
                        }),
                    ..record.clone()
                };
                write_entry(cfg, &cleaning)?;
                let original = if failure.interrupted {
                    CliError::Refused(format!(
                        "`goalcup resume` was interrupted during a pre-admission pending-handoff wait: {}. Cleanup is durable and this body will not be reposted.",
                        failure.error
                    ))
                } else {
                    CliError::Control(failure.error)
                };
                let _ = finish_pending_process_cleanup(
                    client,
                    cfg,
                    &cleaning,
                    entry_id,
                    &registration,
                    out,
                );
                return Err(original);
            }
        }
    }

    // A record with no status page is a local capture run's, and this release resumes none.
    if record.status_url.is_empty()
        && retry_busy(|| client.status(entry_id))
            .is_ok_and(|status| status.payload.capture.is_some())
    {
        return Err(CliError::Refused(format!(
            "entry {entry_id} is a local capture run, and this release cannot resume one. \
             `goalcup seal {entry_id}` ends it and writes its trace."
        )));
    }

    // The cached status URL, before anything else happens. It is
    // a string off the participant's own disk and the last line of this command hands it to
    // a browser; `goalcup ui` refuses the same field, and the two must not disagree about
    // one record. Asked here rather than at the open, because the open is past the relaunch
    // and past the hand-off: a refusal there would leave a registered harness running under
    // a command that returned an error.
    crate::browser::check_page_url(&record.status_url, crate::browser::PageKind::Daemon)?;
    let c = crate::connector::load(connectors, &record.connector_id)?;

    // 2. The lock check, before the POST, because the daemon's own refusal carries no pid.
    let roots = LockRoots::from_entry(&record);
    let findings = check_locks(&c.manifest, &roots)?;
    if !findings.is_empty() {
        return Err(lock_refusal(&findings, &record, &c));
    }

    // `--print-launch` is intentionally not a weak form of resume. It fetches only the
    // read-only pins it needs to render the line: no capture catch-up, process-slot clear,
    // credential seeding, child launch, cache rewrite, status open, or registration.
    if opts.print_launch {
        let info = match retry_busy_entry(|| client.resume_preview(entry_id)) {
            Ok(info) => info,
            Err(e) => return Err(translate(e, entry_id)),
        };
        if let Some(field) = roots.disagreement(&LockRoots::from_resume_preview(&info)) {
            return Err(CliError::Refused(format!(
                "the tracker's answer for entry {entry_id} names a different {field} than the \
                 record this machine wrote when the run was created. The lock check just ran \
                 against this machine's reading, so a relaunch would start in a directory \
                 nothing checked — nothing is launched. `goalcup status {entry_id}` says what \
                 the tracker is holding."
            )));
        }
        if let Some(check) = server_port_gate(info.server_port) {
            match check.level {
                Level::Fail => {
                    return Err(CliError::Refused(check.render().trim_end().to_string()));
                }
                _ => say(out, check.render().trim_end())?,
            }
        }
        let vars = crate::vars::from_resume_preview(&info, entry_id)?;
        let launch_env = scrubbed_env(&process_env());
        let plan = plan(
            &c,
            &vars,
            Path::new(&info.workspace),
            LaunchKind::Resume,
            &launch_env,
        )?;
        say(out, &shell_line(&plan))?;
        return Ok(());
    }

    // 3a. Read the daemon's current generation before the mutating CAS.  The local entry file
    // can have missed a successful cancel response when this CLI died; it is a convenience
    // cache, never authority for a generation that another process may already have seen.
    let mut authoritative = match retry_busy_entry(|| client.resume_preview(entry_id)) {
        Ok(info) => info,
        Err(e) => return Err(translate(e, entry_id)),
    };
    if let Some(field) = roots.disagreement(&LockRoots::from_resume_preview(&authoritative)) {
        return Err(CliError::Refused(format!(
            "the tracker's answer for entry {entry_id} names a different {field} than the \
             record this machine wrote when the run was created. The lock check just ran \
             against this machine's reading, so a relaunch would start in a directory \
             nothing checked — nothing is launched. `goalcup status {entry_id}` says what \
             the tracker is holding."
        )));
    }
    // A crash after `/resume` but before this CLI wrote its pending registration leaves an
    // issued OpenResume generation with no child identity to replay.  Only the successor of
    // the local CLI that wrote the pre-POST marker may return that permit: an unmarked open
    // state might be another still-live command, and cancelling it would strand its launch.
    // Prepared without a cached exact body is deliberately not guessed through: it may name
    // a child this invocation did not launch.
    match authoritative.process_handoff_phase.as_str() {
        "open_resume" => {
            let Some(intent) = record.resume_recovery.clone() else {
                return Err(CliError::Refused(format!(
                    "entry {entry_id} has an open resume permit that this machine did not \
                     mark before posting. It may belong to another local CLI, so nothing was \
                     cancelled or relaunched; wait for that command or inspect `goalcup status \
                     {entry_id}`."
                )));
            };
            let Some(open_generation) = intent.expected_generation.checked_add(1) else {
                return Err(CliError::Refused(format!(
                    "entry {entry_id}'s local recovery marker cannot name a successor \
                     generation, so its open permit was left untouched"
                )));
            };
            if authoritative.process_generation != open_generation {
                return Err(CliError::Refused(format!(
                    "entry {entry_id} has open resume generation {} but this machine's \
                     marker only authorizes recovery of generation {open_generation}. It may \
                     belong to another local CLI, so nothing was cancelled or relaunched.",
                    authoritative.process_generation
                )));
            }
            let cancel_request = ProcessHandoffCancelRequest {
                expected_generation: authoritative.process_generation,
                request_digest: None,
            };
            let cancel_result =
                retry_busy_entry(|| client.cancel_process_handoff(entry_id, &cancel_request));
            let cancelled = match cancel_result {
                Ok(cancelled) => cancelled,
                Err(error) => return Err(translate(error, entry_id)),
            };
            let repaired = EntryRecord {
                process_generation: cancelled.process_generation,
                pending_process_registration: None,
                pending_process_registration_origin: None,
                resume_recovery: None,
                ..record.clone()
            };
            write_entry(cfg, &repaired)?;
            record = repaired;
            authoritative = match retry_busy_entry(|| client.resume_preview(entry_id)) {
                Ok(info) => info,
                Err(error) => return Err(translate(error, entry_id)),
            };
            if authoritative.process_handoff_phase == "open_resume" {
                return Err(CliError::Refused(format!(
                    "entry {entry_id}'s recovery cancellation was acknowledged but the \
                     tracker still reports an open permit. Nothing new was launched."
                )));
            }
        }
        "prepared" => {
            return Err(CliError::Refused(format!(
                "entry {entry_id} has a prepared process handoff but this machine has no exact pending registration body to replay; refusing to replace or cancel it"
            )));
        }
        _ => {
            // A marker can outlive a definite non-open outcome (for example, this CLI died
            // after a transport failure whose request never reached the daemon).  It no
            // longer authorizes anything, so remove it before minting a fresh request.
            if record.resume_recovery.is_some() {
                record = clear_resume_recovery(cfg, &record)?;
            }
        }
    }

    // 3b. The mutating resume itself.  The preview above is deliberately read-only; this
    // POST remains the sole one-winner transition.  Persist recovery authority before the
    // first request byte, while this invocation holds the per-entry OS lock.
    let request = ResumeRequest {
        expected_generation: authoritative.process_generation,
    };
    record = EntryRecord {
        resume_recovery: Some(ResumeRecoveryIntent {
            expected_generation: request.expected_generation,
        }),
        ..record
    };
    write_entry(cfg, &record)?;
    let info = match retry_busy_entry(|| client.resume(entry_id, &request)) {
        Ok(info) => info,
        Err(e) => {
            // This is after the request has been sent.  Even a protocol/HTTP error can be
            // reported after the daemon accepted the CAS but before the client decoded a
            // reply, so it is not evidence the permit remains closed.  Preserve the marker;
            // only a later preview proving a non-open phase may clear it.
            return Err(translate(e, entry_id));
        }
    };
    macro_rules! say_or_cancel {
        ($write:expr) => {
            if let Err(error) = $write {
                cancel_issued_resume(
                    client,
                    cfg,
                    &record,
                    entry_id,
                    info.process_generation,
                    None,
                    out,
                );
                return Err(error);
            }
        };
    }
    say_or_cancel!(say(
        out,
        &format!(
            "entry {entry_id}: caught up {} events before anything new was appended.",
            info.caught_up_events
        ),
    ));
    // What the tracker last held a handle for (D16): the one thing on the resume answer that
    // says what the stopped run was, and a participant whose harness is somehow still alive
    // wants to know its pid before this command starts a second one.
    if let Some(last) = &info.last_process {
        say_or_cancel!(say(
            out,
            &format!(
                "  the tracker's last handle for this run: pid {} ({}), launched at {}{}",
                last.pid,
                crate::commands::status::plain(&last.program),
                crate::commands::status::plain(&last.launched_at),
                match is_alive(last.pid) {
                    true => " — and something is still running under that number",
                    false => "",
                }
            ),
        ));
    }
    if let Some(field) = roots.disagreement(&LockRoots::from_resume(&info)) {
        let error = CliError::Refused(format!(
            "the tracker's answer for entry {entry_id} names a different {field} than the \
             record this machine wrote when the run was created. The lock check just ran \
             against this machine's reading, so a relaunch would start in a directory \
             nothing checked — nothing is launched. `goalcup status {entry_id}` says what \
             the tracker is holding."
        ));
        cancel_issued_resume(
            client,
            cfg,
            &record,
            entry_id,
            info.process_generation,
            None,
            out,
        );
        return Err(error);
    }

    // 4. The relaunch, from `[resume]` and the same eight variables the launch rendered.
    //    The port first: the daemon remembers the port it reserved and does not re-reserve
    //    it, so between the stop and now anything at all may have taken it, and a harness
    //    launched onto a held port binds nothing. `None` for every connector that reserves
    //    none, which is every shipped one: no shipped manifest names `{server_port}`.
    if let Some(check) = server_port_gate(info.server_port) {
        match check.level {
            Level::Fail => {
                let error = CliError::Refused(check.render().trim_end().to_string());
                cancel_issued_resume(
                    client,
                    cfg,
                    &record,
                    entry_id,
                    info.process_generation,
                    None,
                    out,
                );
                return Err(error);
            }
            _ => say_or_cancel!(say(out, check.render().trim_end())),
        }
    }
    let vars = match crate::vars::from_resume(&info, entry_id) {
        Ok(vars) => vars,
        Err(error) => {
            cancel_issued_resume(
                client,
                cfg,
                &record,
                entry_id,
                info.process_generation,
                None,
                out,
            );
            return Err(error);
        }
    };
    let launch_env = scrubbed_env(&process_env());
    let plan = match plan(
        &c,
        &vars,
        Path::new(&info.workspace),
        LaunchKind::Resume,
        &launch_env,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            cancel_issued_resume(
                client,
                cfg,
                &record,
                entry_id,
                info.process_generation,
                None,
                out,
            );
            return Err(error);
        }
    };
    // The per-run home is the same directory the launch used, and the harness has been
    // writing into it ever since — so the seed is merged into what is there rather than
    // replacing it. Repeated on a relaunch and not only on the first start, because a
    // participant may have logged out, rotated an account or upgraded between the two, and a
    // relaunch that met a login wall would be exactly the failure the first start guards
    // against. `$HOME` comes from this process's own environment: this command, unlike
    // `goalcup start`, is handed no environment map to ask.
    //
    // **A refresh, and never a refusal.** `goalcup start` refuses a machine whose harness
    // configuration carries none of the required keys, because launching into a login wall
    // wastes the entry. Here the home already holds what the first launch seeded, so a
    // machine that cannot be read from now — a `$HOME` this process was not given, a
    // configuration file the participant has since moved — is not a reason to refuse to
    // relaunch a run that is already under way. It is a line on the screen.
    //
    // **The resume seeds, so the resume records too.** `carried` is what this relaunch took
    // in — normally every declared key already held at the value being written, and the
    // credential file left exactly as the first launch copied it — and it rides on the
    // handle this relaunch is about to register.
    let mut carried = None;
    if let Some(spec) = &c.manifest.home_seed {
        match crate::per_run_home::home_from(&process_env()).and_then(|home| {
            crate::home_seed::seed(
                &c.manifest.id,
                spec,
                &home,
                Path::new(&info.harness_home),
                Path::new(&info.workspace),
            )
        }) {
            Ok(seeded) => {
                carried = Some(seeded.record.clone());
                say_or_cancel!(say(out, &seeded.line()))
            }
            Err(e) => say_or_cancel!(say(
                out,
                &format!(
                    "this run's harness home could not be re-seeded from this machine's \
                     configuration: {e}\n  It still holds what the first launch put there, \
                     so this relaunch goes ahead. If the harness asks you to log in, log in \
                     outside the tracker and run `goalcup resume` again."
                ),
            )),
        }
    }
    let record_before_native_guard = record.clone();
    if let Some(guard) =
        crate::commands::start::unresolved_native_guard(&plan, ProcessCleanupOrigin::Resume)
    {
        let guarded = EntryRecord {
            pending_unresolved_native_cleanup: Some(guard),
            ..record.clone()
        };
        if let Err(error) = write_entry(cfg, &guarded) {
            cancel_issued_resume(
                client,
                cfg,
                &record_before_native_guard,
                entry_id,
                info.process_generation,
                None,
                out,
            );
            return Err(error);
        }
        record = guarded;
    }
    let mut spawned = match spawn(&plan) {
        Ok(spawned) => spawned,
        Err(error) => {
            // `spawn` returned no Child. Restore the exact pre-guard row before attempting
            // permit cancellation, so an unavailable cancellation preserves
            // `resume_recovery` rather than a false unresolved-child blocker that would
            // prevent the next invocation from reaching preview reconciliation.
            let guard_clear = if record.pending_unresolved_native_cleanup.is_some() {
                write_entry(cfg, &record_before_native_guard)
            } else {
                Ok(())
            };
            cancel_issued_resume(
                client,
                cfg,
                &record_before_native_guard,
                entry_id,
                info.process_generation,
                None,
                out,
            );
            guard_clear?;
            return Err(error);
        }
    };
    let launched_at = crate::launch::launched_at_now();
    let mut handed = match handle(&plan, &spawned, &launched_at) {
        Ok(handed) => handed,
        // No handle, so no group: the pid is the whole of what is known about the child.
        Err(e) => {
            let error = crate::commands::start::kill(spawned.child, None, e);
            cancel_issued_resume(
                client,
                cfg,
                &record,
                entry_id,
                info.process_generation,
                None,
                out,
            );
            return Err(error);
        }
    };
    handed.home_seed = carried;
    let registration = ProcessRegistration {
        generation: info.process_generation,
        handle: handed.clone(),
    };
    let pre_post_cleanup = record
        .pending_unresolved_native_cleanup
        .as_ref()
        .map(|_| EntryRecord {
            process_generation: info.process_generation,
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: Some(registration.clone()),
            pending_process_cleanup_origin: Some(ProcessCleanupOrigin::Resume),
            pending_unresolved_native_cleanup: crate::commands::start::unresolved_native_cleanup(
                &registration.handle,
                ProcessCleanupOrigin::Resume,
            ),
            ..record.clone()
        });
    if let Some(cleaning) = &pre_post_cleanup
        && let Err(error) = write_entry(cfg, cleaning)
    {
        let error = crate::commands::start::kill_registered(spawned.child, &handed, error);
        let _ = cancel_issued_resume(
            client,
            cfg,
            &record,
            entry_id,
            info.process_generation,
            None,
            out,
        );
        return Err(error);
    }
    // The last moment a Ctrl-C is this command's to act on, and the only one where there is
    // a child to take with it. Past `register_process` the daemon owns the process and will
    // terminate it at the deadline; before it, nothing will ever measure this child.
    if crate::arming::interrupted() {
        let error = crate::commands::start::kill_registered(
            spawned.child,
            &handed,
            CliError::Refused(
                "`goalcup resume` was interrupted before the relaunched harness was handed \
                 over, so it has been stopped. The entry is untouched and this can be run \
                 again."
                    .to_string(),
            ),
        );
        if let Some(cleaning) = &pre_post_cleanup {
            let _ =
                finish_pending_process_cleanup(client, cfg, cleaning, entry_id, &registration, out);
        } else {
            cancel_issued_resume(
                client,
                cfg,
                &record,
                entry_id,
                info.process_generation,
                None,
                out,
            );
        }
        return Err(error);
    }

    // 5. The hand-off. The daemon retains the predecessor as termination authority while
    //    this issued generation is open, then atomically replaces it only after this exact
    //    registration succeeds. A relaunch that skips this step leaves the new child outside
    //    the durable seal target, which is the whole reason the hand-off exists.
    // The replay body is durable before the first request byte.  After this point an
    // Unreachable may be a lost accepted response, so no local cleanup can discard it.
    let pending_before_post = EntryRecord {
        process_generation: info.process_generation,
        pending_process_registration: Some(registration.clone()),
        pending_process_registration_origin: Some(ProcessCleanupOrigin::Resume),
        pending_process_cleanup: None,
        pending_process_cleanup_origin: None,
        pending_unresolved_native_cleanup: pre_post_cleanup
            .as_ref()
            .and_then(|record| record.pending_unresolved_native_cleanup.clone()),
        ..record.clone()
    };
    if let Err(error) = write_entry(cfg, &pending_before_post) {
        let error = crate::commands::start::kill_registered(spawned.child, &handed, error);
        if let Some(cleaning) = &pre_post_cleanup {
            let _ =
                finish_pending_process_cleanup(client, cfg, cleaning, entry_id, &registration, out);
        } else {
            let _ = cancel_issued_resume(
                client,
                cfg,
                &record,
                entry_id,
                info.process_generation,
                None,
                out,
            );
        }
        return Err(error);
    }
    if let Err(failure) = retry_busy_process_classified(|| {
        DaemonClient::register_process_with_admission(client, entry_id, &registration)
    }) {
        if failure.may_have_been_admitted {
            // An ambiguous response may follow `Prepared` or `Registered`; neither a local
            // kill nor cancellation is safe. Persist the exact body for a later replay.
            return Err(if failure.interrupted {
                CliError::Refused(format!(
                    "`goalcup resume` was interrupted while the replacement handoff may already have been admitted: {}. The exact handoff remains pending; run resume again to recover it.",
                    failure.error
                ))
            } else {
                CliError::Control(failure.error)
            });
        }
        // This is a definite server response, not a lost acknowledgement.  The child never
        // became the daemon's process, so stop it even if the follow-up cancellation cannot
        // be confirmed; otherwise a refused hand-off leaks an untracked harness.  Clear the
        // exact replay body first so a later CLI never retries a registration the daemon
        // already rejected.  The recovery marker stays if cancellation is ambiguous.
        let handoff_refusal = if failure.interrupted {
            CliError::Refused(format!(
                "`goalcup resume` was interrupted during a pre-admission replacement wait: {}. The replacement harness has been stopped.",
                failure.error
            ))
        } else {
            CliError::Control(failure.error)
        };
        let rejected = EntryRecord {
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: Some(registration.clone()),
            pending_process_cleanup_origin: Some(ProcessCleanupOrigin::Resume),
            pending_unresolved_native_cleanup: crate::commands::start::unresolved_native_cleanup(
                &registration.handle,
                ProcessCleanupOrigin::Resume,
            ),
            ..record.clone()
        };
        if let Err(cache_error) = write_entry(cfg, &rejected) {
            let _ = say(
                out,
                &format!(
                    "  (the definite refusal was received, but cleanup could not be made durable before stopping the child: {cache_error})"
                ),
            );
            return Err(crate::commands::start::kill_registered(
                spawned.child,
                &registration.handle,
                handoff_refusal,
            ));
        }
        let hand_off_error = crate::commands::start::kill_registered(
            spawned.child,
            &registration.handle,
            handoff_refusal,
        );
        let _ =
            finish_pending_process_cleanup(client, cfg, &rejected, entry_id, &registration, out);
        return Err(hand_off_error);
    }

    // From here the daemon owns the process, and the entry stands whatever happens below.
    // The pid is the one field the relaunch changes; a rewrite that failed is worth saying
    // and is not worth ending a live run over, exactly as it is in `goalcup start`.
    let status_url = record.status_url.clone();
    if let Err(e) = write_entry(
        cfg,
        &EntryRecord {
            process_generation: info.process_generation,
            pending_process_registration: None,
            pending_process_registration_origin: None,
            pending_process_cleanup: None,
            pending_process_cleanup_origin: None,
            pending_unresolved_native_cleanup: None,
            resume_recovery: None,
            last_pid: Some(handed.pid),
            // The daemon's bound session id, which is what this relaunch was rendered with
            // and what the next resume must select. The field held the id the daemon minted
            // at creation; on a Codex run the harness minted its own and the two differ, so a
            // cache left at the first one is a cache that describes a session nothing is
            // writing to. See [`LockRoots`].
            session_id: info.session_id.clone(),
            ..record
        },
    ) {
        say(
            out,
            &format!("  (this machine's own note of the harness's pid could not be updated: {e})"),
        )?;
    }
    // The URL is printed whether or not a browser is opened, so a headless machine
    // still gets it. It comes off the record because `ResumeInfo` carries none — the status
    // page is a creation-time fact and a resume does not restate it.
    say(out, &format!("status: {status_url}"))?;
    // Checked at step 1, before the daemon was asked anything, so nothing here can refuse:
    // what is left is a machine with no browser, which is a note and not a failure.
    if opts.open_browser
        && let Some(note) = crate::browser::open_page(
            &status_url,
            crate::browser::PageKind::Daemon,
            &crate::browser::system_opener,
        )?
    {
        say(out, &note)?;
    }
    say(out, KEEP_GOING)?;
    say(out, "Recording. Leave this running.")?;

    // The harness owns the terminal from here, exactly as it does after `goalcup start`: a
    // command that returned now would hand the prompt back while a TUI was drawing into it.
    // The harness's own exit code is not this CLI's — a run whose agent exited non-zero is
    // still a run that was recorded, and this command reports the ladder in `lib.rs`.
    let status = spawned.child.wait().map_err(|source| CliError::Io {
        path: handed.program.clone(),
        source,
    })?;
    // "Recording. Leave this running." is true of the minute a TUI is up and a promise about
    // a process that may already be gone, so it is followed by the sentence that closes it:
    // the exit is named, and beside it what the tracker has — the count this resume caught up
    // before the relaunch, and what the daemon holds now.
    say(out, &format!("the harness exited: {status}."))?;
    let held = match retry_busy(|| client.status(entry_id)) {
        Ok(document) if document.payload.entry_id == entry_id => format!(
            " The tracker has {} event(s) uploaded and {} pending.",
            document.payload.events_uploaded, document.payload.events_pending
        ),
        // Not a failure of this command: the run is recorded either way, and a tracker that
        // did not answer is a fact to state rather than an error to end a finished relaunch
        // over.
        _ => String::new(),
    };
    say(
        out,
        &format!(
            "  {} event(s) were caught up before this relaunch.{held} `goalcup resume \
             {entry_id}` continues it; `goalcup seal {entry_id}` finishes it.",
            info.caught_up_events
        ),
    )?;
    Ok(())
}

/// The lock refusal: exit 4, every path, what is known about each holder, and
/// the one move that clears it.
///
/// [`CliError::Gate`] and not `Refused`, and the distinction is the one `lib.rs` draws: this
/// is a rule saying no to a relaunch that would otherwise have gone ahead, and a script
/// branching on the number gets "something is holding this session open" rather than "you
/// typed something wrong".
///
/// `findings` is never empty — its only caller has just checked — and an empty one would say
/// so rather than name nothing, because this crate answers instead of unwrapping.
fn lock_refusal(findings: &[LockFinding], record: &EntryRecord, c: &Connector) -> CliError {
    let entry = &record.entry_id;
    let connector = &c.manifest.id;
    let mut lines = String::new();
    for f in findings.iter().take(MAX_LISTED_LOCKS) {
        let holder = match (f.holder_pid, f.holder_alive) {
            (Some(pid), true) => {
                format!("held by pid {pid} ({}), which is still running", named(f))
            }
            (Some(pid), false) => format!("names pid {pid}, which is not running"),
            (None, _) => "says nothing about who holds it".to_string(),
        };
        lines.push_str(&format!("\n  {} — {holder}", f.path));
    }
    if findings.len() > MAX_LISTED_LOCKS {
        lines.push_str(&format!(
            "\n  … and at least {} more.",
            findings.len() - MAX_LISTED_LOCKS
        ));
    }
    // A pid to stop comes first where there is one: a `rm` that races a live writer is the
    // worse advice of the two, and the participant only ever needs the first move.
    //
    // **No `kill` command is printed**. The pid comes out of a
    // file under `harness_home` — a directory the measured harness owns and can write with
    // a shell tool — and [`is_alive`] only asks whether something answers to that number, so
    // pid 1, the participant's editor and `goalcup-daemon` itself all read as live holders.
    // A command line the participant can paste turns that into an instruction to kill a
    // process the agent under measurement chose. So this prints the pid, what the process
    // table calls it, and asks the participant to recognise it: the evidence is theirs to
    // read and the move is theirs to make.
    let what_to_do = match findings.iter().find(|f| f.holder_alive) {
        Some(held) => {
            let pid = match held.holder_pid {
                Some(pid) => pid.to_string(),
                // Not reachable: `holder_alive` is only ever set from a pid that was read.
                None => "the holder".to_string(),
            };
            format!(
                "Pid {pid} is running and the process table calls it {name}. That number \
                 came out of {path}, which the harness wrote — so look at it before you act \
                 on it: if that is your harness, stop it, check that {path} is gone, and run \
                 `goalcup resume {entry}` again.",
                name = named(held),
                path = held.path
            )
        }
        None => format!(
            "Nothing is holding these, so they are the residue of a hard kill — but the \
             tracker refuses on the path EXISTING, not on a holder, and this command will \
             not delete a file a harness wrote. Clear them yourself and run `goalcup resume \
             {entry}` again:{}",
            findings
                .iter()
                .take(MAX_LISTED_LOCKS)
                .map(|f| format!("\n  rm {}", quote(&f.path)))
                .collect::<String>()
        ),
    };
    CliError::Gate(format!(
        "entry {entry} cannot be resumed: `{connector}`'s lock is still on disk.{lines}\nM0 \
         §2.5: {M0_LOCK_SENTENCE} — a second root agent in the trace, and this run's \
         continuity lost. Nothing was asked of the tracker.\n{what_to_do}"
    ))
}

/// What `ps` calls a finding's holder, or a phrase saying it could not be asked.
///
/// Never an empty pair of quotation marks in the middle of a sentence: a machine where the
/// process table could not be read says so, because "the tracker does not know what that
/// process is" and "the tracker says it is nothing" are two different things to act on.
fn named(f: &LockFinding) -> String {
    match &f.holder_command {
        Some(command) => format!("`{command}`"),
        None => "a process this machine's `ps` could not name".to_string(),
    }
}

/// The daemon's own refusals, put on the rungs `lib.rs` reserves for them.
///
/// `ControlError::Conflict` leaves under rung 1 by default, which is right for the commands
/// that only report it and wrong here: a resume the daemon refused over a lock is "a rule
/// said no, and killing something is the move", which is rung 4 — the same rung the lock
/// this side found itself leaves under ([`lock_refusal`]). A **sealed** entry is the one
/// that is genuinely over, and it stays on rung 1: nothing is going to unseal it.
///
/// **`Busy` is rung 1 here, exactly as it is everywhere else.** The rung says what kind of
/// thing said no, and a busy daemon is the tracker's own problem rather than a gate's verdict
/// on this run: nothing about the harness, the workspace or the locks has been decided, and
/// the entry is untouched. `goalcup seal` puts the same fact on rung 1, and one fact on two
/// rungs is a number a script cannot branch on. So it is a refusal the participant can fix
/// by running the command again, which is what rung 1 is.
///
/// The seal is told from the lock by the daemon's own words, because that is all a `409`
/// carries: `resume_entry` answers "entry {id} is sealed: …" for the first and "entry {id}
/// cannot be resumed while the harness's lock is still on disk: …" for the second, and
/// neither sentence contains the other's verb.
///
/// **The split reads the daemon's sentence and never a path.** Both messages put the
/// daemon's own words before the first colon and everything variable — paths, in the lock
/// case — after it, so [`names_a_seal`] looks only at the head. A `contains("seal")` over the
/// whole message would read a lock refusal naming `/Users/x/sealed-runs/…` as a sealed entry
/// and send the participant to rung 1, where there is nothing to do.
fn translate(e: ControlError, entry_id: &str) -> CliError {
    match e {
        ControlError::Busy(said) => CliError::Refused(format!(
            "the tracker is still busy with entry {entry_id} after up to {} attempts: {said}. The \
             entry is held, not broken — wait a moment and run `goalcup resume {entry_id}` \
             again.",
            crate::retry::ENTRY_BUSY_ATTEMPTS
        )),
        ControlError::Conflict(said) if names_a_seal(&said) => CliError::Refused(format!(
            "entry {entry_id} is already sealed, so there is nothing to resume: {said}. A \
             sealed entry's chain is closed and nothing will reopen it; `goalcup status \
             {entry_id}` shows what was recorded."
        )),
        // The daemon's own words, and nothing appended. Both sides apply one rule —
        // `goalcup_protocol::manifest::lock_path_is_held` — so the two readings can differ only
        // because they ran a moment apart, or because the daemon resolved a root this side did
        // not. Advice about "whatever is holding it" would be advice about a file this side has
        // not looked at, so there is none.
        ControlError::Conflict(said) => CliError::Gate(format!(
            "the tracker refused to resume entry {entry_id}: {said}. This machine's own lock \
             check ran first, under the same rule, and found nothing — most likely because \
             this is a FIRST resume and the two sides rendered different file names: this \
             side renders the id the start minted, the tracker the one the run bound. The \
             lock may also have appeared between the two readings, or sit under a root only \
             the tracker resolved. `goalcup status {entry_id}` says what it is holding."
        )),
        other => CliError::Control(other),
    }
}

/// Whether a `409` is the daemon's sealed refusal rather than its lock refusal.
///
/// Only the head of the message — everything before the first colon — because that is the
/// part the daemon writes itself. See [`translate`] for what reading the whole of it cost.
fn names_a_seal(said: &str) -> bool {
    let head = said.split(':').next().unwrap_or(said);
    head.to_lowercase().contains("is sealed")
}

/// One line to the participant, or the write error naming the stream.
fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "stdout".to_string(),
        source,
    })
}
