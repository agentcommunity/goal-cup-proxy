//! The launch recipe, rendered: the exact argv, the exact environment delta, the exact
//! working directory — and the handle the daemon finishes the job with (D22).
//!
//! # Building a plan is pure
//!
//! [`build_capture_launch`] takes a connector, a variable map, a directory and the
//! environment the child will start from, and returns a [`LaunchPlan`]. It spawns nothing and
//! reads no environment variable of its own. That is what lets every pin in this file be tested
//! on a machine with no harness installed. The V1 builder `plan` was pure the same way, which is
//! what let the V1 `goalcup start --print-launch` print a line without running it; D48 removed
//! both.
//!
//! # The environment is built from nothing (D1, D23, D38)
//!
//! [`build_capture_launch`] is the one builder left, and its child inherits nothing.
//! [`LaunchPlan::env_base`] is empty and [`LaunchPlan::env_remove`] is empty; [`spawn`] clears
//! the child's environment, so what reaches the harness is exactly [`LaunchPlan::env_set`]:
//!
//! * the profile's own `capture.env`, with the running platform's `capture.platform` entries
//!   over it, rendered;
//! * the allowlist [`CAPTURE_IDENTITY_ENV`] (`USER`, `LOGNAME`, `TMPDIR`), carried across
//!   from the tracker's environment and nothing else from it;
//! * the run's `HOME` (per run, or the connector's shared run home, D59), `PATH` (the
//!   participant's own, or [`CAPTURE_PATH`] when they have none: [`harness_path`]) and the
//!   profile's home variable;
//! * the participant's terminal environment ([`CHAT_PASSTHROUGH_ENV`] and `LC_*`), under the
//!   connector's own values;
//! * the passive **loopback** recording proxy through process-scoped `HTTPS_PROXY`/
//!   `https_proxy` — the M2 reservation AGENTS.md names, and the only proxy a launch may name,
//!   checked by `loopback_proxy_url` — with `NO_PROXY`/`no_proxy` ([`CAPTURE_NO_PROXY`]) and
//!   the profile's process-scoped CA variable;
//! * Goal Cup Proxy's own two, the same for every harness: [`IN_HARNESS_ENV`], under which
//!   `goalcup seal` refuses, and the run's time-left file the agent may read
//!   ([`crate::run::time_left::FILE_ENV`], D66). A connector cannot name either (`GOALCUP_*`
//!   is refused in `capture.env`).
//!
//! **D1: Goal Cup Proxy never sets a model base URL.** Nothing on that list can be one except
//! the profile's `capture.env`, and `goalcup_protocol::capture_policy` refuses a profile that
//! names a `*_BASE_URL`, `*_API_URL` or `*ENDPOINT` variable, a `*_PROXY` or bare `PROXY`
//! (other than `NODE_USE_ENV_PROXY = "1"`, which names no endpoint, D59), or any other `*_URL`
//! there, before a plan is ever built. The participant's shell cannot add
//! one either, because nothing outside the allowlist is inherited (the terminal pass-through is
//! an allowlist too): an allowlist leaves no `*_URL` the participant exported to strip.
//!
//! # A launch keeps the terminal (D57)
//!
//! Every connector runs its harness in its normal chat mode in the participant's own terminal
//! (`[capture] interactive = true`; headless mode was removed): [`build_capture_launch`] places
//! no goal, and the plan is [`LaunchMode::Tui`]. Its child inherits this process's stdin, stdout and
//! stderr (no `/dev/null`, no pipe: the participant types the prompt and reads the answer), and
//! it stays in **this process's own process group**, which is the terminal's foreground group
//! when `goalcup start` is run from a shell.
//!
//! The other way, a group of its own made the foreground group with `tcsetpgrp` and the
//! foreground handed back after it exits, was considered and not taken, for what the
//! termination strategies need:
//!
//! * `process_group` (Claude Code) signals a `pgid` only when the handle records one, and a
//!   `Tui` handle records none ([`pgid_of`]), so the daemon ends the harness by the descendant
//!   walk from its pid. The host's group is never signalled, and the host is never in the tree
//!   below the harness, so the harness's termination can never reach the host.
//! * `native_pid` (Codex) signals the native binary the wrapper started and its descendants;
//!   which group they are in does not matter to it.
//! * A group of its own would need `tcsetpgrp` before the harness's first read (a race with the
//!   exec, or a second gate step) and again after it exits, from a host that by then may be in
//!   the background of a terminal that is gone (`SIGTTOU`). Sharing the group needs neither.
//!
//! What sharing costs is that every signal the terminal sends the foreground group (Ctrl-C,
//! Ctrl-\, Ctrl-Z, a closed window) reaches the host as well; `goalcup start` absorbs those
//! while the harness runs (`goalcup_platform::terminal::Shield`) and says nothing on the
//! terminal meanwhile (`crate::host_log`).
//!
//! # The three modes differ in exactly one place
//!
//! [`LaunchMode::Tui`] must **never** call `process_group(0)`. A TUI in a process group of
//! its own is not the terminal's foreground group, so its first read from the tty raises
//! SIGTTIN and stops it — the harness appears to hang before it has drawn a frame. A TUI
//! handle therefore carries `pgid: None`, and `KillStrategy::ProcessGroup` degrades
//! to the descendant walk it already implements.
//!
//! [`LaunchMode::Server`] is a harness the participant talks to over a socket. It has no tty to
//! lose, so it may own its group and its handle carries a real `pgid` — which is the strictly
//! better thing to hand a terminator. No capture plan uses this mode; the variant stays in the
//! process-handle schema.
//!
//! [`LaunchMode::AttachOnly`] builds no plan at all. In V1, attach mode armed the folder and
//! the participant opened a session in it; there was no argv, so the V1 `plan` refused it
//! rather than inventing one. Every capture plan (D57) is [`LaunchMode::Tui`].
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
use goalcup_protocol::harness::{CaptureSpec, PlatformKey, Strategy};
use goalcup_protocol::manifest::{Kill, LaunchMode};
use goalcup_protocol::process::{ProcessHandle, find_named, process_start_identity, snapshot};
use goalcup_protocol::template::{TemplateVars, render};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

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
}

