//! The launch recipe, rendered: the exact argv, the exact environment delta, the exact
//! working directory — and the handle the daemon finishes the job with (D22).
//!
//! # Building a plan is pure
//!
//! [`build_capture_launch`] takes a connector, a variable map, a directory and the
//! environment the child will start from, and returns a [`LaunchPlan`]. It spawns nothing,
//! reads no environment variable of its own and touches no disk. That is what lets every pin
//! in this file be tested on a machine with no harness installed. The V1 builder `plan` was
//! pure the same way, which is what let the V1 `goalcup start --print-launch` print a line
//! without running it; D48 removed both.
//!
//! # The environment is built from nothing (D1, D23, D38)
//!
//! [`build_capture_launch`] is the one builder left, and its child inherits nothing.
//! [`LaunchPlan::env_base`] is empty and [`LaunchPlan::env_remove`] is empty; [`spawn`] clears
//! the child's environment, so what reaches the harness is exactly [`LaunchPlan::env_set`]:
//!
//! * the profile's own `capture.env`, rendered;
//! * the allowlist [`CAPTURE_IDENTITY_ENV`] (`USER`, `LOGNAME`, `TMPDIR`), carried across
//!   from the tracker's environment and nothing else from it;
//! * `HOME` (the per-run home), `PATH` ([`CAPTURE_PATH`]) and the profile's home variable;
//! * the passive **loopback** recording proxy through process-scoped `HTTPS_PROXY`/
//!   `https_proxy` — the M2 reservation AGENTS.md names, and the only proxy a launch may name,
//!   checked by `loopback_proxy_url` — with `NO_PROXY`/`no_proxy` ([`CAPTURE_NO_PROXY`]) and
//!   the profile's process-scoped CA variable.
//!
//! **D1: the tracker never sets a model base URL.** Nothing on that list can be one except
//! the profile's `capture.env`, and `goalcup_protocol::capture_policy` refuses a profile that
//! names a `*_BASE_URL`, `*_API_URL` or `*ENDPOINT` variable, a `*_PROXY` or bare `PROXY`, or
//! any other `*_URL` there, before a plan is ever built. The participant's shell cannot add
//! one either, because nothing outside the allowlist is inherited: an allowlist leaves no
//! `*_URL` the participant exported to strip.
//!
//! Headless as it is, the capture child stays in the launcher's process group: the daemon
//! never confirms a group ended (D30), and there a Ctrl-C typed at the terminal reaches it.
//!
//! # The three modes differ in exactly one place
//!
//! [`LaunchMode::Tui`] must **never** call `process_group(0)`. A TUI in a process group of
//! its own is not the terminal's foreground group, so its first read from the tty raises
//! SIGTTIN and stops it — the harness appears to hang before it has drawn a frame. A TUI
//! handle therefore carries `pgid: None`, and `KillStrategy::ProcessGroup` degrades
//! to the descendant walk it already implements.
//!
//! [`LaunchMode::Server`] is a headless harness the participant talks to over a socket. It
//! has no tty to lose, so it may own its group and its handle carries a real `pgid` — which
//! is the strictly better thing to hand a terminator. No shipped connector uses this mode;
//! the variant stays in the schema for a harness that is genuinely headless.
//!
//! The port refusal is not this mode's. Any recipe naming `{server_port}` is refused on an
//! empty one, whatever the launch mode: `--port ""` is a harness started on nothing, and the
//! daemon reserving no port is a bug worth naming.
//!
//! [`LaunchMode::AttachOnly`] builds no plan at all. In V1, attach mode armed the folder and
//! the participant opened a session in it; there was no argv, so the V1 `plan` refused it
//! rather than inventing one. A capture plan is always [`LaunchMode::Server`].
//!
//! # `handle()` is separate from `spawn()`
//!
//! For two reasons. `native_pid` discovery needs a live process tree to walk, so it cannot
//! happen before the child exists. And it may legitimately answer `None` at launch, because
//! the npm wrapper spawns the native binary a moment later — the daemon re-resolves at seal
//! from a fresh snapshot, which is why this is best-effort rather than a precondition.
//!
//! Being separate is exactly why [`spawn`] hands back the program path it used, in
//! [`Spawned::program`], and why [`handle`] takes that value rather than looking the name up
//! a second time. Between the two calls a shim can be rewritten, a version manager can
//! reorder `PATH`, an installer can finish: a second lookup would record a program the child
//! never exec'd, and the daemon's identity guard would then reject a harness that is doing
//! exactly what it was told to do. One resolution, at the one moment it is true.
//!
//! # A capture child is gated before `exec` (D40)
//!
//! [`spawn`] is the ordinary launch (a TUI or a server). A v5 capture child is never started
//! by it: [`spawn_gated`] forks the child and parks it before `exec`, [`gated_handle`] names it
//! while it is parked, and [`GatedSpawn::release`] lets it run only after the daemon has durably
//! registered it. The protocol and its failure modes are written beside `spawn_gated` below.
//!
//! # Why the handle records argv, and who checks it afterwards
//!
//! The profile's argument checks ([`harness_arg_refusal`]) run **once**, when the plan is
//! built. Nothing in them stops a participant from killing the launched harness and starting
//! their own by hand in the same workspace, and **this crate cannot close that hole**: no
//! check watches the process while the run is live (the V1 per-entry poll that did was
//! removed by D48). What this module owes the daemon is enough recorded truth to make the one
//! later check possible, and that truth is three fields on [`ProcessHandle`] — `program`,
//! `argv` and `env_keys`. Before the daemon signals anything at the seal, its termination
//! guard (`goalcup_daemon::terminate`) compares the live process's creation identity and
//! command line against them, so a stranger on a reused pid is refused rather than killed.
//! `env_keys` is names only and never values, because a process's environment is not portably
//! readable after the fact and because one of those values can be a credential.
//!
//! **That check is the daemon's and this module must not grow a second copy of it.**
//! Nor may it grow a deadline killer: terminating a recorded run is
//! `goalcup_daemon::terminate::terminate`, and two killers with different grace periods racing
//! at the deadline is the one failure whose symptom is silence. What this module does end is
//! only this process's own [`Child`], with `Child::kill` and no grace: one the gate parked and
//! never released, one whose gate step failed, and one just released whose tee could not start
//! (the seal that follows answers for that one). A child released onto a sealed entry is ended,
//! with a grace, by `released_child.rs`. The `tests/it/process_handoff.rs` sweep matches signals by
//! name and cannot see `Child::kill`, so those sites are listed here.

use crate::CliError;
use crate::connector::HarnessConnector;
use crate::proxy::connect::PROXY_USERNAME;
use goalcup_protocol::capture::MAX_GOAL_BYTES;
use goalcup_protocol::harness::{CaptureSpec, IdGrammar, IdentitySource, Strategy};
use goalcup_protocol::manifest::{Kill, LaunchMode};
use goalcup_protocol::process::{ProcessHandle, find_named, process_start_identity, snapshot};
use goalcup_protocol::template::{TemplateVars, render};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// The tees of a Hermes capture child's streams live in their own file; they are part of this
/// module's interface (D40, D41).
pub use crate::capture_tee::{
    CaptureStderr, ScopeMarker, ScopeReceiver, tee_marked, tee_marked_capture,
    tee_marked_capture_to, tee_marked_capture_with_identity, tee_marked_capture_with_identity_to,
    tee_marked_capture_with_rule, tee_marked_with_rule, tee_stderr_capture_with_rule,
};
/// The ending of a child this process just released lives in its own file, which the
/// "this crate never terminates anything" sweep exempts by name; it is part of this module's
/// interface (D40, the root ruling).
pub use crate::released_child::{ReleasedChildEnd, end_released_child};

/// Everything a launch is, decided before anything is spawned.
///
/// `Debug` is hand-written for the same reason `TemplateVars`' and `ControlFile`'s are:
/// [`Self::env_set`] may hold a `*_TOKEN` pin, and a derived one would put that token into
/// whatever a `{:?}` on a launch failure reaches.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The program as the manifest names it, resolved on `PATH` at spawn time. Not resolved
    /// here, because resolution reads the filesystem and this struct is built by a pure
    /// function.
    pub program: String,
    /// The rendered argument vector, after the banned-args guard. No shell is involved at
    /// any point: each element becomes one `Command::arg`, so nothing here is quoted and
    /// nothing here needs to be.
    pub argv: Vec<String>,
    /// `[launch.env]`, rendered. Keys are literal, values are templates.
    pub env_set: BTreeMap<String, String>,
    /// Removed from [`Self::env_base`] before the child is spawned.
    pub env_remove: Vec<String>,
    /// The environment the child starts from. [`spawn`] clears the child's environment and
    /// rebuilds it from this, so a variable that is not here does not reach the harness.
    /// [`build_capture_launch`] leaves it empty: a capture child inherits nothing (D38).
    pub env_base: BTreeMap<String, String>,
    /// The workspace. The harness's working directory is the directory the run is about.
    pub cwd: PathBuf,
    pub mode: LaunchMode,
    pub kill: Kill,
    /// Set by [`build_capture_launch`], the one builder left: a headless capture launch. The
    /// V1 `plan`, removed by D48, left it `None`.
    pub capture: Option<HeadlessCapture>,
}

/// What makes a capture launch different from an ordinary one, beyond its argv and environment.
///
/// [`spawn`] gives such a child `/dev/null` for stdin (both harnesses
/// append piped stdin to the goal); its environment is exactly [`LaunchPlan::env_set`],
/// because [`build_capture_launch`] leaves [`LaunchPlan::env_base`] empty (built from nothing, D38);
/// and `argv[goal_index]` is the goal, which `Debug` does not print.
///
/// A recipe that names a `goal_flag` carries its goal inside that flag's own element
/// (`--query=<goal>`), so the element is `goal_prefix` bytes of flag and `=` and then the goal;
/// a recipe whose args end in `--` has `goal_prefix` 0 and the whole element is the goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadlessCapture {
    pub goal_index: usize,
    pub goal_prefix: usize,
}

