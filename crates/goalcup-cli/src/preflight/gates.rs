//! The pure half of [`super`]: every question that is answered by reading, and nothing that
//! starts a process. The gates that read a manifest, an environment map and a directory live
//! here; the version probe that spawns a child, kills its process group and drains its pipes
//! is in [`super::probe`]. `super`'s `pub use` re-exports every item at one path.
//!
//! The one thing that crosses the boundary is D12: `version_gate_measured` asks
//! [`super::probe::sample_harness_version`] what the harness answered, which is exactly why
//! the probe exists.

use super::probe::{PROBE_FILE_PREFIX, resolve_on_path, sample_harness_version};
use crate::connector::{Connector, HarnessConnector, PinState, pin_state};
use crate::version::{Standing, VersionRange};
use goalcup_protocol::manifest::Tier;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The one spelling of the one door. `goalcup start --dev` and
/// `goalcup doctor --verify-with-dev` are the two places a human types it, and the header of
/// [`super`] lists the four things it admits.
pub const DEV_DOOR: &str = "--dev";

/// What one check can say.
///
/// Three levels and not two, because "the harness is not installed on this machine" and
/// "the harness is installed and is the wrong version" are different answers and a
/// participant reading a `goalcup doctor` needs to tell them apart. `Warn` never fails a
/// report; [`PreflightReport::failed`] counts `Fail` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Pass,
    Warn,
    Fail,
}

impl Level {
    /// The word this level renders as, padded to one width so a report's levels line up.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Level::Pass => "PASS",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

/// One question, its answer, and what to do about the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable across releases and never translated: this is what a script greps for.
    pub id: String,
    pub level: Level,
    /// What was found, in the participant's own terms, naming the variable, the port, the
    /// digest or the version it is about.
    pub detail: String,
    /// The command or the change that makes this check stop saying what it just said.
    /// Empty only where there is nothing to do — a `Pass`.
    pub fix: String,
}

impl Check {
    /// One check as a participant reads it: the level, the id, the detail, and the fix
    /// indented under it where there is one to give.
    ///
    /// The one renderer. [`PreflightReport::render`] calls it for a connector's checks and
    /// `goalcup doctor` calls it for the daemon's, so the daemon's line and a connector's
    /// line cannot drift into two layouts that read as two reports.
    pub fn render(&self) -> String {
        let mut out = format!("  {}  {:<16}{}\n", self.level.word(), self.id, self.detail);
        if !self.fix.is_empty() {
            out.push_str(&format!("        fix: {}\n", self.fix));
        }
        out
    }

    pub(crate) fn new(id: &str, level: Level, detail: String, fix: &str) -> Check {
        Check {
            id: id.to_string(),
            level,
            detail,
            fix: fix.to_string(),
        }
    }

    pub(crate) fn pass(id: &str, detail: String) -> Check {
        Check::new(id, Level::Pass, detail, "")
    }
}

/// Every check for one connector, in the order they are asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreflightReport {
    pub connector_id: String,
    pub checks: Vec<Check>,
    /// What the D12 version check actually measured, where it got an answer.
    ///
    /// The check's own `Level` is about the range; this is the number, carried out so that
    /// `goalcup start` can put it on `WorkspaceOpts` and the daemon can write it onto the
    /// entry row. `None` where the binary is
    /// not on `PATH` or would not answer — the two `Warn` branches — and `None` too where
    /// the manifest's range is unreadable.
    ///
    /// Serialised into `goalcup doctor --json` beside the checks, deliberately: a report
    /// that says a version passed a range and does not say which version is a report a
    /// reader has to parse a sentence to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
}

impl PreflightReport {
    /// Whether anything here refuses the run. `Warn` does not: a machine without the
    /// harness installed is a report, not a broken tracker.
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.level == Level::Fail)
    }

    /// The report as a participant reads it: the connector, then [`Check::render`] for each
    /// of its checks.
    pub fn render(&self) -> String {
        let mut out = format!("{}\n", self.connector_id);
        for c in &self.checks {
            out.push_str(&c.render());
        }
        out
    }
}