impl fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchPlan")
            .field("program", &self.program)
            .field("argv", &self.argv)
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
            .finish()
    }
}

/// What a secret value prints as, on every surface in this crate that prints one.
const REDACTED: &str = "<redacted>";

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

// ------------------------------------------------------------------ M2 capture launch

/// The `PATH` of a capture launch: the system directories and nothing a participant's shell
/// put in front of them (D38).
pub const CAPTURE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// `NO_PROXY`/`no_proxy` of a capture launch: the loopback proxy itself is never proxied.
pub const CAPTURE_NO_PROXY: &str = "127.0.0.1,localhost";

/// The only names a capture launch carries across from the tracker's own environment
/// (the allowlist; `HOME`, `PATH` and `SHELL` are set, not carried).
pub(crate) const CAPTURE_IDENTITY_ENV: [&str; 3] = ["USER", "LOGNAME", "TMPDIR"];

/// What a chat-mode launch (D57) also carries across from the participant's environment, beside
/// [`CAPTURE_IDENTITY_ENV`]: the harness runs in their terminal with their tools, so it gets
/// their `PATH` (an npm harness is a `#!/usr/bin/env node` script, and the agent runs Homebrew,
/// cargo and the rest), their terminal type and colours, their language, and their shell. Every
/// `LC_*` variable is carried too. Nothing else: no proxy, no base URL, no credential, none of
/// goalcup's own secrets. A connector's `[capture.env]` still wins (its `SHELL`, for one), and
/// the launch's own pins (`HOME`, the home variable, the loopback proxy, the CA) win over both.
pub const CHAT_PASSTHROUGH_ENV: [&str; 5] = ["PATH", "TERM", "COLORTERM", "LANG", "SHELL"];

/// The `PATH` the harness is given: the participant's own (D57), when they have one, and
/// [`CAPTURE_PATH`] otherwise.
pub fn harness_path(parent_env: &BTreeMap<String, String>) -> String {
    match parent_env.get("PATH") {
        Some(path) if !path.is_empty() => path.clone(),
        _ => CAPTURE_PATH.to_string(),
    }
}

/// Set to `1` in every recording launch's environment, whatever the harness: `goalcup seal` run
/// from inside the harness sees it and refuses, so the agent cannot end its own segment.
pub const IN_HARNESS_ENV: &str = "GOALCUP_IN_HARNESS";

/// Whether this process runs inside a recorded harness: [`IN_HARNESS_ENV`] is set and not
/// empty. An empty value is the same as unset.
pub fn in_harness() -> bool {
    std::env::var_os(IN_HARNESS_ENV).is_some_and(|value| !value.is_empty())
}

/// The `ssh` the agent's git runs ([`agent_base_env`]): no config file (`~/.ssh/config` names
/// keys), no key offered, no agent key, and no question.
pub const AGENT_GIT_SSH_COMMAND: &str =
    "ssh -F /dev/null -o IdentitiesOnly=yes -o IdentityFile=none -o BatchMode=yes";