impl fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchPlan")
            .field("program", &self.program)
            .field(
                "argv",
                &Argv {
                    argv: &self.argv,
                    goal: self.capture.map(|c| (c.goal_index, c.goal_prefix)),
                },
            )
            .field("env_set", &RedactedEnv(&self.env_set))
            .field("env_remove", &self.env_remove)
            // Names only. This is the participant's own environment: their API keys, their
            // cloud credentials, their shell history file. A `{:?}` on a launch failure is
            // not a place any of that belongs, and which variables were carried across is
            // the whole of what a reader of one needs.
            .field("env_base", &Keys(&self.env_base))
            .field("cwd", &self.cwd)
            .field("mode", &self.mode)
            .field("kill", &self.kill)
            .field("capture", &self.capture)
            .finish()
    }
}

/// What a secret value prints as, on every surface in this crate that prints one.
const REDACTED: &str = "<redacted>";

/// Prints the argv with the goal, if there is one, as its length only (the flag that carries
/// it, `--query=`, is printed). The goal is retained as a redacted evidence stream and nowhere
/// else.
struct Argv<'a> {
    argv: &'a [String],
    /// The goal element's index and the length of the flag text in front of the goal.
    goal: Option<(usize, usize)>,
}

impl fmt::Debug for Argv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut l = f.debug_list();
        for (i, a) in self.argv.iter().enumerate() {
            match self.goal {
                Some((goal_index, prefix)) if goal_index == i => {
                    let flag = a.get(..prefix).unwrap_or("");
                    l.entry(&format_args!(
                        "{flag}<goal: {} bytes>",
                        a.len().saturating_sub(prefix)
                    ));
                }
                _ => {
                    l.entry(a);
                }
            }
        }
        l.finish()
    }
}

/// Prints every key and every value except a `*_TOKEN` one, and with the userinfo of any
/// URL in a value replaced.
///
/// The suffix and not an exact name, so a connector naming its own `*_TOKEN` need not
/// remember to come back here. Compared case-insensitively: an environment key is
/// conventionally upper case and `TemplateVars`' own rule is spelled on the lower-case
/// variable name, so the check has to hold for either spelling.
///
/// The userinfo rule is for the capture launch's
/// `HTTPS_PROXY=http://goalcup:<secret>@127.0.0.1:<port>`: a key-name filter cannot see a
/// secret that rides inside a URL.
struct RedactedEnv<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for RedactedEnv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut m = f.debug_map();
        for (k, v) in self.0 {
            if is_secret_key(k) {
                m.entry(k, &REDACTED);
            } else if let Some(redacted) = without_userinfo(v) {
                m.entry(k, &redacted);
            } else {
                m.entry(k, v);
            }
        }
        m.finish()
    }
}

fn is_secret_key(key: &str) -> bool {
    key.to_ascii_uppercase().ends_with("_TOKEN")
}