/// D12 and D23 for one connector, with the environment handed in so a test never reads the
/// participant's.
///
/// Strict: a rulebook this build does not pin is a `Fail`. [`connector_checks_with_dev`] is
/// the same set with the one door open.
pub fn connector_checks(
    c: &Connector,
    env: &BTreeMap<String, String>,
    cfg: &Path,
) -> PreflightReport {
    connector_checks_with_dev(c, env, cfg, false)
}

/// [`connector_checks`], with `dev` saying whether `--dev` was typed by a human.
///
/// `dev` reaches exactly one check — [`manifest_pin_gate`] — and it never removes a check
/// or silences a report. See the header of [`super`].
pub fn connector_checks_with_dev(
    c: &Connector,
    env: &BTreeMap<String, String>,
    cfg: &Path,
    dev: bool,
) -> PreflightReport {
    // The version is measured once, here, and both what the check says about it and the
    // number itself come out of that one probe. A second `sample_harness_version`
    // to fill the field would spawn the harness twice per start and could answer differently
    // from the check standing beside it.
    let (version, measured) = version_gate_measured(c, env);
    PreflightReport {
        connector_id: c.manifest.id.clone(),
        checks: vec![
            tier_gate(c),
            manifest_pin_gate(c, dev),
            version,
            banned_env_gate(c, env),
            required_paths_gate(c, cfg),
            free_ports_gate(c),
        ],
        harness_version: measured,
    }
}

/// D23's measured tier table, as one function over `manifest.tier`.
///
/// **Only [`Tier::Full`] passes**, and the other two are refused in their own words because
/// they are different answers:
///
/// * `elevated_review` means the table measured the harness and put it behind a review,
///   which this build does not run.
/// * `unsupported` means the table says no, full stop. D23 marks **dsh** so, and dsh ships
///   no connector, so a participant reaches this refusal only on a hand-written manifest.
///
/// One `Fail` with one message for both would tell a participant nothing about which of the
/// two they hit.
pub fn tier_gate(c: &Connector) -> Check {
    let id = &c.manifest.id;
    match c.manifest.tier {
        Tier::Full => Check::pass(
            "tier",
            format!("`{id}` is `full` in D23's measured table: M1 runs it."),
        ),
        Tier::ElevatedReview => Check::new(
            "tier",
            Level::Fail,
            format!(
                "`{id}` is `elevated_review` in D23's measured table: the table measured \
                 this harness and put it behind a review, and M1 does not run one. It is \
                 refused in M1 rather than refused forever — a later milestone that stands \
                 the review up admits this connector unchanged."
            ),
            "run one of the connectors D23 marks `full`, or wait for the milestone that \
             runs the review.",
        ),
        Tier::Unsupported => Check::new(
            "tier",
            Level::Fail,
            format!(
                "`{id}` is `unsupported` in D23's measured table: this harness is not \
                 supported, in M1 or later. The only harness D23 marks so is dsh, which \
                 ships no connector, so this manifest is a hand-written one."
            ),
            "run one of the connectors this build ships.",
        ),
    }
}

/// The release-hash allowlist, asked at start time.
///
/// The manifest supplies the human rule, the exclusion catalogue and the tool classes: it
/// is the rulebook, and a rulebook the participant may rewrite is not a rulebook. The
/// verifier will call an edited one altered in the verdict; this is the same question asked
/// where the answer is still worth something.
///
/// `dev` is the one door. It does not remove the check and it does not silence the report:
/// the check stays, at [`Level::Warn`], and its detail names `--dev` — so a report produced
/// under the door can never be read as a report produced without it.
pub fn manifest_pin_gate(c: &Connector, dev: bool) -> Check {
    let id = &c.manifest.id;
    // `pin_state` compares the directory digest (manifest plus every template under it),
    // so that is the digest the refusal names.
    let found = &c.dir_sha256;
    match pin_state(c) {
        PinState::Pinned => Check::pass(
            "manifest_pin",
            format!("`{id}`'s rulebook is the one this build pins ({found})."),
        ),
        PinState::Altered { pinned } => {
            let what = format!(
                "`{id}`'s rulebook is not the one this build pins: it is {found}, and this \
                 build pins {pinned}. The manifest carries the human rule, the exclusion \
                 catalogue and the tool classes, so an edited one is a run judged by rules \
                 nobody agreed to."
            );
            dev_or_fail(what, dev)
        }
        PinState::Unpinned => {
            let what = format!(
                "no build pins `{id}` — its rulebook ({found}) is hand-written, or from a \
                 release this build does not know. Nothing has agreed to the rules this run \
                 would be judged by."
            );
            dev_or_fail(what, dev)
        }
    }
}