/// What every program the agent runs sees of the participant, beside the harness's own
/// variables: [`CAPTURE_IDENTITY_ENV`], their `PATH` ([`harness_path`]) and `HOME` set to `home`.
///
/// The agent pushes with the token the participant puts in their project folder's `.env.local`,
/// for the one repository of the run, and with no other login of theirs (OD9). Nothing of their
/// git login is handed on (no `SSH_AUTH_SOCK`, `GIT_CONFIG_GLOBAL` or `GH_CONFIG_DIR`), and two
/// ways it would still reach the agent are closed: `GIT_CONFIG_NOSYSTEM=1`, because the system
/// git config (Homebrew's and Apple's set `credential.helper=osxkeychain`) would hand git the
/// participant's keychain login to any repository; and [`AGENT_GIT_SSH_COMMAND`] as
/// `GIT_SSH_COMMAND`, because OpenSSH finds `~/.ssh` from the user database, not from `HOME`, and
/// would offer a key without a passphrase. A connector cannot name either (`GIT_` is refused in
/// `capture.env`), and they are set after the connector's own values.
pub fn agent_base_env(
    parent_env: &BTreeMap<String, String>,
    home: &str,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in CAPTURE_IDENTITY_ENV {
        if let Some(value) = parent_env.get(key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    env.insert("HOME".into(), home.to_string());
    env.insert("PATH".into(), harness_path(parent_env));
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env.insert("GIT_SSH_COMMAND".into(), AGENT_GIT_SSH_COMMAND.into());
    env
}

/// The participant's variables a launch carries across ([`CHAT_PASSTHROUGH_ENV`] and every
/// `LC_*` the schema would admit as a name).
fn chat_passthrough(parent_env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    parent_env
        .iter()
        .filter(|(key, value)| {
            !value.contains('\0')
                && (CHAT_PASSTHROUGH_ENV.contains(&key.as_str())
                    || (key.starts_with("LC_")
                        && goalcup_protocol::capture_policy::env_key_denied(
                            key,
                            goalcup_protocol::capture_policy::EnvContext::CaptureEnv,
                        )
                        .is_none()))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// A harness executable as a launch resolved it: the canonical path it runs.
///
/// The build is recorded, never pinned (D53): the daemon hashes the registered program and the
/// running image into `Launch.harness_build` itself, so the CLI keeps no digest and reads none
/// of the file's bytes. Only the shape is checked, because the launch execs this exact path:
/// absolute, not a symlink, a regular file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutable {
    path: String,
    /// The `PATH` the harness is given ([`harness_path`]), when the resolution knew it: what its
    /// own `#!/usr/bin/env <interpreter>` line and its version probe look things up on.
    search_path: Option<String>,
}

impl ResolvedExecutable {
    /// The absolute canonical path the capture launch runs.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The `PATH` the harness is given, or [`CAPTURE_PATH`] when the resolution did not say.
    pub fn search_path(&self) -> &str {
        self.search_path.as_deref().unwrap_or(CAPTURE_PATH)
    }
}

/// The connector's program as the participant's `PATH` (in `env`) finds it, followed to the file
/// an updater's link names, and checked to start on the `PATH` the harness is given
/// ([`harness_path`]): a script whose first line is `#!/usr/bin/env <interpreter>` (the npm
/// harnesses: Codex CLI, Claude Code) needs that interpreter on that `PATH`. The file is the one
/// the launch runs. It is checked, never started here: `goalcup start`, `goalcup doctor` and
/// `goalcup prep` all ask this one question, so a harness doctor passes is one start launches.
///
/// Only the first line of the file is read, for its interpreter; its build is not hashed here
/// (D53).
pub fn capture_executable(
    c: &HarnessConnector,
    profile: &CaptureSpec,
    env: &BTreeMap<String, String>,
) -> Result<ResolvedExecutable, CliError> {
    let program = &profile.executable.program;
    let Some(found) = crate::preflight::resolve_on_path(program, env) else {
        return Err(CliError::Refused(format!(
            "`{program}` is not on PATH, so `{}`'s capture executable cannot be found. Nothing \
             is launched.",
            c.loaded.manifest.id
        )));
    };
    let file = std::fs::canonicalize(&found).map_err(|source| CliError::Io {
        path: found.display().to_string(),
        source,
    })?;
    let mut resolved = resolve_harness_executable(&file)?;
    let search_path = harness_path(env);
    if let Some(interpreter) = env_interpreter(&file)
        && crate::preflight::resolve_on_path(
            &interpreter,
            &BTreeMap::from([("PATH".to_string(), search_path.clone())]),
        )
        .is_none()
    {
        return Err(CliError::Refused(format!(
            "`{program}` ({}) is a `{interpreter}` script, and `{interpreter}` is not on the PATH \
             the harness is given ({search_path}). Nothing is launched.",
            file.display()
        )));
    }
    resolved.search_path = Some(search_path);
    Ok(resolved)
}

/// The interpreter a `#!/usr/bin/env <name>` first line names (options such as `-S` skipped),
/// or `None` for anything else: a binary, a script with an absolute interpreter, a file that
/// cannot be read. At most 256 bytes are read.
///
/// The check fails open by design: a first line with no newline within 256 bytes, or a file
/// that cannot be opened or read here, names no interpreter, and the launch itself says what is
/// wrong with such a file.
fn env_interpreter(file: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = open_no_follow(file).ok()?.read(&mut head).ok()?;
    let line = head[..n].strip_prefix(b"#!")?;
    let line = &line[..line.iter().position(|b| *b == b'\n')?];
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_whitespace();
    if !words.next()?.ends_with("/env") {
        return None;
    }
    words
        .find(|word| !word.starts_with('-') && !word.contains('='))
        .filter(|name| !name.contains('/'))
        .map(str::to_string)
}

/// Check the regular file at `path` and keep its canonical path.
///
/// `path` is resolved by the caller: the measured recipes run the native binary (not the
/// Codex npm wrapper) and the versioned Claude Code binary (not the symlink its updater
/// moves), whose location depends on how the harness was installed. The kind of file is
/// checked on the open handle; no byte of it is read.
///
/// **A symlink is refused, not followed** (exec the versioned path directly).
/// `~/.local/bin/claude` is retargeted by the next updater, so a link would run whatever it
/// names at spawn time rather than the file that was named. Directory links above the file
/// (`/var` -> `/private/var`) are resolved, and the resolved path is the canonical one; the file
/// itself is then opened without following a link, so a name swapped for a link after the
/// check is refused by the open.
///
/// Refusals: a relative or non-UTF-8 path, a symlink, or anything but a regular file:
/// [`CliError::Refused`], naming the path; a file this process cannot open: [`CliError::Io`].
pub fn resolve_harness_executable(path: &Path) -> Result<ResolvedExecutable, CliError> {
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
    require_regular_file_no_follow(&canonical, given)?;
    Ok(ResolvedExecutable {
        path: text.to_string(),
        search_path: None,
    })
}

/// Open the exact path read-only, nonblocking and without following a symlink at its final
/// component, so a substituted FIFO cannot hang the check before `metadata` rejects it, and
/// refuse anything that is not a regular file. Nothing is read.
fn require_regular_file_no_follow(path: &Path, display: &str) -> Result<(), CliError> {
    let io = |source| CliError::Io {
        path: display.to_string(),
        source,
    };
    let file = open_no_follow(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(CliError::Refused(format!(
            "the capture executable `{display}` is not a regular file. Nothing is launched."
        )));
    }
    Ok(())
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

/// What a capture launch needs that its connector's profile cannot say.
///
/// `Debug` prints the proxy URL without its userinfo.
pub struct CaptureLaunch<'a> {
    pub executable: &'a ResolvedExecutable,
    /// The per-run `HOME`, created by the caller in the shape the profile's `home` names.
    pub run_home: &'a Path,
    /// The isolated harness home `launch.home_env` points at (the profile's `config_home`).
    pub config_home: &'a Path,
    /// `http://goalcup:<credential>@<127.0.0.1|[::1]|localhost>:<port>`, the recording proxy this
    /// launch records through ([`crate::proxy::connect::ProxyCredential::proxy_url`]'s shape).
    pub proxy_url: &'a str,
    /// The recording proxy's per-launch CA certificate, handed to each of the profile's own
    /// `ca.env` variables, in either mode.
    pub ca_certificate: &'a Path,
    /// The system's roots plus the recording proxy's CA ([`crate::ca_bundle::bundle_for`]), handed
    /// to each [`SHARED_CA_ENV`](goalcup_protocol::capture_policy::SHARED_CA_ENV) row the profile
    /// names. `None` when it names none; a launch that needs it and has none is refused.
    pub ca_bundle: Option<&'a Path>,
    /// The tracker's own environment. Only [`CAPTURE_IDENTITY_ENV`], [`CHAT_PASSTHROUGH_ENV`]
    /// and `LC_*` are read from it.
    pub parent_env: &'a BTreeMap<String, String>,
    /// The run's time-left file ([`crate::run::time_left::file_path`]), handed to the harness
    /// as [`crate::run::time_left::FILE_ENV`] (D66). `goalcup start` always gives one.
    pub time_left_file: Option<&'a Path>,
}

impl fmt::Debug for CaptureLaunch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureLaunch")
            .field("executable", &self.executable)
            .field("run_home", &self.run_home)
            .field("config_home", &self.config_home)
            .field(
                "proxy_url",
                &without_userinfo(self.proxy_url).unwrap_or_else(|| self.proxy_url.to_string()),
            )
            .field("ca_certificate", &self.ca_certificate)
            .field("ca_bundle", &self.ca_bundle)
            .field("parent_env", &Keys(self.parent_env))
            .field("time_left_file", &self.time_left_file)
            .finish()
    }
}

/// The capture launch rendered from harness-contract schema 1.
pub fn build_capture_launch(
    c: &HarnessConnector,
    v: &TemplateVars,
    workspace: &Path,
    launch: &CaptureLaunch<'_>,
) -> Result<LaunchPlan, CliError> {
    build_launch(c, v, workspace, &Inputs::recording(launch), false)
}

/// [`build_capture_launch`] for a run that is being continued (`goalcup continue`): the
/// profile's `resume_args` follow its `args`, so the harness picks up its last session.
pub fn build_resumed_launch(
    c: &HarnessConnector,
    v: &TemplateVars,
    workspace: &Path,
    launch: &CaptureLaunch<'_>,
) -> Result<LaunchPlan, CliError> {
    build_launch(c, v, workspace, &Inputs::recording(launch), true)
}

/// Whether a continued run has a session its harness can pick up: a file of one of the
/// connector's `[[capture.native]]` stores (the harness's own record of the conversation),
/// written at or after `since_unix`, the run's start, when the store reads from the run's start
/// (D57), else any file it holds. A harness that closed before the first message (a setup that
/// ends the program, a trust question answered "no") wrote none, so `goalcup continue` opens it
/// the normal way instead of asking it to resume a session that is not there.
///
/// `Ok(false)` only when every store's folder is missing or holds no matching file. A connector
/// with no native store, or a store whose root this cannot place, cannot tell: `Ok(true)`, and
/// it is resumed as before. A store that cannot be read (`Err`, with why) cannot tell either;
/// the caller resumes and logs the reason.
pub fn has_native_session(
    profile: &CaptureSpec,
    run_home: &Path,
    config_home: &Path,
    since_unix: u64,
) -> Result<bool, String> {
    if profile.native.is_empty() {
        return Ok(true);
    }
    let since = std::time::UNIX_EPOCH + std::time::Duration::from_secs(since_unix);
    let mut unreadable = None;
    for native in &profile.native {
        let root = if let Some(rest) = native.root.strip_prefix("{config_home}") {
            config_home.join(rest.trim_start_matches('/'))
        } else if let Some(rest) = native.root.strip_prefix("{run_home}") {
            run_home.join(rest.trim_start_matches('/'))
        } else {
            return Ok(true);
        };
        let parts: Vec<&str> = native.pattern.split('/').collect();
        let floor = native.since_run_start.then_some(since);
        match holds_session_file(&root, &parts, floor) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(why) => unreadable = Some(why),
        }
    }
    unreadable.map_or(Ok(false), Err)
}

/// A file under `dir` whose path matches `parts` one component each, never through a link,
/// written at or after `floor` when there is one. A missing folder, or a matching name that is
/// gone before it is looked at, holds none; any other error is returned, naming the path.
fn holds_session_file(
    dir: &Path,
    parts: &[&str],
    floor: Option<std::time::SystemTime>,
) -> Result<bool, String> {
    let Some((first, rest)) = parts.split_first() else {
        return Ok(false);
    };
    let unreadable = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(unreadable(dir, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| unreadable(dir, e))?;
        if !goalcup_protocol::harness::glob_matches(&entry.file_name().to_string_lossy(), first) {
            continue;
        }
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(unreadable(&path, e)),
        };
        let found = if rest.is_empty() {
            meta.is_file()
                && match floor {
                    None => true,
                    Some(floor) => meta.modified().map_err(|e| unreadable(&path, e))? >= floor,
                }
        } else {
            meta.is_dir() && holds_session_file(&path, rest, floor)?
        };
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What [`build_launch`] needs from the caller: a [`CaptureLaunch`], or the same without the
/// recording proxy (`goalcup prep`'s launch, [`build_prep_launch`]).
struct Inputs<'a> {
    executable: &'a ResolvedExecutable,
    run_home: &'a Path,
    config_home: &'a Path,
    /// The loopback proxy URL, the recording proxy's CA certificate and the roots-plus-proxy bundle
    /// handed to the harness; `None` for a launch that records nothing and so has no proxy to
    /// name.
    proxy: Option<(&'a str, &'a Path, Option<&'a Path>)>,
    parent_env: &'a BTreeMap<String, String>,
    time_left_file: Option<&'a Path>,
}

impl<'a> Inputs<'a> {
    fn recording(launch: &'a CaptureLaunch<'a>) -> Inputs<'a> {
        Inputs {
            executable: launch.executable,
            run_home: launch.run_home,
            config_home: launch.config_home,
            proxy: Some((launch.proxy_url, launch.ca_certificate, launch.ca_bundle)),
            parent_env: launch.parent_env,
            time_left_file: launch.time_left_file,
        }
    }
}

fn build_launch(
    c: &HarnessConnector,
    v: &TemplateVars,
    workspace: &Path,
    launch: &Inputs<'_>,
    resume: bool,
) -> Result<LaunchPlan, CliError> {
    let id = &c.loaded.manifest.id;
    let profile = &c.loaded.manifest.capture;
    let platform = PlatformKey::current();
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
    let proxy = match launch.proxy {
        Some((url, ca, bundle)) => {
            let ca_certificate = absolute_utf8("proxy CA certificate", ca)?;
            let ca_bundle = bundle
                .map(|b| absolute_utf8("system roots and proxy CA bundle", b))
                .transpose()?;
            loopback_proxy_url(url)?;
            Some((url, ca_certificate, ca_bundle))
        }
        None => None,
    };

    let mut argv = Vec::with_capacity(profile.args.len() + profile.resume_args.len());
    for template in &profile.args {
        let arg = render(template, v)?;
        if let Some(why) = harness_arg_refusal(profile, &arg) {
            return Err(CliError::Refused(format!(
                "`{arg}` {why}. Nothing is launched."
            )));
        }
        argv.push(arg);
    }
    // A continued launch adds the profile's resume arguments after its own, under the same
    // rules.
    if resume {
        for template in &profile.resume_args {
            let arg = render(template, v)?;
            if let Some(why) = harness_arg_refusal(profile, &arg) {
                return Err(CliError::Refused(format!(
                    "`{arg}` {why}. Nothing is launched."
                )));
            }
            argv.push(arg);
        }
    }
    // The harness gets the participant's terminal environment first; the connector's own
    // values and the launch's pins below win over it.
    let mut env = chat_passthrough(launch.parent_env);
    // An env value may also name the run's two homes (`{config_home}`, `{run_home}`); an
    // argument may not.
    let mut env_vars = v.clone();
    env_vars.set("config_home", config_home.to_string());
    env_vars.set("run_home", run_home.to_string());
    for (key, template) in profile.resolved_env(platform) {
        env.insert(key, render(&template, &env_vars)?);
    }
    env.extend(agent_base_env(launch.parent_env, run_home));
    // A harness without a home variable keeps its state under the private `HOME` above.
    if let Some(home_env) = &profile.home.env {
        env.insert(home_env.clone(), config_home.to_string());
    }
    if let Some((proxy_url, ca_certificate, ca_bundle)) = proxy {
        for key in ["HTTPS_PROXY", "https_proxy"] {
            env.insert(key.into(), proxy_url.to_string());
        }
        for key in ["NO_PROXY", "no_proxy"] {
            env.insert(key.into(), CAPTURE_NO_PROXY.into());
        }
        // A harness's own variable trusts the recording proxy alone (a replace-mode one so fails
        // closed, D41); a shared row, which tool children read too, gets the system's roots as
        // well.
        for key in &profile.ca.env {
            let file = match crate::ca_bundle::handed_file(key) {
                crate::ca_bundle::HandedFile::ProxyCa => ca_certificate,
                crate::ca_bundle::HandedFile::SystemRootsAndProxyCa => {
                    ca_bundle.ok_or_else(|| {
                        CliError::Internal(format!(
                            "`{id}` names `{key}`, which is handed the system roots plus the \
                             proxy CA, and the launch has no such file"
                        ))
                    })?
                }
            };
            env.insert(key.clone(), file.to_string());
        }
        // Goal Cup Proxy's own variables, the same for every harness: the marker `goalcup seal`
        // refuses under, and the time-left file the agent may read (D66). Nothing is pushed.
        env.insert(IN_HARNESS_ENV.into(), "1".into());
        if let Some(file) = launch.time_left_file {
            let file = absolute_utf8("time-left file", file)?;
            env.insert(crate::run::time_left::FILE_ENV.into(), file.to_string());
        }
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
        // The harness keeps the terminal: see the module header.
        mode: LaunchMode::Tui,
        kill: Kill {
            strategy,
            native_binary: termination.native_binary,
            grace_seconds,
        },
    };
    let needed = exec_footprint(&plan);
    let limit = exec_limit();
    if needed > limit {
        return Err(CliError::Refused(format!(
            "`{id}`'s capture launch needs {needed} bytes of argument and environment space \
             and this system allows {limit} (ARG_MAX). Nothing is launched."
        )));
    }
    Ok(plan)
}

/// What `goalcup prep` opens the harness with: the executable, and the two homes a run's harness
/// has (the private `HOME`, and the harness home every run of the connector shares).
pub struct PrepLaunch<'a> {
    pub executable: &'a ResolvedExecutable,
    pub run_home: &'a Path,
    pub config_home: &'a Path,
    /// The tracker's own environment; only [`CAPTURE_IDENTITY_ENV`] is read from it.
    pub parent_env: &'a BTreeMap<String, String>,
}