/// `value` with the userinfo of every `scheme://userinfo@host` in it replaced by
/// [`REDACTED`], or `None` when it carries none.
fn without_userinfo(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    let mut changed = false;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#' | ',') || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str(REDACTED);
                out.push_str(&authority[at..]);
                changed = true;
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// Prints a map's keys and none of its values.
struct Keys<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for Keys<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// This process's environment, as a map — the `parent_env` [`build_capture_launch`] takes
/// the [`CAPTURE_IDENTITY_ENV`] allowlist from.
///
/// `vars_os` and not `vars`, which **panics** on a variable whose name or value is not valid
/// Unicode — a participant with one exported would get a panic where they had asked for a
/// launch, and this crate does not panic. Such a pair is skipped instead: the harness does
/// not receive it, which is the same thing that happens to every other variable this launch
/// does not carry across, and it is visible on the plan rather than silent.
///
/// Here rather than inside one command, because every command that builds a plan has to read
/// the environment the same way. The V1 `goalcup resume` (removed by D48) built one too, and
/// a relaunch that read the environment a different way would have been exactly the
/// difference between a run's two halves that this module exists to prevent. It is not
/// called by [`build_capture_launch`], which stays pure by taking the map as an argument.
pub fn process_env() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

/// The instant a launch is stamped with, in the one spelling this tree writes a time in.
///
/// Read by the caller at the moment the process started and handed to [`handle`]: the
/// daemon's terminator compares it against the age the process table reports for the pid and
/// refuses to signal one younger than its handle says, which is how a recycled number is
/// told from the launch's own process.
///
/// A clock somehow before the epoch reads as the epoch rather than panicking; nothing
/// downstream would believe either number.
pub fn launched_at_now() -> String {
    goalcup_protocol::control::format_z(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// A recipe that names `{server_port}` may not be launched on an empty one.
///
/// The check is on the templates rather than on the rendered strings, because an empty
/// render leaves nothing to look for: `--port "" --hostname 127.0.0.1` and a recipe that
/// never wanted a port are the same argv once the substitution has happened. So the
/// question asked is the honest one — does this recipe need the variable, and did the
/// daemon supply it.
///
/// Not restricted to [`LaunchMode::Server`]: a `Tui` recipe carrying `--port`/`--hostname`
/// flags is as much a harness started on nothing when the value is empty. The thing refused
/// is a recipe launched on an empty value it asked for, whatever owns the terminal. The
/// daemon reserves on the same question, over the same templates.
fn needs_a_port_it_has(
    args: &[String],
    env: &BTreeMap<String, String>,
    v: &TemplateVars,
    id: &str,
) -> Result<(), CliError> {
    const PORT: &str = "{server_port}";
    let named = args.iter().any(|t| t.contains(PORT)) || env.values().any(|t| t.contains(PORT));
    if !named {
        return Ok(());
    }
    // The V1 `vars::build` (removed by D48) rendered an unreserved port as the empty string
    // deliberately, so that this refusal could exist rather than a number being invented
    // there. A map with no `server_port` at all reads the same.
    if v.get("server_port").unwrap_or("").is_empty() {
        return Err(CliError::Refused(format!(
            "`{id}`'s launch recipe renders `{PORT}`, and no port was reserved for this \
             entry: the port is empty, so the harness would be started on nothing. Nothing \
             is launched."
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------ M2 capture launch

/// The `PATH` of a capture launch: the system directories and nothing a participant's shell
/// put in front of them (D38).
pub const CAPTURE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// `NO_PROXY`/`no_proxy` of a capture launch: the loopback proxy itself is never proxied.
pub const CAPTURE_NO_PROXY: &str = "127.0.0.1,localhost";

/// The only names a capture launch carries across from the tracker's own environment
/// (the allowlist; `HOME`, `PATH` and `SHELL` are set, not carried).
pub const CAPTURE_IDENTITY_ENV: [&str; 3] = ["USER", "LOGNAME", "TMPDIR"];

/// The longest provider key a capture launch hands a child.
pub const MAX_CHILD_SECRET_BYTES: usize = 256;

/// The provider credential of a capture launch, held for the child alone (D40, D41).
///
/// Made only by [`child_secret_for_harness`], from the variable the profile's credential
/// names. It is applied by the one command builder, after the plan's own environment, and by
/// nothing else: it is never in a [`LaunchPlan`], an argv, a TOML file, a log, the registered
/// handle or any evidence. It is not `Clone` and not `Serialize`, its `Debug` prints the
/// variable's name and no value, and its bytes are overwritten when it drops.
///
/// Zeroing is hygiene and not a boundary: the value stays in this process's environment for the
/// process's whole life, where any process of the same user can read it, which D37 accepts. What
/// this type keeps is the value out of every place this crate would otherwise print or store it.
pub struct ChildSecret {
    name: &'static str,
    /// Printable ASCII of `[A-Za-z0-9_-]` only, checked when it was made.
    value: Vec<u8>,
}

impl ChildSecret {
    /// The variable the child receives it in.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The value, for the one command builder and the key scan. Not public: a caller outside
    /// this crate has no use for it.
    pub(crate) fn expose(&self) -> &[u8] {
        &self.value
    }

    fn value_str(&self) -> &str {
        // Checked as ASCII when made; a value that somehow is not is an empty variable, never
        // a panic.
        std::str::from_utf8(&self.value).unwrap_or("")
    }
}

impl fmt::Debug for ChildSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChildSecret({}={REDACTED})", self.name)
    }
}

impl Drop for ChildSecret {
    fn drop(&mut self) {
        crate::key_scan::wipe(&mut self.value);
    }
}

/// The child-only credential of `profile`, read from the launcher's own `env`, or `None` for a
/// profile that names none (Codex, Claude Code).
///
/// The value must be one provider-key token: [`crate::key_scan::MIN_KEY_BYTES`] to
/// [`MAX_CHILD_SECRET_BYTES`] bytes of `[A-Za-z0-9_-]`, which is every key shape the closed
/// [`CAPTURE_CREDENTIAL_ENV`] list admits and nothing a shell could reinterpret. A missing or
/// unusable value is a [`CliError::Refused`] that names the variable and never the value, so a
/// run with no key is refused before the daemon is asked for anything.
/// The child-only provider credential declared by a schema-1 capture profile.
pub fn child_secret_for_harness(
    profile: &CaptureSpec,
    env: &BTreeMap<String, String>,
) -> Result<Option<ChildSecret>, CliError> {
    child_secret_named(profile.credential.as_ref().map(|c| c.env.as_str()), env)
}

fn child_secret_named(
    wanted: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<Option<ChildSecret>, CliError> {
    let Some(wanted) = wanted else {
        return Ok(None);
    };
    let Some(name) = goalcup_protocol::capture_policy::CREDENTIAL_ENV
        .iter()
        .find(|n| **n == wanted)
    else {
        return Err(CliError::Internal(format!(
            "the capture profile names `{wanted}` as its credential, which is not a variable this build hands a child"
        )));
    };
    let Some(value) = env.get(*name) else {
        return Err(CliError::Refused(format!(
            "`{name}` is not set in the environment `goalcup start` was run from. The harness's \
             provider key is read from it and handed to the harness only. Nothing was created."
        )));
    };
    let bytes = value.as_bytes();
    let usable = (crate::key_scan::MIN_KEY_BYTES..=MAX_CHILD_SECRET_BYTES).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if !usable {
        return Err(CliError::Refused(format!(
            "`{name}` is not a usable provider key: it must be {} to {MAX_CHILD_SECRET_BYTES} \
             characters of letters, digits, `_` and `-`, with no quotes, spaces or line breaks. \
             Nothing was created.",
            crate::key_scan::MIN_KEY_BYTES
        )));
    }
    Ok(Some(ChildSecret {
        name,
        value: bytes.to_vec(),
    }))
}

/// A harness executable whose bytes were hashed and matched a capture profile's pin.
///
/// Only [`pin_harness_executable`] makes one, so a capture launch cannot be built for an
/// executable nobody checked. After durable process registration and immediately before gate
/// release, the launcher re-opens this exact canonical path and streams it into SHA-256 again.
/// That read is neither a coherent snapshot nor inode-bound execution: same-UID mutation from
/// the first read or pathname replacement before `exec` remains a stated residual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedExecutable {
    path: String,
    sha256: String,
}

impl PinnedExecutable {
    /// The absolute path that was hashed, which is the path the capture launch runs.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The lowercase SHA-256 of its bytes.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Hash the executable at `path` and accept it only if it is `profile`'s pinned one.
///
/// `path` is resolved by the caller: the measured recipes run the native binary (not the
/// Codex npm wrapper) and the versioned Claude Code binary (not the symlink its updater
/// moves), whose location depends on how the harness was installed. The bytes decide, and
/// the kind of file is checked on the open handle.
///
/// **A symlink is refused, not followed** (exec the versioned path
/// directly). `~/.local/bin/claude` hashes right today and is retargeted by the next
/// updater, so a pin over the link would bless whatever it names at spawn time. Directory
/// links above the file (`/var` → `/private/var`) are resolved, and the pinned path is the
/// canonical one; the file itself is then opened without following a link, so a name
/// swapped for a link after the check is refused by the open.
///
/// Refusals: a relative or non-UTF-8 path, a symlink, anything but a regular file, or bytes
/// that are not the pinned ones — [`CliError::Refused`], naming the path (and both digests
/// for a mismatch); a file this process cannot read — [`CliError::Io`].
pub fn pin_harness_executable(
    profile: &CaptureSpec,
    path: &Path,
) -> Result<PinnedExecutable, CliError> {
    pin_executable_fields(
        &profile.executable.version,
        &profile.executable.sha256,
        path,
    )
}

fn pin_executable_fields(
    version: &str,
    expected_sha256: &str,
    path: &Path,
) -> Result<PinnedExecutable, CliError> {
    let Some(given) = path.to_str().filter(|_| path.is_absolute()) else {
        return Err(CliError::Refused(format!(
            "the capture executable must be an absolute UTF-8 path, and `{}` is not. Nothing is \
             launched.",
            path.display()
        )));
    };
    let io = |source| CliError::Io {
        path: given.to_string(),
        source,
    };
    if std::fs::symlink_metadata(path)
        .map_err(io)?
        .file_type()
        .is_symlink()
    {
        return Err(CliError::Refused(format!(
            "the capture executable `{given}` is a symlink; name the versioned file it points \
             at, which is what the capture recipe runs. Nothing is launched."
        )));
    }
    let canonical = std::fs::canonicalize(path).map_err(io)?;
    let Some(text) = canonical.to_str() else {
        return Err(CliError::Refused(format!(
            "the capture executable `{given}` resolves to a path that is not UTF-8. Nothing is \
             launched."
        )));
    };
    let sha256 = sha256_regular_file_no_follow(&canonical, given)?;
    if !sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(CliError::Refused(format!(
            "`{text}` is not the harness executable this capture profile was measured with \
             (version {}): its sha256 is {sha256}, the profile pins {}. Install that exact \
             version and try again. Nothing is launched.",
            version, expected_sha256
        )));
    }
    Ok(PinnedExecutable {
        path: text.to_string(),
        sha256,
    })
}

/// Re-open the exact canonical path that was pinned, without canonicalising it again, and hash
/// the whole regular file. This is the last check before the parked capture child is released.
pub fn recheck_pinned_executable(executable: &PinnedExecutable) -> Result<(), CliError> {
    let path = Path::new(executable.path());
    let observed = sha256_regular_file_no_follow(path, executable.path()).map_err(|_| {
        CliError::Refused(format!(
            "the capture executable `{}` changed after process registration and could not be \
             re-opened as the pinned regular file. Nothing is launched.",
            executable.path()
        ))
    })?;
    if observed != executable.sha256() {
        return Err(CliError::Refused(format!(
            "the capture executable `{}` changed after process registration: its sha256 is \
             {observed}, the pinned sha256 is {}. Nothing is launched.",
            executable.path(),
            executable.sha256()
        )));
    }
    Ok(())
}

/// Hash all bytes from one exact regular-file path. The final component is opened read-only,
/// nonblocking and without following a symlink, so a substituted FIFO cannot strand the parked
/// child before `metadata` rejects it. Callers decide whether the path may first be canonicalised.
fn sha256_regular_file_no_follow(path: &Path, display: &str) -> Result<String, CliError> {
    let io = |source| CliError::Io {
        path: display.to_string(),
        source,
    };
    let mut file = open_no_follow(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(CliError::Refused(format!(
            "the capture executable `{display}` is not a regular file. Nothing is launched."
        )));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// Open `path` for reading, refusing (with `ELOOP`) rather than following a symlink at its
/// last component.
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options.open(path)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a capture launch needs that its connector's profile cannot say.
///
/// `Debug` prints the goal's length and the proxy URL without its userinfo, never either
/// value.
pub struct CaptureLaunch<'a> {
    /// The initial goal, exactly as read. Appended as one argument after the template's
    /// closing `--`, or as the value of the profile's one `--flag=<goal>` element: no shell, no
    /// quoting, no splitting.
    pub goal: &'a [u8],
    pub executable: &'a PinnedExecutable,
    /// The per-run `HOME`, created by the caller in the shape the profile's `home` names.
    pub run_home: &'a Path,
    /// The isolated harness home `launch.home_env` points at (the profile's `config_home`).
    pub config_home: &'a Path,
    /// `http://goalcup:<credential>@<127.0.0.1|[::1]|localhost>:<port>`, the proxy this launch
    /// records through ([`crate::proxy::connect::ProxyCredential::proxy_url`]'s shape).
    pub proxy_url: &'a str,
    /// The proxy's per-launch CA certificate, handed to each of the profile's `ca_env`.
    pub ca_certificate: &'a Path,
    /// The tracker's own environment. Only [`CAPTURE_IDENTITY_ENV`] is read from it.
    pub parent_env: &'a BTreeMap<String, String>,
    /// The one validated Claude command-text allowance for this launch.
    pub allowed_tool_rule: Option<&'a str>,
}

impl fmt::Debug for CaptureLaunch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureLaunch")
            .field("goal", &format_args!("<{} bytes>", self.goal.len()))
            .field("executable", &self.executable)
            .field("run_home", &self.run_home)
            .field("config_home", &self.config_home)
            .field(
                "proxy_url",
                &without_userinfo(self.proxy_url).unwrap_or_else(|| self.proxy_url.to_string()),
            )
            .field("ca_certificate", &self.ca_certificate)
            .field("parent_env", &Keys(self.parent_env))
            .field("allowed_tool_rule", &self.allowed_tool_rule)
            .finish()
    }
}

/// Why the Claude capture tool allowance `rule` cannot be used with connector `c`, whose capture
/// arguments are `args` (the profile's own, or the same rendered): `None` when it can. The one
/// predicate behind both the refusal before anything is created and the plan's own check, so the
/// two cannot drift, and each refusal says which condition failed.
/// The connector id is the only retained product-specific allowance; every other condition is
/// checked against the manifest argv.
pub fn allowance_refusal(
    c: &HarnessConnector,
    args: &[String],
    rule: &str,
) -> Option<&'static str> {
    let has = |flag: &str, value: &str| {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    };
    if c.loaded.manifest.id != "claude-code-cli" {
        Some("only the pinned claude-code-cli profile may carry one")
    } else if !goalcup_protocol::capture::is_claude_capture_allowed_tool_rule(rule) {
        Some("the rule is not one bounded `Bash(python3 <name>.py *)`")
    } else if args.iter().any(|arg| arg == "--allowedTools") {
        Some("the profile already has an --allowedTools of its own")
    } else if args.last().map(String::as_str) != Some("--") {
        Some("the profile's arguments do not end at `--`, so the allowance has no place to go")
    } else if !has("--permission-mode", "acceptEdits") || !has("--permission-prompts", "none") {
        Some(
            "the profile is not in the pinned mode (`--permission-mode acceptEdits`, `--permission-prompts none`)",
        )
    } else {
        None
    }
}

/// The goal as one argument: UTF-8, non-empty, NUL-free and bounded.
pub(crate) fn checked_goal(goal: &[u8]) -> Result<&str, CliError> {
    let refuse = |why: &str| {
        Err(CliError::Refused(format!(
            "the goal ({} bytes) {why}. Nothing is launched.",
            goal.len()
        )))
    };
    if goal.len() > MAX_GOAL_BYTES {
        return refuse(&format!("is longer than {MAX_GOAL_BYTES} bytes"));
    }
    let Ok(text) = std::str::from_utf8(goal) else {
        return refuse("is not valid UTF-8");
    };
    if text.contains('\0') {
        return refuse("holds a NUL byte, which no argument can carry");
    }
    if text.trim().is_empty() {
        return refuse("is empty or only white space");
    }
    Ok(text)
}

/// The capture launch rendered from harness-contract schema 1.
pub fn build_capture_launch(
    c: &HarnessConnector,
    v: &TemplateVars,
    workspace: &Path,
    launch: &CaptureLaunch<'_>,
) -> Result<LaunchPlan, CliError> {
    let id = &c.loaded.manifest.id;
    let profile = &c.loaded.manifest.capture;
    if !launch
        .executable
        .sha256
        .eq_ignore_ascii_case(&profile.executable.sha256)
    {
        return Err(CliError::Refused(format!(
            "the executable was pinned for another capture profile, not `{id}`'s. Nothing is \
             launched."
        )));
    }
    let goal = checked_goal(launch.goal)?;
    for (what, value) in [
        ("workspace", Some(workspace.to_string_lossy())),
        (
            "{workspace}",
            v.get("workspace").map(std::borrow::Cow::Borrowed),
        ),
    ] {
        if let Some(value) = value
            && value
                .chars()
                .any(|c| c == '"' || c == '\\' || c.is_control())
        {
            return Err(CliError::Refused(format!(
                "the {what} path holds a quote, backslash or control character, which the \
                 capture recipe cannot pass through its configuration syntax. Nothing is \
                 launched."
            )));
        }
    }
    let run_home = absolute_utf8("per-run HOME", launch.run_home)?;
    let config_home = absolute_utf8("harness home", launch.config_home)?;
    let ca_certificate = absolute_utf8("proxy CA certificate", launch.ca_certificate)?;
    loopback_proxy_url(launch.proxy_url)?;
    needs_a_port_it_has(&profile.args, &profile.env, v, id)?;

    let mut argv = Vec::with_capacity(profile.args.len() + 1);
    for template in &profile.args {
        let arg = render(template, v)?;
        if let Some(why) = harness_arg_refusal(profile, &arg) {
            return Err(CliError::Refused(format!(
                "`{arg}` {why}. Nothing is launched."
            )));
        }
        argv.push(arg);
    }
    let ends_in_separator = argv.last().map(String::as_str) == Some("--");
    if ends_in_separator == profile.goal_flag.is_some() {
        return Err(CliError::Internal(format!(
            "`{id}`'s capture args place the goal neither one way nor the other"
        )));
    }
    if let Some(rule) = launch.allowed_tool_rule {
        if let Some(why) = allowance_refusal(c, &argv, rule) {
            return Err(CliError::Refused(format!(
                "the Claude capture tool allowance cannot be used: {why}. Nothing is launched."
            )));
        }
        let before_goal = argv.len() - 1;
        argv.splice(
            before_goal..before_goal,
            ["--allowedTools".into(), rule.into()],
        );
    }
    let goal_prefix = profile.goal_flag.as_ref().map_or(0, |flag| flag.len() + 1);
    match &profile.goal_flag {
        Some(flag) => argv.push(format!("{flag}={goal}")),
        None => argv.push(goal.to_string()),
    }
    let goal_index = argv.len() - 1;

    let mut env = BTreeMap::new();
    for (key, template) in &profile.env {
        env.insert(key.clone(), render(template, v)?);
    }
    for key in CAPTURE_IDENTITY_ENV {
        if let Some(value) = launch.parent_env.get(key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    env.insert("HOME".into(), run_home.to_string());
    env.insert("PATH".into(), CAPTURE_PATH.into());
    env.insert(profile.home.env.clone(), config_home.to_string());
    for key in ["HTTPS_PROXY", "https_proxy"] {
        env.insert(key.into(), launch.proxy_url.to_string());
    }
    for key in ["NO_PROXY", "no_proxy"] {
        env.insert(key.into(), CAPTURE_NO_PROXY.into());
    }
    for key in &profile.ca.env {
        env.insert(key.clone(), ca_certificate.to_string());
    }

    let termination = c.loaded.manifest.termination();
    let grace_seconds = termination.grace_seconds();
    let strategy = match termination.strategy() {
        Strategy::ProcessGroup => goalcup_protocol::manifest::KillStrategy::ProcessGroup,
        Strategy::NativePid => goalcup_protocol::manifest::KillStrategy::NativePid,
        _ => {
            return Err(CliError::Refused(
                "the harness uses a termination strategy this goalcup does not support".into(),
            ));
        }
    };
    let plan = LaunchPlan {
        program: launch.executable.path.clone(),
        argv,
        env_set: env,
        env_remove: Vec::new(),
        env_base: BTreeMap::new(),
        cwd: workspace.to_path_buf(),
        mode: LaunchMode::Server,
        kill: Kill {
            strategy,
            native_binary: termination.native_binary,
            grace_seconds,
        },
        capture: Some(HeadlessCapture {
            goal_index,
            goal_prefix,
        }),
    };
    let credential = profile.credential.as_ref().map_or(0, |credential| {
        (credential.env.len() + 1 + MAX_CHILD_SECRET_BYTES + 1 + 8) as u64
    });
    let needed = exec_footprint(&plan) + credential;
    let limit = exec_limit();
    if needed > limit {
        return Err(CliError::Refused(format!(
            "`{id}`'s capture launch needs {needed} bytes of argument and environment space \
             and this system allows {limit} (ARG_MAX). Nothing is launched."
        )));
    }
    Ok(plan)
}

fn absolute_utf8<'a>(what: &str, path: &'a Path) -> Result<&'a str, CliError> {
    match path.to_str() {
        Some(text) if path.is_absolute() && !text.contains('\0') => Ok(text),
        _ => Err(CliError::Refused(format!(
            "the {what} must be an absolute UTF-8 path, and `{}` is not",
            path.display()
        ))),
    }
}

/// The only proxy a capture launch may name (AGENTS.md, D1's M2 reservation), in exactly the
/// shape [`crate::proxy::connect::ProxyCredential::proxy_url`] writes:
/// `http://goalcup:<32 lowercase hex>@<host>:<port>`, where `<host>` is `127.0.0.1`,
/// `[::1]` or `localhost` and `<port>` is a decimal in 1–65535. Nothing else.
///
/// Parsed as one whole authority and matched exactly, not searched: the userinfo is the
/// text before the first `@`, the port the text after the last `:`, and userinfo, host and
/// port must each be an allowed spelling in full. A looser reading is how
/// `http://evil:3128/@127.0.0.1:1` looks loopback to a check and is a CONNECT to `evil` to
/// every URL parser a harness runs. The refusal does not quote the URL, which carries the
/// proxy credential.
fn loopback_proxy_url(url: &str) -> Result<(), CliError> {
    if is_loopback_proxy_url(url) {
        Ok(())
    } else {
        Err(CliError::Refused(format!(
            "the capture proxy must be exactly http://{PROXY_USERNAME}:<credential>@<host>:<port> \
             with host 127.0.0.1, [::1] or localhost"
        )))
    }
}

fn is_loopback_proxy_url(url: &str) -> bool {
    let Some(authority) = url.strip_prefix("http://") else {
        return false;
    };
    // Each part below is compared in full against the few allowed spellings, so a `/`, `?`,
    // `#`, `\`, `%`, space or second `@` anywhere leaves a part that matches none of them.
    let Some((userinfo, host_port)) = authority.split_once('@') else {
        return false;
    };
    let secret = userinfo
        .strip_prefix(PROXY_USERNAME)
        .and_then(|rest| rest.strip_prefix(':'));
    let credential_ok = secret.is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    let Some((host, port)) = host_port.rsplit_once(':') else {
        return false;
    };
    let port_ok = !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p != 0);
    credential_ok && port_ok && matches!(host, "127.0.0.1" | "[::1]" | "localhost")
}

/// The bytes `execve` needs for this plan, by the measured formula
/// `Σ(len(argv_i)+1) + Σ(len(env_j)+1) + 8·(argc+envc+2)`, with the program as `argv_0`,
/// plus the image path once more (its share was not measured, so it is counted).
///
/// The environment counted is the one [`spawn`] builds: [`LaunchPlan::env_base`] minus
/// [`LaunchPlan::env_remove`], then [`LaunchPlan::env_set`].
pub fn exec_footprint(p: &LaunchPlan) -> u64 {
    let mut env: BTreeMap<&str, &str> = p
        .env_base
        .iter()
        .filter(|(k, _)| !p.env_remove.contains(k))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for (k, v) in &p.env_set {
        env.insert(k, v);
    }
    let argv: u64 = std::iter::once(&p.program)
        .chain(p.argv.iter())
        .map(|a| a.len() as u64 + 1)
        .sum();
    let envs: u64 = env
        .iter()
        .map(|(k, v)| (k.len() + 1 + v.len() + 1) as u64)
        .sum();
    let pointers = 8 * (p.argv.len() as u64 + 1 + env.len() as u64 + 2);
    argv + envs + pointers + p.program.len() as u64 + 1
}

/// This system's `ARG_MAX`, the bound [`exec_footprint`] is held to. `_POSIX_ARG_MAX`
/// (4096) where the system does not say, which refuses rather than guesses.
pub fn exec_limit() -> u64 {
    #[cfg(unix)]
    {
        // SAFETY: `sysconf` takes an integer name and reads or writes no memory we own.
        let v = unsafe { libc::sysconf(libc::_SC_ARG_MAX) };
        if v > 0 {
            return v as u64;
        }
    }
    4096
}

/// Whether this plan's child is put at the head of a process group of its own.
///
/// A capture launch is not, although it is headless: a group handle is never confirmed ended
/// (`docs/TERMINATION.md`, D30), so the daemon ends it by the descendant walk, and in the
/// launcher's group a Ctrl-C typed at the terminal reaches it too.
fn leads_its_group(p: &LaunchPlan) -> bool {
    owns_its_group(p.mode) && p.capture.is_none()
}

/// Whether this mode's launch may own a process group of its own.
///
/// The whole of the difference between the three modes. See the module header for why a
/// `Tui` may not.
fn owns_its_group(mode: LaunchMode) -> bool {
    match mode {
        LaunchMode::Tui => false,
        LaunchMode::Server => true,
        // Unreachable: a capture plan is always `Server`, and the V1 `plan` refused this
        // mode. Answered rather than panicked, because a `match` that cannot fail is worth
        // more than an arm that can.
        LaunchMode::AttachOnly => false,
    }
}

/// POSIX single-quoting, applied only where it changes anything.
///
/// `pub(crate)` because `goalcup seal` prints a `goalcup-verify --capture-integrity <dir>` line for
/// the participant to paste, and a second quoting rule written beside this one is the one
/// that gets a path with a space in it wrong.
///
/// A string of characters no shell treats specially is left alone, so the printed line reads
/// like the command it is — `--settings /tmp/x/settings-launch.json` and not
/// `'--settings' '/tmp/x/settings-launch.json'`. Anything else is wrapped in single quotes
/// with embedded quotes spliced the only way a POSIX shell allows (`'\''`), which is safe
/// for every byte including newlines.
pub(crate) fn quote(s: &str) -> String {
    const SAFE: &str = "@%+=:,./-_";
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || SAFE.contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The harness, started.
///
/// **The child's stdio is not touched.** A TUI owns the participant's terminal; capturing
/// its stdout would leave them looking at a harness that draws nothing, and this CLI has no
/// use for the bytes. Inheritance is the behaviour, and it is spelled by the absence of a
/// `Stdio` call rather than by one — the same reason the version probe, which genuinely
/// needs pipes, says so explicitly.
///
/// The environment is built rather than inherited: `env_clear`, then
/// [`LaunchPlan::env_base`], then the removals, then the pins. The order is fixed so a
/// connector that named a variable on both the removal and the pin list gets the pin rather
/// than a result that depends on `BTreeMap` iteration.
///
/// **A `setpgid` that fails is fatal for a launch that owns its group, by design.** There is
/// no fallback arm: [`LaunchMode::Server`] promises the daemon a `pgid` it can signal, and a
/// server started outside the group it was promised in is a process the terminator would miss
/// at the deadline. `Command::process_group` reports that failure as a spawn error, so the
/// launch stops with a child that never started rather than continuing with a handle that is
/// wrong about the one field it exists to carry.
pub fn spawn(p: &LaunchPlan) -> Result<Spawned, CliError> {
    let (mut command, program) = build_command(p, Pipes::default(), None);
    let child = command.spawn().map_err(|source| CliError::Io {
        path: program.clone(),
        source,
    })?;
    Ok(Spawned { child, program })
}

/// Which of a capture child's output streams are piped to this process; the rest are
/// inherited. Codex pipes stdout (its scope tee); Hermes pipes both (stdout and stderr are
/// forwarded verbatim and scanned for key fragments, and stderr carries its session marker).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pipes {
    pub stdout: bool,
    pub stderr: bool,
}

/// The manifest rule for selecting a session identity from stdout JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdoutJsonRule {
    pub match_field: String,
    pub match_value: String,
    pub id_field: String,
}

/// The manifest rule for selecting a session identity from one stderr line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StderrLineRule {
    pub prefix: String,
    pub grammar: IdGrammar,
}

/// How the capture launcher consumes stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdoutUse {
    Inherit,
    Forward,
    IdentityJson(StdoutJsonRule),
}