/// The schema-1 capture connector directory pin, using the same one-door levels and wording.
///
/// A connector whose declared id is pinned never reaches the Dev door: an altered copy remains
/// a failure even when `--dev` was typed. The loader separately refuses `Origin::Dev` for any
/// directory whose digest is a shipped pin.
pub fn harness_pin_gate(c: &HarnessConnector, dev: bool) -> Check {
    let id = &c.loaded.manifest.id;
    let found = &c.loaded.connector_dir_sha256;
    match goalcup_protocol::pins::pinned_connector_dir_sha256(id) {
        Some(_)
            if goalcup_protocol::pins::is_pinned(
                goalcup_protocol::pins::CONNECTOR_DIR_SHA256S,
                id,
                found,
            ) =>
        {
            Check::pass(
                "manifest_pin",
                format!("`{id}`'s rulebook is the one this build pins ({found})."),
            )
        }
        Some(pinned) => Check::new(
            "manifest_pin",
            Level::Fail,
            format!(
                "`{id}`'s rulebook is not the one this build pins: it is {found}, and this \
                 build pins {pinned}. A pinned connector never receives {DEV_DOOR} relaxations."
            ),
            &format!("restore the shipped `connectors/{id}` directory."),
        ),
        None => {
            let what = format!(
                "no build pins `{id}` — its rulebook ({found}) is hand-written, or from a \
                 release this build does not know. Nothing has agreed to the rules this run \
                 would be judged by."
            );
            dev_or_fail(what, dev)
        }
    }
}

/// The one door, applied. Named so the two `manifest_pin_gate` arms cannot word it
/// differently from each other.
fn dev_or_fail(what: String, dev: bool) -> Check {
    if dev {
        Check::new(
            "manifest_pin",
            Level::Warn,
            format!(
                "{what} {DEV_DOOR} was typed, and {DEV_DOOR} is the only thing that admits \
                 it. This run is a development run and its results are not a judged entry's."
            ),
            "",
        )
    } else {
        Check::new(
            "manifest_pin",
            Level::Fail,
            what,
            &format!(
                "restore the shipped `connectors/<id>/manifest.toml`, or pass {DEV_DOOR} \
                 and accept that the run is a development run."
            ),
        )
    }
}