/// The launch `goalcup prep` makes: a chat-mode connector's harness exactly as a run starts it
/// (the same arguments, the same environment, the same two homes) but with **no recording at
/// all**: no proxy and no CA variable, no daemon entry, no run directory beyond the throwaway
/// one holding `run_home`. It is the participant's own session: they sign in, and add settings,
/// MCP servers and skills to the home that their runs will then use.
pub fn build_prep_launch(
    c: &HarnessConnector,
    v: &TemplateVars,
    workspace: &Path,
    launch: &PrepLaunch<'_>,
) -> Result<LaunchPlan, CliError> {
    build_launch(
        c,
        v,
        workspace,
        &Inputs {
            executable: launch.executable,
            run_home: launch.run_home,
            config_home: launch.config_home,
            proxy: None,
            parent_env: launch.parent_env,
            time_left_file: None,
        },
        false,
    )
}

/// Open the harness for `goalcup prep` and wait for it to exit. It keeps the participant's
/// terminal as a run's chat-mode harness does (stdin, stdout and stderr inherited, no group of
/// its own); this process absorbs what the terminal sends while it runs, so a Ctrl-C meant for
/// the harness cannot end the command that is waiting for it, and puts the terminal's modes
/// back afterwards. A harness that stops itself is continued.
pub fn prep_launch(
    c: &HarnessConnector,
    launch: &PrepLaunch<'_>,
    workspace: &Path,
) -> Result<std::process::ExitStatus, CliError> {
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    // Only a name some profile argument may carry; the harness makes nothing of it here.
    vars.set(
        "session_id",
        "00000000-0000-4000-8000-000000000000".to_string(),
    );
    let plan = build_prep_launch(c, &vars, workspace, launch)?;
    let modes = goalcup_platform::terminal::Modes::save(libc::STDIN_FILENO);
    let spawned = spawn(&plan)?;
    // Without the shield a Ctrl-C meant for the harness also ends this command: said, not hidden.
    let shield = match goalcup_platform::terminal::Shield::take() {
        Ok(shield) => Some(shield),
        Err(error) => {
            eprintln!(
                "goalcup: the terminal's signals could not be absorbed ({error}), so a Ctrl-C \
                 typed to the harness may end this command too."
            );
            None
        }
    };
    let waited = goalcup_platform::terminal::wait_resuming(spawned.child.id(), &mut |_| true);
    drop(shield);
    if let Some(modes) = modes {
        let _ = modes.restore();
    }
    waited.map_err(|source| CliError::Io {
        path: spawned.program.clone(),
        source,
    })
}