/// How the capture launcher consumes stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StderrUse {
    Inherit,
    Forward,
    IdentityLine(StderrLineRule),
}

/// The child-output decision derived from one validated harness capture profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPlan {
    pub scan_for_key: bool,
    pub stdout: StdoutUse,
    pub stderr: StderrUse,
}

impl StreamPlan {
    pub fn for_spec(spec: &CaptureSpec) -> Result<Self, CliError> {
        let scan_for_key = spec.credential.is_some();
        let (stdout, stderr) = match spec.identity.source {
            IdentitySource::Preassigned | IdentitySource::None => (
                if scan_for_key {
                    StdoutUse::Forward
                } else {
                    StdoutUse::Inherit
                },
                if scan_for_key {
                    StderrUse::Forward
                } else {
                    StderrUse::Inherit
                },
            ),
            IdentitySource::StdoutJson => (
                StdoutUse::IdentityJson(StdoutJsonRule {
                    match_field: required_identity_field(
                        spec.identity.match_field.as_deref(),
                        "match_field",
                    )?,
                    match_value: required_identity_field(
                        spec.identity.match_value.as_deref(),
                        "match_value",
                    )?,
                    id_field: required_identity_field(
                        spec.identity.id_field.as_deref(),
                        "id_field",
                    )?,
                }),
                if scan_for_key {
                    StderrUse::Forward
                } else {
                    StderrUse::Inherit
                },
            ),
            IdentitySource::StderrLine => (
                if scan_for_key {
                    StdoutUse::Forward
                } else {
                    StdoutUse::Inherit
                },
                StderrUse::IdentityLine(StderrLineRule {
                    prefix: required_identity_field(spec.identity.prefix.as_deref(), "prefix")?,
                    grammar: spec.identity.grammar.ok_or_else(|| {
                        CliError::Internal(
                            "validated harness identity stderr_line has no grammar".into(),
                        )
                    })?,
                }),
            ),
            _ => {
                return Err(CliError::Refused(
                    "the harness uses an identity source this goalcup does not support".into(),
                ));
            }
        };
        Ok(Self {
            scan_for_key,
            stdout,
            stderr,
        })
    }