/// D12: which version of the harness is on this machine, against the range the connector
/// was tested at, with the number it measured beside the verdict it reached about it.
///
/// The range is quoted in **every** branch, including the one where the binary is not
/// installed, because the number a participant needs is the number they have to install and
/// it is no less useful when they have nothing yet.
///
/// A binary that is not on `PATH` is a [`Level::Warn`]: a machine without the harness
/// installed is a `goalcup doctor` report, not a broken tracker. A probe that could not get
/// an answer out of an installed harness is a `Warn` too — "this build could not measure it"
/// is not the same claim as "it is the wrong version", and only the second one is a `Fail`.
///
/// Under D12 "the wrong version" means below the range's floor. A version newer than the
/// tested range is a `Warn` — allowed and reported as untested — never a `Fail`; see
/// [`crate::version::Standing`].
///
/// The verdict and the number are one probe. `WorkspaceOpts.harness_version` carries the
/// measurement to the daemon, which runs no version command of its own. The number is `None`
/// on all three branches that reach none: an unreadable range, a binary that is not on
/// `PATH`, a probe that would not answer. A version outside the range is still returned; the
/// run is refused before anything can use it, and `goalcup doctor --json` is owed the number
/// it is complaining about.
fn version_gate_measured(c: &Connector, env: &BTreeMap<String, String>) -> (Check, Option<String>) {
    let m = &c.manifest;
    let binary = &m.detection.binary;
    let want = &m.tested_version_range;
    let range = match VersionRange::parse(want) {
        Ok(range) => range,
        // A range this build cannot read is the manifest's defect, not the machine's, and
        // it is reported as one rather than turned into a version answer nobody can act on.
        Err(e) => {
            return (
                Check::new(
                    "harness_version",
                    Level::Fail,
                    format!(
                        "`{}`'s `tested_version_range` ({want:?}) is unreadable: {e}",
                        m.id
                    ),
                    "this is a defect in the connector manifest, not on this machine: report \
                     it.",
                ),
                None,
            );
        }
    };
    if resolve_on_path(binary, env).is_none() {
        return (
            Check::new(
                "harness_version",
                Level::Warn,
                format!(
                    "`{binary}` is not on PATH, so its version could not be measured. D12 \
                     tested `{}` at `{want}`.",
                    m.id
                ),
                &format!(
                    "install `{binary}` at {} or newer and put it on PATH.",
                    range.floor()
                ),
            ),
            None,
        );
    }
    match sample_harness_version(c, env) {
        Ok(version) => {
            let floor = range.floor();
            let check = match range.standing(&version) {
                Standing::Tested => Check::pass(
                    "harness_version",
                    format!(
                        "`{binary}` is {version}, which is inside D12's tested range `{want}`."
                    ),
                ),
                // D12: a newer harness is never refused. It runs, and
                // the report says it is untested, so a run on it reads as what it is.
                Standing::NewerThanTested => Check::new(
                    "harness_version",
                    Level::Warn,
                    format!(
                        "`{binary}` is {version}, newer than D12's tested range `{want}`: \
                         allowed, and recorded as untested."
                    ),
                    &format!(
                        "nothing is required. `{want}` is the tested version if you want a \
                         tested run."
                    ),
                ),
                Standing::BelowFloor => Check::new(
                    "harness_version",
                    Level::Fail,
                    format!(
                        "`{binary}` is {version}, below D12's floor {floor} (tested range \
                         `{want}`). A self-updating harness moves under a run, which is why \
                         this is measured at the start rather than discovered in the verdict."
                    ),
                    &format!("install `{binary}` at {floor} or newer."),
                ),
            };
            (check, Some(version))
        }
        Err(e) => (
            Check::new(
                "harness_version",
                Level::Warn,
                format!(
                    "`{binary}` is installed and this build could not measure its version: \
                     {e}. D12 tested `{}` at `{want}`.",
                    m.id
                ),
                &format!(
                    "check that `{}` answers on this machine.",
                    m.detection.version_command
                ),
            ),
            None,
        ),
    }
}

/// The manifest's `preflight.banned_env`: variables whose mere presence refuses the start.
///
/// Every one that is set is named — not the first — because a participant who unsets one
/// and runs again only to be refused for the next has been made to do the work twice.
pub fn banned_env_gate(c: &Connector, env: &BTreeMap<String, String>) -> Check {
    let banned = &c.manifest.preflight.banned_env;
    let present: Vec<&String> = banned.iter().filter(|k| env.contains_key(*k)).collect();
    if present.is_empty() {
        return Check::pass(
            "banned_env",
            if banned.is_empty() {
                format!("`{}` bans no environment variable.", c.manifest.id)
            } else {
                format!(
                    "none of `{}`'s banned variables is set ({}).",
                    c.manifest.id,
                    banned.join(", ")
                )
            },
        );
    }
    let names: Vec<String> = present.iter().map(|k| (*k).clone()).collect();
    Check::new(
        "banned_env",
        Level::Fail,
        format!(
            "`{}` refuses a run with {} set: these change what the harness talks to or what \
             it records, so a run under one is not the run the trace describes.",
            c.manifest.id,
            names.join(", ")
        ),
        &format!("unset {} and run this again.", names.join(" ")),
    )
}