/// How long one `capture.configure` command may run before it is ended and the launch refused.
pub const CONFIGURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run the connector's `capture.configure` commands (D61), in order, each as the harness's own
/// program with exactly the environment [`build_prep_launch`] gives it (the run's `HOME`, the
/// harness home, no proxy and no CA), stdin `/dev/null`, its own process group, and stdout and
/// stderr appended to `log`. The first one that does not exit 0 within [`CONFIGURE_TIMEOUT`]
/// refuses with a sentence naming it and how it ended, then `then` (where its output is, and
/// what to do next).
/// `goalcup prep` runs them after the participant's own setup, `goalcup start` before each
/// launch; nothing they print reaches the terminal or the trace.
pub fn configure_harness(
    c: &HarnessConnector,
    launch: &PrepLaunch<'_>,
    workspace: &Path,
    log: &std::fs::File,
    then: &str,
) -> Result<(), CliError> {
    let profile = &c.loaded.manifest.capture;
    if profile.configure.is_empty() {
        return Ok(());
    }
    let mut vars = TemplateVars::new();
    vars.set("workspace", workspace.display().to_string());
    let plan = build_prep_launch(c, &vars, workspace, launch)?;
    let name = c.display_name();
    for command in &profile.configure {
        let shown = format!("{} {}", profile.executable.program, command.join(" "));
        let refuse = |why: String| {
            CliError::Refused(format!(
                "{name} could not be set up for recording: `{shown}` {why}. {then}"
            ))
        };
        // The one command builder: the plan's environment, built from nothing, and nothing
        // else. Then this command's own stdio and a group of its own, so a timeout ends all it
        // started.
        let mut one = plan.clone();
        one.argv = command.clone();
        let (mut cmd, _) = build_command(&one);
        cmd.stdin(std::process::Stdio::null());
        let handles = (log.try_clone(), log.try_clone());
        let (Ok(stdout), Ok(stderr)) = handles else {
            return Err(refuse("could not be given its log".to_string()));
        };
        cmd.stdout(stdout).stderr(stderr);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| refuse(format!("could not be started ({e})")))?;
        let Some(status) = crate::harness_version::wait_or_end_group(&mut child, CONFIGURE_TIMEOUT)
        else {
            return Err(refuse(format!(
                "did not finish within {} seconds",
                CONFIGURE_TIMEOUT.as_secs()
            )));
        };
        if !status.success() {
            return Err(refuse(format!("ended with {}", describe_status(&status))));
        }
    }
    Ok(())
}