    pub fn pipes(&self) -> Pipes {
        Pipes {
            stdout: !matches!(self.stdout, StdoutUse::Inherit),
            stderr: !matches!(self.stderr, StderrUse::Inherit),
        }
    }
}

fn required_identity_field(value: Option<&str>, field: &str) -> Result<String, CliError> {
    value.map(str::to_owned).ok_or_else(|| {
        CliError::Internal(format!("validated harness identity rule has no {field}"))
    })
}

/// When the daemon may bind the native store, derived only from the identity source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchScope {
    Preassigned,
    FromStdout,
    AtFinish,
    None,
}

impl LaunchScope {
    pub fn for_spec(spec: &CaptureSpec) -> Result<Self, CliError> {
        match spec.identity.source {
            IdentitySource::Preassigned => Ok(Self::Preassigned),
            IdentitySource::StdoutJson => Ok(Self::FromStdout),
            IdentitySource::StderrLine => Ok(Self::AtFinish),
            IdentitySource::None => Ok(Self::None),
            _ => Err(CliError::Refused(
                "the harness uses an identity source this goalcup does not support".into(),
            )),
        }
    }
}

/// Why one argument is denied by the schema-1 harness profile.
pub fn harness_arg_refusal(spec: &CaptureSpec, arg: &str) -> Option<&'static str> {
    spec.argv_guard.as_ref().and_then(|guard| guard.denies(arg))
}