/// The manifest's `preflight.required_paths`, as far as they can be known before a run.
///
/// Most of them are templates over things that do not exist until a run does, and there is
/// nothing on this machine yet to check them against.
///
/// The daemon creates the run's roots — the harness home, the artifact root — before the
/// launch.
///
/// What is checkable is checked: a literal path must exist, and the tracker's own
/// configuration directory must be a directory this process can write, because a machine
/// that cannot write there cannot hold an entry either. The templates are printed rather
/// than silently skipped, so a reader can see which paths were deferred and to whom.
pub fn required_paths_gate(c: &Connector, cfg: &Path) -> Check {
    let required = &c.manifest.preflight.required_paths;
    let (templated, literal): (Vec<&String>, Vec<&String>) =
        required.iter().partition(|p| p.contains('{'));
    let mut missing: Vec<String> = literal
        .iter()
        .filter(|p| !Path::new(p.as_str()).exists())
        .map(|p| (*p).clone())
        .collect();
    if let Err(e) = writable(cfg) {
        missing.push(format!("{} ({e})", cfg.display()));
    }
    let deferred = if templated.is_empty() {
        String::new()
    } else {
        // The daemon creates the roots — `{harness_home}`, `{artifact_root}` — before the
        // launch; the templates over them are printed as deferred and not checked.
        format!(
            " Deferred to the run: {}. The daemon creates the roots (the harness home, the \
             artifact root) before the launch; a store directory under one of them appears \
             when the harness first writes into it, not when the run starts.",
            templated
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if missing.is_empty() {
        Check::pass(
            "required_paths",
            format!(
                "everything `{}` needs before a run is in place.{deferred}",
                c.manifest.id
            ),
        )
    } else {
        Check::new(
            "required_paths",
            Level::Fail,
            format!(
                "`{}` cannot start: {} is missing or not writable.{deferred}",
                c.manifest.id,
                missing.join(", ")
            ),
            "create the path, or make it writable by the account running `goalcup`.",
        )
    }
}

/// The manifest's `preflight.free_ports`.
///
/// A harness that binds a fixed port fails **after** the workspace has been created and the
/// folder armed if something else holds it, and a participant whose run died there has an
/// entry, a workspace and no harness. Asking now costs one bind. No shipped connector
/// declares a port.
pub fn free_ports_gate(c: &Connector) -> Check {
    let ports = &c.manifest.preflight.free_ports;
    if ports.is_empty() {
        return Check::pass(
            "free_ports",
            format!("`{}`'s launch reserves no port.", c.manifest.id),
        );
    }
    let held: Vec<String> = ports
        .iter()
        .filter(|p| !port_is_free(**p))
        .map(|p| p.to_string())
        .collect();
    if held.is_empty() {
        Check::pass(
            "free_ports",
            format!(
                "`{}`'s launch needs {} free on 127.0.0.1, and it is.",
                c.manifest.id,
                ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    } else {
        Check::new(
            "free_ports",
            Level::Fail,
            format!(
                "`{}`'s launch needs port {} free on 127.0.0.1, and something else holds \
                 it. The launch would fail after the workspace and the entry already exist.",
                c.manifest.id,
                held.join(", ")
            ),
            &format!(
                "stop whatever is listening on {} (`lsof -nP -iTCP:{} -sTCP:LISTEN`) and \
                 run this again.",
                held.join(", "),
                held.join(",")
            ),
        )
    }
}

/// The port a stopped run reserved, checked before it is relaunched into.
///
/// The daemon **remembers** the port it reserved on the registry row and never re-reserves
/// it, so between the stop and the resume anything at all may have taken it.
/// `ResumeInfo.server_port` is `None` for every connector that reserves no port, which is why
/// this answers `None` rather than a `Pass` nobody asked for.
pub fn server_port_gate(port: Option<u16>) -> Option<Check> {
    let port = port?;
    if port_is_free(port) {
        Some(Check::pass(
            "server_port",
            format!("the port this run reserved ({port}) is still free."),
        ))
    } else {
        Some(Check::new(
            "server_port",
            Level::Fail,
            format!(
                "the port this run reserved ({port}) is held by something else now. The \
                 daemon remembers the port it reserved and does not re-reserve it, so a \
                 relaunch would bind nothing."
            ),
            &format!(
                "stop whatever is listening on {port} (`lsof -nP -iTCP:{port} \
                 -sTCP:LISTEN`) and resume again."
            ),
        ))
    }
}

/// Whether 127.0.0.1:`port` can be bound right now.
///
/// The listener is dropped immediately: this asks a question, it does not hold an answer.
/// Loopback and not `0.0.0.0`, because loopback is what the launch binds — a check on a
/// wider address would refuse a port that is perfectly available to the run.
fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Whether `dir` is a directory this process can create a file in.
///
/// Asked by creating one and removing it, rather than by reading a mode: the mode is not
/// the answer on a directory owned by another user, inside a read-only mount, or under a
/// sandbox, and every one of those is a machine a run would fail on.
fn writable(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err("not a directory".to_string());
    }
    let probe = dir.join(format!("{PROBE_FILE_PREFIX}{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}