/// `exit code N` or `signal N`, for a refusal.
fn describe_status(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "no exit code".to_string(),
    }
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
pub(crate) fn exec_footprint(p: &LaunchPlan) -> u64 {
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
pub(crate) fn exec_limit() -> u64 {
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
/// A capture launch is a [`LaunchMode::Tui`] and is not: it must stay in the terminal's
/// foreground group, and a group handle is never confirmed ended (`docs/TERMINATION.md`, D30),
/// so the daemon ends it by the descendant walk.
fn leads_its_group(p: &LaunchPlan) -> bool {
    owns_its_group(p.mode)
}

/// Whether this mode's launch may own a process group of its own.
///
/// The whole of the difference between the three modes. See the module header for why a
/// `Tui` may not.
fn owns_its_group(mode: LaunchMode) -> bool {
    match mode {
        LaunchMode::Tui => false,
        LaunchMode::Server => true,
        // Unreachable: a capture plan is `Server` or `Tui`, and the V1 `plan` refused this
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
    let (mut command, program) = build_command(p);
    let child = command.spawn().map_err(|source| CliError::Io {
        path: program.clone(),
        source,
    })?;
    Ok(Spawned { child, program })
}

/// Why one argument is denied by the schema-1 harness profile.
pub fn harness_arg_refusal(spec: &CaptureSpec, arg: &str) -> Option<&'static str> {
    spec.argv_guard.as_ref().and_then(|guard| guard.denies(arg))
}

/// The command a launch runs, and the program path it was resolved to: argv, working
/// directory, the built environment and the process group a server leads; stdin, stdout and
/// stderr are inherited. [`spawn`] and [`spawn_gated`] both run what this builds, so a gated
/// capture child's argv, environment and stdio are exactly an ungated one's.
fn build_command(p: &LaunchPlan) -> (Command, String) {
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
    // The launch inherits all three streams: the participant's terminal is its input. A test
    // build that runs it with no terminal ([`crate::chat_without_terminal`]) gives it none.
    if crate::chat_without_terminal() {
        command.stdin(std::process::Stdio::null());
    }
    // The one line the three modes differ in. A TUI never reaches it, and nor does a capture
    // launch (see `leads_its_group`).
    #[cfg(unix)]
    if leads_its_group(p) {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    (command, program)
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
    // is a child that already died (a zombie has none, on macOS and Linux alike): it never ran.
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
        argv: p.argv.clone(),
        // Names only, never values. One of those values is this run's bearer token, and a
        // process's environment is not portably readable after the fact anyway — what the
        // daemon can check is that a pinned key is still there.
        env_keys: p.env_set.keys().cloned().collect(),
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
/// pid is known; the harness has not run.
///
/// `ready_within` bounds the wait for the pid and, later, for the child's acknowledgement.
#[cfg(unix)]
pub fn spawn_gated(
    p: &LaunchPlan,
    ready_within: std::time::Duration,
) -> Result<GatedSpawn, CliError> {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;

    let (mut command, program) = build_command(p);
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
    /// that one that ran for milliseconds was killed here: CONTRACTS.md, C6 capture spawn gate ¶5(f)), so the
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

#[cfg(test)]
mod tests {
    use super::has_native_session;
    use goalcup_protocol::harness::HarnessManifest;
    use std::time::{Duration, UNIX_EPOCH};

    /// A session is a native-store file written since the run started, found without following
    /// a link. Only a missing folder or no matching file is "no session": a store that cannot be
    /// read, or a connector with no native store, cannot tell, and is said to have one.
    #[test]
    fn a_session_is_a_native_file_written_since_the_run_started() {
        use std::os::unix::fs::PermissionsExt;
        let manifest = HarnessManifest::from_toml_str(include_str!(
            "../tests/fixtures/connectors/chat-shared-home/harness.toml"
        ))
        .unwrap();
        let profile = &manifest.capture;
        let t = tempfile::tempdir().unwrap();
        let (run_home, config_home) = (t.path().join("run"), t.path().join("config"));
        let since = 2_000_000_000;
        let has = || has_native_session(profile, &run_home, &config_home, since);
        assert_eq!(has(), Ok(false), "no store folder yet");

        let project = config_home.join("projects/a");
        std::fs::create_dir_all(&project).unwrap();
        let file = std::fs::File::create(project.join("s.jsonl")).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(since - 100))
            .unwrap();
        assert_eq!(
            has(),
            Ok(false),
            "a session from before the run is not this run's"
        );
        file.set_modified(UNIX_EPOCH + Duration::from_secs(since))
            .unwrap();
        assert_eq!(has(), Ok(true), "written at the run's start");

        // A file the pattern does not name, or one behind a link, is not a session.
        std::fs::remove_file(project.join("s.jsonl")).unwrap();
        std::fs::write(project.join("s.txt"), b"x").unwrap();
        let elsewhere = t.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("s.jsonl"), b"x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, config_home.join("projects/b")).unwrap();
        assert_eq!(has(), Ok(false));

        // A store folder that cannot be read is not "no session": it says why.
        let store = config_home.join("projects");
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = has();
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
        let why = unreadable.expect_err("an unreadable store cannot tell");
        assert!(why.contains(&store.display().to_string()), "{why}");

        let mut none = profile.clone();
        none.native.clear();
        assert_eq!(
            has_native_session(&none, &run_home, &config_home, since),
            Ok(true)
        );
    }
}