/// The command a launch runs, and the program path it was resolved to: argv, working
/// directory, the built environment, `/dev/null` for a capture child's stdin, the process
/// group a server leads, and the requested pipes. [`spawn`] and [`spawn_gated`] both run what
/// this builds, so a gated capture child's argv, environment and stdio are exactly an
/// ungated one's. The child-only `secret` is set here, last, and is the one place it is.
fn build_command(p: &LaunchPlan, pipes: Pipes, secret: Option<&ChildSecret>) -> (Command, String) {
    let program = resolved_program(&p.program, &p.env_base);
    let mut command = Command::new(&program);
    command.args(&p.argv).current_dir(&p.cwd);
    command.env_clear();
    for (k, v) in &p.env_base {
        command.env(k, v);
    }
    for k in &p.env_remove {
        command.env_remove(k);
    }
    for (k, v) in &p.env_set {
        command.env(k, v);
    }
    if let Some(secret) = secret {
        command.env(secret.name(), secret.value_str());
    }
    // A capture launch is headless and has no input channel: both measured harnesses append
    // piped stdin to the goal, so the child never inherits ours.
    if p.capture.is_some() {
        command.stdin(std::process::Stdio::null());
    }
    // The one line the three modes differ in. A TUI never reaches it, and nor does a capture
    // launch (see `leads_its_group`).
    #[cfg(unix)]
    if leads_its_group(p) {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if pipes.stdout {
        command.stdout(std::process::Stdio::piped());
    }
    if pipes.stderr {
        command.stderr(std::process::Stdio::piped());
    }
    (command, program)
}

/// The two bounded messages from a capture stdout tee: its one scope identity and its
/// completion after EOF and all forwarding. Waiting for completion never joins the thread.
/// A keyed `stdout_json` launch has no such tee: its stdout is read by the key-scanning tee,
/// which hands back only the scope receiver and reports the stream's end itself.
pub struct CaptureStdout {
    pub scope: std::sync::mpsc::Receiver<Option<String>>,
    completed: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

impl CaptureStdout {
    /// `true` confirms completion; `false` is explicit deadline expiry. A descendant may
    /// hold stdout open, or destination back-pressure may still hold the final bytes.
    pub fn finish(&self, deadline: std::time::Instant) -> std::io::Result<bool> {
        match self
            .completed
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            Ok(result) => result.map(|()| true),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(std::io::Error::other(
                "capture stdout tee ended without completion",
            )),
        }
    }
}

/// Start the same tee used by the capture child. The completion message is sent only after
/// the last read has been forwarded (or discarded for a closed destination) and EOF observed.
pub fn tee_capture_stdout(
    reader: impl std::io::Read + Send + 'static,
    writer: impl std::io::Write + Send + 'static,
) -> std::io::Result<CaptureStdout> {
    tee_capture_stdout_with_rule(
        reader,
        writer,
        StdoutJsonRule {
            match_field: "type".into(),
            match_value: "thread.started".into(),
            id_field: "thread_id".into(),
        },
    )
}

/// Start the stdout tee with the identity paths declared by the harness manifest.
pub fn tee_capture_stdout_with_rule(
    reader: impl std::io::Read + Send + 'static,
    writer: impl std::io::Write + Send + 'static,
    rule: StdoutJsonRule,
) -> std::io::Result<CaptureStdout> {
    let (sender, scope) = std::sync::mpsc::channel();
    let (done, completed) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("capture-stdout".into())
        .spawn(move || {
            let result = stdout_tee_with_rule(reader, writer, sender, &rule);
            let _ = done.send(result);
        })?;
    Ok(CaptureStdout { scope, completed })
}

/// Codex's scope binding uses stdout without changing a byte of participant output.
/// Started on a child the gate has already released (D40), never a spawner: the tee remains
/// detached, but its completion is observed within the finish deadline.
pub fn tee_scoped_capture(spawned: &mut Spawned) -> Result<CaptureStdout, CliError> {
    tee_scoped_capture_with_rule(
        spawned,
        StdoutJsonRule {
            match_field: "type".into(),
            match_value: "thread.started".into(),
            id_field: "thread_id".into(),
        },
    )
}

/// Scope tee using the manifest's stdout JSON identity rule.
pub fn tee_scoped_capture_with_rule(
    spawned: &mut Spawned,
    rule: StdoutJsonRule,
) -> Result<CaptureStdout, CliError> {
    let Some(reader) = spawned.child.stdout.take() else {
        let _ = spawned.child.kill();
        let _ = spawned.child.wait();
        return Err(CliError::Io {
            path: "capture stdout tee".into(),
            source: std::io::Error::other("the harness's stdout was not piped"),
        });
    };
    match tee_capture_stdout_with_rule(reader, std::io::stdout(), rule) {
        Ok(stdout) => Ok(stdout),
        Err(source) => {
            let _ = spawned.child.kill();
            let _ = spawned.child.wait();
            Err(CliError::Io {
                path: "capture stdout tee".into(),
                source,
            })
        }
    }
}

/// Forward every byte, scanning complete lines of at most 1 MiB until the first
/// `thread.started`. A closed destination turns forwarding into draining, not a child error.
pub fn stdout_tee(
    reader: impl std::io::Read,
    writer: impl std::io::Write,
    sender: std::sync::mpsc::Sender<Option<String>>,
) -> std::io::Result<()> {
    stdout_tee_with_rule(
        reader,
        writer,
        sender,
        &StdoutJsonRule {
            match_field: "type".into(),
            match_value: "thread.started".into(),
            id_field: "thread_id".into(),
        },
    )
}

/// Forward stdout unchanged while scanning no more than 64 complete lines for the manifest's
/// first matching identity event. A matching event ends the scan even when its id is unusable.
pub fn stdout_tee_with_rule(
    mut reader: impl std::io::Read,
    mut writer: impl std::io::Write,
    sender: std::sync::mpsc::Sender<Option<String>>,
    rule: &StdoutJsonRule,
) -> std::io::Result<()> {
    let mut scan = StdoutIdentityScan::new(rule.clone(), sender);
    let mut forwarding = true;
    let mut buf = [0u8; 64 * 1024];
    let result = loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e),
        };
        // Announce the scope before a participant's stdout back-pressure can delay Launch.
        scan.feed(&buf[..n]);
        if forwarding && (writer.write_all(&buf[..n]).is_err() || writer.flush().is_err()) {
            forwarding = false;
        }
    };
    scan.end();
    result
}

/// The stdout JSON identity matcher, fed every byte one stdout reader reads: complete lines of
/// at most 1 MiB, at most 64 of them, until the first event whose `match_field` is
/// `match_value`. It sends exactly one message: that event's `id_field` (or `None` when the id
/// is unusable), or `None` when the line budget or the stream ends first.
///
/// For a keyed launch it also holds a key-fragment matcher of its own ([`Self::with_key_scan`]).
/// The id it sends is what the daemon writes into the chain as the scope, so a matching event
/// whose line or decoded id holds any key-scan form is not bound: it sends `None`, the same
/// unbound path an unusable id takes, and records a key hit, which fails the row. The line
/// buffer is overwritten before it is cleared and when the matcher is dropped, as the tee's own
/// buffer is.
pub(crate) struct StdoutIdentityScan {
    rule: StdoutJsonRule,
    sender: std::sync::mpsc::Sender<Option<String>>,
    line: Vec<u8>,
    overlong: bool,
    scanning: bool,
    complete_lines: usize,
    key_scan: Option<(
        crate::key_scan::FragmentMatcher,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    )>,
}

impl StdoutIdentityScan {
    const LINE_LIMIT: usize = 1024 * 1024;
    const LINE_COUNT_LIMIT: usize = 64;

    pub(crate) fn new(
        rule: StdoutJsonRule,
        sender: std::sync::mpsc::Sender<Option<String>>,
    ) -> Self {
        Self {
            rule,
            sender,
            line: Vec::new(),
            overlong: false,
            scanning: true,
            complete_lines: 0,
            key_scan: None,
        }
    }

    /// Refuse to bind an id that holds a fragment of the launch's key, and set `hit` when a
    /// matching event held one.
    pub(crate) fn with_key_scan(
        mut self,
        matcher: crate::key_scan::FragmentMatcher,
        hit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        self.key_scan = Some((matcher, hit));
        self
    }

    /// Empty the held line, overwriting it first when this scan holds key material (a keyed
    /// launch's line may hold a fragment of the key).
    fn clear_line(&mut self) {
        if self.key_scan.is_some() {
            crate::key_scan::wipe(&mut self.line);
        }
        self.line.clear();
    }

    /// The id to send for a matching event: `None` when it is unusable, and `None` with a key
    /// hit recorded when the line or the id holds a fragment of the key.
    fn bound_id(&self, id: Option<&str>) -> Option<String> {
        if let Some((matcher, hit)) = &self.key_scan
            && (matcher.contains(&self.line)
                || id.is_some_and(|id| matcher.contains(id.as_bytes())))
        {
            hit.store(true, std::sync::atomic::Ordering::Release);
            return None;
        }
        id.map(str::to_owned)
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if !self.scanning {
            return;
        }
        for &byte in bytes {
            if byte == b'\n' {
                self.complete_lines += 1;
                if !self.overlong
                    && let Ok(event) = serde_json::from_slice::<serde_json::Value>(&self.line)
                    && json_path(&event, &self.rule.match_field).and_then(|v| v.as_str())
                        == Some(self.rule.match_value.as_str())
                {
                    let id = self
                        .bound_id(json_path(&event, &self.rule.id_field).and_then(|v| v.as_str()));
                    let _ = self.sender.send(id);
                    self.scanning = false;
                    self.clear_line();
                    return;
                }
                self.clear_line();
                self.overlong = false;
                if self.complete_lines == Self::LINE_COUNT_LIMIT {
                    let _ = self.sender.send(None);
                    self.scanning = false;
                    return;
                }
            } else if !self.overlong {
                if self.line.len() == Self::LINE_LIMIT {
                    self.clear_line();
                    self.overlong = true;
                } else {
                    self.line.push(byte);
                }
            }
        }
    }

    /// The stream ended (or its reader failed) before a matching event: say so, once.
    pub(crate) fn end(&mut self) {
        if self.scanning {
            let _ = self.sender.send(None);
            self.scanning = false;
        }
    }
}

impl Drop for StdoutIdentityScan {
    fn drop(&mut self) {
        self.end();
        self.clear_line();
    }
}

/// A stdout reader that feeds what it reads to a [`StdoutIdentityScan`] before handing it on,
/// so the key-scanning tee and the identity matcher share one reader of one pipe. The identity
/// is seen when the bytes are read, before the tee decides whether to forward them.
pub(crate) struct IdentityReader<R> {
    inner: R,
    scan: StdoutIdentityScan,
}

impl<R> IdentityReader<R> {
    pub(crate) fn new(inner: R, scan: StdoutIdentityScan) -> Self {
        Self { inner, scan }
    }
}

impl<R: std::io::Read> std::io::Read for IdentityReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            self.scan.end();
        } else {
            self.scan.feed(&buf[..n]);
        }
        Ok(n)
    }
}

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut at = value;
    for component in path.split('.') {
        if component.is_empty() {
            return None;
        }
        at = at.get(component)?;
    }
    Some(at)
}

/// Bound Codex's Launch delay by identity, stdout EOF, process exit or the spawn deadline.
/// The deadline is a parameter, not read here, so bounded tests can exercise the same wait.
pub fn wait_native_scope(
    child: &mut Child,
    receiver: &std::sync::mpsc::Receiver<Option<String>>,
    deadline: std::time::Instant,
) -> Option<String> {
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match receiver.recv_timeout(left.min(std::time::Duration::from_millis(20))) {
            Ok(scope) => return scope,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !matches!(child.try_wait(), Ok(None)) {
            return receiver.try_recv().ok().flatten();
        }
    }
}

/// A launched harness, and the program path it was launched from.
///
/// The two travel together because [`handle`] needs the second and must not go looking for it
/// again — see the module header. `program` is what [`spawn`] handed the OS: an absolute path
/// where the name resolved, and the manifest's name unchanged where it did not.
pub struct Spawned {
    pub child: Child,
    pub program: String,
}

/// `program` resolved on this process's `PATH`, or the name unchanged.
///
/// Resolved by [`crate::preflight::resolve_on_path`], the same walk the preflight's version
/// gate predicts the launch with, so the two cannot disagree about which binary a name
/// means — and against the `PATH` in the environment the child is being given, which is the
/// `PATH` that would do the lookup if this one did not.
///
/// Called **once**, by [`spawn`], whose answer [`handle`] then records. See the module
/// header for why a second lookup is a bug and not a convenience.
///
/// A name that does not resolve is handed to `Command` unchanged rather than refused: the
/// OS does its own lookup and produces the real error, and a `handle` that refused after the
/// child was already running would leave the daemon measuring — and terminating — a process
/// it was never told about, which is strictly worse than a program field that is a bare
/// name.
fn resolved_program(program: &str, env: &BTreeMap<String, String>) -> String {
    match crate::preflight::resolve_on_path(program, env) {
        Some(path) => path.display().to_string(),
        None => program.to_string(),
    }
}

/// What a [`ProcessHandle`] records in place of a capture launch's goal argument.
///
/// Fixed and not derived from the goal: the handle is persisted in the daemon's registry,
/// printed by `Debug` and folded into the registration digest, and a hash of a low-entropy
/// goal is a guessable stand-in for it (the goal is retained only as its
/// redacted evidence stream). The daemon reads `argv[0]` alone, so nothing it checks moves.
pub const GOAL_ARG_MARKER: &str = "<goal>";

/// The argv a handle records: the plan's, with a capture launch's goal element replaced by
/// [`GOAL_ARG_MARKER`].
fn recorded_argv(p: &LaunchPlan) -> Vec<String> {
    let mut argv = p.argv.clone();
    if let Some(capture) = p.capture
        && let Some(slot) = argv.get_mut(capture.goal_index)
    {
        // The flag that carries the goal stays, and the goal is the marker.
        let flag = slot.get(..capture.goal_prefix).unwrap_or("").to_string();
        *slot = format!("{flag}{GOAL_ARG_MARKER}");
    }
    argv
}

/// What the daemon is handed, built after the child exists.
///
/// `now` is passed in rather than read off the clock, so a caller that already stamped the
/// rest of a start with one instant does not stamp this with a second one.
///
/// The [`Spawned`] and not a bare `Child`: `program` is read off it, never looked up again.
pub fn handle(p: &LaunchPlan, s: &Spawned, now: &str) -> Result<ProcessHandle, CliError> {
    let pid = s.child.id();
    build_handle(p, pid, &s.program, || native_identity_of(p, pid), now)
}

/// [`handle`] for a capture child still parked before `exec` (D40).
///
/// The native hint is always absent: no native child can exist before the wrapper has run, and
/// a missing hint is a valid registration the daemon re-resolves at seal. `process_start` is
/// the fork-time creation identity, which `exec` does not change.
pub fn gated_handle(p: &LaunchPlan, g: &GatedSpawn, now: &str) -> Result<ProcessHandle, CliError> {
    // The only way this fails is that the parked child has no readable creation identity, which
    // on macOS is a child that already died (a zombie has none): it never ran.
    build_handle(p, g.pid, &g.program, || (None, None), now).map_err(|_| {
        CliError::Internal(format!(
            "the parked child of {} ended before it could be registered, so it never ran",
            g.program
        ))
    })
}

fn build_handle(
    p: &LaunchPlan,
    pid: u32,
    program: &str,
    native: impl FnOnce() -> (Option<u32>, Option<String>),
    now: &str,
) -> Result<ProcessHandle, CliError> {
    // The creation identity first, before anything that takes time: a harness that exits at
    // once is a zombie within milliseconds, and a zombie has none to read. The native probe,
    // which polls for up to 100 ms, comes after it.
    let process_start = process_start_identity(pid).ok_or_else(|| {
        CliError::Internal(format!(
            "could not capture the process-instance identity for launched pid {pid}; refusing to hand numeric termination authority to the daemon"
        ))
    })?;
    let (native_pid, native_process_start) = native();
    Ok(ProcessHandle {
        pid,
        pgid: pgid_of(p, pid),
        native_pid,
        native_binary: p.kill.native_binary.clone(),
        native_process_start,
        // This must be taken from the child that was just spawned, not resolved again at
        // seal time.  It is the durable process-instance guard that makes a same-argv PID
        // recycle a refusal instead of a new root of termination authority.
        process_start: Some(process_start),
        strategy: p.kill.strategy,
        grace_seconds: p.kill.grace_seconds,
        launched_at: now.to_string(),
        // What the child was exec'd from, as `spawn` resolved it. A `PATH` reordered or a
        // shim rewritten since then does not change what this process is running.
        program: program.to_string(),
        argv: recorded_argv(p),
        // Names only, never values. One of those values is this run's bearer token, and a
        // process's environment is not portably readable after the fact anyway — what the
        // daemon can check is that a pinned key is still there.
        env_keys: p.env_set.keys().cloned().collect(),
        // Never filled in here: this function is handed a child and a launch plan rather
        // than a connector's `[home_seed]`. A capture launch seeds nothing and the daemon
        // refuses a capture registration that carries a seed record, so this stays `None`.
        // The V1 seeding that set it was removed by D48.
        home_seed: None,
    })
}

/// The group this child owns, or `None`.
///
/// Recorded only after a `setpgid` that actually happened, and that is asked of the kernel
/// rather than assumed from the mode: `getpgid(pid) == pid` is true exactly when
/// `process_group(0)` put this child at the head of a group of its own. A mode that never
/// asked for a group, a platform without one, or a pid that has already been reaped all
/// answer `None` — which is the value the `ProcessGroup` strategy degrades to the
/// descendant walk on.
fn pgid_of(p: &LaunchPlan, pid: u32) -> Option<u32> {
    if !leads_its_group(p) {
        return None;
    }
    heads_its_own_group(pid).then_some(pid)
}

/// Whether `pid` is the leader of its own process group — which is exactly what
/// `process_group(0)` makes true, and what nothing else does.
///
/// Split off `pgid_of` so the platform difference is one function with two bodies rather
/// than a `cfg` block in the middle of a value.
#[cfg(unix)]
fn heads_its_own_group(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: `getpgid` takes an integer pid, dereferences no memory this process owns and
    // writes none. It answers -1 for a pid it cannot see, which is never equal to `raw`.
    unsafe { libc::getpgid(raw) == raw }
}

/// The native process the wrapper hides (D22), best-effort.
///
/// `None` is a legitimate answer at launch and not a failure: the npm wrapper spawns the
/// native binary a moment later, so the tree walked here may genuinely not contain it yet.
/// The daemon re-resolves at seal from a fresh snapshot, which is the whole reason this is
/// recorded as a hint rather than required as a precondition — and why a `ps` that could not
/// be run is `None` too, rather than a refusal that would abandon a live child.
fn native_identity_of(p: &LaunchPlan, pid: u32) -> (Option<u32>, Option<String>) {
    let Some(binary) = p.kill.native_binary.as_deref() else {
        return (None, None);
    };
    // `Command::spawn` returns after the wrapper execs, not necessarily after it forks the
    // native binary. Give that launch-time authority a small bounded opportunity to appear;
    // a missing hint remains valid and the daemon still re-resolves it at seal.
    for attempt in 0..=10 {
        let Ok(table) = snapshot() else {
            return (None, None);
        };
        if let Some(native_pid) = find_named(pid, binary, &table)
            && let Some(native_process_start) = process_start_identity(native_pid)
        {
            return (Some(native_pid), Some(native_process_start));
        }
        if attempt < 10 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    (None, None)
}

// ---------------------------------------------------------------------------------------
// The capture spawn gate (D40, CONTRACTS.md C6 capture spawn gate)
// ---------------------------------------------------------------------------------------
//
// A capture child must never run before the daemon has durably recorded it, and must never
// outlive a launcher that died before that. The child is therefore forked and **parked on a
// socketpair before `exec`**, and `exec`s the harness only when the launcher says so, after the
// daemon's `204` for its registration.
//
// The protocol, one byte each way after the pid:
//
// 1. the child closes the parent's end, writes its own pid (4 bytes) and waits;
// 2. the launcher reads the pid, drops its copy of the child's end (so the parent's end sees EOF
//    exactly when the child is dead), builds the handle and registers it;
// 3. on `204` the launcher writes `0x01`; the child answers `0x02` and returns from `pre_exec`,
//    so std `exec`s the harness; any other outcome, EOF included, makes the closure fail with
//    `ECANCELED`, std reports the errno and `_exit`s, and the harness never runs.
//
// A launcher that dies leaves the child reading EOF (or, past that, writing into a dead
// socket and taking SIGPIPE): either way it ends without `exec`. The pid and `process_start`
// registered while parked are fork-time identities that `exec` does not change.

#[cfg(unix)]
const GATE_RELEASE: u8 = 0x01;
#[cfg(unix)]
const GATE_ACK: u8 = 0x02;

/// A capture child forked and held before `exec` until its registration is durable (D40).
#[cfg(unix)]
pub struct GatedSpawn {
    pub pid: u32,
    /// What `exec` will run, resolved once by [`resolved_program`].
    pub program: String,
    parent_end: std::os::unix::net::UnixStream,
    /// `Some` only until the pid arrives; the launcher must not keep the child's end open, or
    /// the parent's end could never see the child die.
    child_end: Option<std::os::unix::net::UnixStream>,
    spawning: std::thread::JoinHandle<std::io::Result<Child>>,
    ready_within: std::time::Duration,
}

/// What a socket descriptor must clear: the standard streams the child's own stdio set-up
/// would otherwise overwrite if this process started with one of them closed.
#[cfg(unix)]
fn above_stdio(
    end: std::os::unix::net::UnixStream,
) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::AsFd;
    use std::os::unix::io::AsRawFd;
    if end.as_raw_fd() > 2 {
        return Ok(end);
    }
    // `try_clone_to_owned` duplicates at or above descriptor 3, close-on-exec.
    Ok(end.as_fd().try_clone_to_owned()?.into())
}

/// The only way a capture child is spawned (D40). It returns once the child is parked and its
/// pid is known; the harness has not run. A `secret` is the provider credential the child alone
/// receives, set on the command before the fork and nowhere else (Hermes, D40).
///
/// `ready_within` bounds the wait for the pid and, later, for the child's acknowledgement.
#[cfg(unix)]
pub fn spawn_gated(
    p: &LaunchPlan,
    pipes: Pipes,
    ready_within: std::time::Duration,
    secret: Option<&ChildSecret>,
) -> Result<GatedSpawn, CliError> {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;

    let (mut command, program) = build_command(p, pipes, secret);
    let io = |source: std::io::Error| CliError::Io {
        path: "the capture spawn gate".to_string(),
        source,
    };
    let (parent_end, child_end) = UnixStream::pair()
        .and_then(|(a, b)| Ok((above_stdio(a)?, above_stdio(b)?)))
        .map_err(io)?;
    let (parent_fd, child_fd) = (parent_end.as_raw_fd(), child_end.as_raw_fd());
    // SAFETY: the closure runs in the forked child before `exec` and calls only
    // async-signal-safe functions (`close`, `getpid`, `read`, `write`); it allocates nothing and
    // owns nothing (two descriptor numbers, copied).
    unsafe {
        command.pre_exec(move || park_until_released(parent_fd, child_fd));
    }
    // `Command::spawn` returns only after `exec` (or failure), which is after the release.
    let spawning = std::thread::Builder::new()
        .name("capture-gate-spawn".into())
        .spawn(move || command.spawn())
        .map_err(io)?;
    let mut gated = GatedSpawn {
        pid: 0,
        program: program.clone(),
        parent_end,
        child_end: Some(child_end),
        spawning,
        ready_within,
    };
    match gated.read_pid() {
        PidRead::Pid(pid) => {
            gated.pid = pid;
            // The child holds its own copy by now; this one must not outlive the pid read.
            gated.child_end = None;
            Ok(gated)
        }
        PidRead::SpawnEnded => {
            let GatedSpawn {
                parent_end,
                child_end,
                spawning,
                ..
            } = gated;
            drop((parent_end, child_end));
            match spawning.join() {
                Ok(Err(source)) => Err(CliError::Io {
                    path: program,
                    source,
                }),
                Ok(Ok(mut child)) => {
                    // The child ended before it wrote its pid and std read that as an exec.
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(CliError::Internal(format!(
                        "the harness {program} ended before it reached its pre-start gate; it did not run"
                    )))
                }
                Err(_) => Err(CliError::Internal(
                    "the capture spawn thread panicked; no harness ran".to_string(),
                )),
            }
        }
        PidRead::Failed(why) => {
            gated.abort();
            Err(CliError::Internal(format!(
                "the harness {program} did not reach its pre-start gate ({why}); it did not run"
            )))
        }
    }
}

#[cfg(unix)]
enum PidRead {
    Pid(u32),
    /// The spawn thread finished with no pid: the fork or the child's set-up failed.
    SpawnEnded,
    Failed(String),
}

#[cfg(unix)]
impl GatedSpawn {
    /// The child's pid, read in 20 ms slices until `ready_within`. A death before the pid is
    /// seen through the spawn thread ending (the launcher still holds the child's end then, so
    /// there is no EOF to wait for).
    fn read_pid(&mut self) -> PidRead {
        use std::io::Read;
        let until = std::time::Instant::now() + self.ready_within;
        let mut bytes = [0u8; 4];
        let mut got = 0;
        if let Err(e) = self
            .parent_end
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        {
            return PidRead::Failed(e.to_string());
        }
        loop {
            match (&self.parent_end).read(&mut bytes[got..]) {
                Ok(0) => return PidRead::Failed("the gate closed before the pid".to_string()),
                Ok(n) => {
                    got += n;
                    if got == bytes.len() {
                        return PidRead::Pid(u32::from_ne_bytes(bytes));
                    }
                    continue;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return PidRead::Failed(e.to_string()),
            }
            if self.spawning.is_finished() {
                return PidRead::SpawnEnded;
            }
            if std::time::Instant::now() >= until {
                return PidRead::Failed(format!("no pid within {}s", self.ready_within.as_secs()));
            }
        }
    }

    /// Release the parked child once its registration is durable. `Ok` only after the child's
    /// acknowledgement **and** std's successful `exec`; every `Err` means no harness ran (or, if the
    /// acknowledgement arrives after `ready_within` and the child `exec`s before the gate closes,
    /// that one that ran for milliseconds was killed here: CONTRACTS.md, gate amendment ¶5(f)), so the
    /// caller records no `Launch`.
    ///
    /// Stated residual: a signal that kills the child after it writes the acknowledgement and
    /// before `exec` makes std read EOF and return `Ok(Child)` for a dead child. The window is
    /// shorter than a microsecond and needs a signal.
    pub fn release(self) -> Result<Spawned, CliError> {
        use std::io::{Read, Write};
        let GatedSpawn {
            pid,
            program,
            mut parent_end,
            child_end,
            spawning,
            ready_within,
        } = self;
        drop(child_end);
        let gate = |what: String| {
            CliError::Internal(format!(
                "the harness {program} could not be released from its pre-start gate ({what}); no harness ran"
            ))
        };
        if let Err(e) = parent_end.write_all(&[GATE_RELEASE]) {
            // EPIPE: the child is dead.
            reap_failed_gate(parent_end, spawning);
            return Err(gate(e.to_string()));
        }
        let mut ack = [0u8; 1];
        let _ = parent_end.set_read_timeout(Some(ready_within));
        match parent_end.read_exact(&mut ack) {
            Ok(()) if ack[0] == GATE_ACK => {}
            other => {
                reap_failed_gate(parent_end, spawning);
                let what = match other {
                    Ok(()) => "an unexpected answer".to_string(),
                    Err(e) => e.to_string(),
                };
                return Err(gate(what));
            }
        }
        drop(parent_end);
        match spawning.join() {
            Ok(Ok(child)) if child.id() == pid => Ok(Spawned { child, program }),
            Ok(Ok(mut child)) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(CliError::Internal(format!(
                    "the harness {program} started as pid {} and not the pid {pid} registered for it",
                    child.id()
                )))
            }
            // `exec` failed after the acknowledgement: no harness ran.
            Ok(Err(source)) => Err(CliError::Io {
                path: program,
                source,
            }),
            Err(_) => Err(CliError::Internal(
                "the capture spawn thread panicked; the harness's state is unknown".to_string(),
            )),
        }
    }

    /// Never release: the child reads EOF and ends without `exec`. A child that nonetheless
    /// exec'd, or died, is ended and reaped here (it is this process's own).
    pub fn abort(self) {
        let GatedSpawn {
            parent_end,
            child_end,
            spawning,
            ..
        } = self;
        drop(child_end);
        reap_failed_gate(parent_end, spawning);
    }
}

/// Close the gate, wait for the spawn thread and end whatever child it returns: after a failed
/// release the child may be dead (reaped here) or may have received the release byte just
/// before the failure was read.
#[cfg(unix)]
fn reap_failed_gate(
    parent_end: std::os::unix::net::UnixStream,
    spawning: std::thread::JoinHandle<std::io::Result<Child>>,
) {
    drop(parent_end);
    if let Ok(Ok(mut child)) = spawning.join() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The child's side of the gate, between `fork` and `exec`. Async-signal-safe throughout.
#[cfg(unix)]
fn park_until_released(parent_fd: i32, child_fd: i32) -> std::io::Result<()> {
    let cancelled = || std::io::Error::from_raw_os_error(libc::ECANCELED);
    // SAFETY: `close`, `getpid`, `read` and `write` take integers and buffers owned by this
    // frame, and are async-signal-safe.
    unsafe {
        // The parent's end, or this process's own copy would mask the parent's death.
        libc::close(parent_fd);
        let pid = (libc::getpid() as u32).to_ne_bytes();
        if !gate_write(child_fd, &pid) {
            return Err(cancelled());
        }
        let mut byte = [0u8; 1];
        if !gate_read(child_fd, &mut byte) || byte[0] != GATE_RELEASE {
            return Err(cancelled());
        }
        if !gate_write(child_fd, &[GATE_ACK]) {
            return Err(cancelled());
        }
    }
    Ok(())
}

#[cfg(unix)]
unsafe fn gate_write(fd: i32, bytes: &[u8]) -> bool {
    let mut done = 0;
    while done < bytes.len() {
        // SAFETY: the pointer and length name the live tail of `bytes`.
        let n = unsafe { libc::write(fd, bytes[done..].as_ptr().cast(), bytes.len() - done) };
        if n > 0 {
            done += n as usize;
        } else if !(n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)) {
            return false;
        }
    }
    true
}

#[cfg(unix)]
unsafe fn gate_read(fd: i32, buffer: &mut [u8]) -> bool {
    loop {
        // SAFETY: the pointer and length name `buffer`.
        let n = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if n == buffer.len() as isize {
            return true;
        }
        if !(n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)) {
            return false;
        }
    }
}
