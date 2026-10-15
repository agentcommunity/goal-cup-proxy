//! `goalcup doctor` — is the daemon there, and what can it see.
//!
//! This command is [`crate::preflight`] run over every shipped connector's `harness.toml` with
//! the daemon's state added, and it leaves under the CLI's own ladder: **0** when nothing failed,
//! **1** when anything did. Per connector it checks the connector files' pin, the harness
//! executable `goalcup start` would run (found on `PATH` and hashed, never started, and any build
//! accepted, since it is recorded rather than pinned (D53): [`crate::launch::capture_executable`],
//! the check `start` itself makes) and the
//! decoders. No daemon running is not a failure: the daemon is run-scoped, and
//! `goalcup start` starts one for the run, so the check fails only when none could be started
//! (no `goalcup-daemon` to find) or one answers that this build cannot speak to. A sealed capture
//! is checked by `goalcup-verify --capture-integrity`, not here; the V1 `--verify` mode was
//! removed with the rest of the V1 path (D48).
//!
//! # `--harness <id>` narrows the report and the number
//!
//! Unscoped (`--harness all`, or no `prep.json` yet), this command reports every shipped
//! connector and exits 1 if any of them failed. With no flag it is scoped to the harness
//! `goalcup prep` settled.
//! "Is this machine healthy" and "can I start this harness" are two questions with two right
//! answers, and a participant about to run one harness is asking the second. The flag scopes
//! the report and the number. The daemon check is in both, because no harness starts
//! without one. A harness id no connector here declares is rung 1 naming the ids that are
//! there, not an empty report that exits 0.

use crate::connector::HarnessConnector;
use crate::preflight::{self, Check, Level, PreflightReport, harness_pin_gate};
use crate::{CliError, config, connector};
use goalcup_protocol::control::{CONTROL_API_VERSION, DaemonClient};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `--harness all`: check every shipped connector, whatever `goalcup prep` settled.
pub const ALL_HARNESSES: &str = "all";

/// What the flags asked for. Built by `main.rs` and by tests, never assembled from the
/// environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctorOptions {
    pub json: bool,
    /// `--harness <id>`: report on this connector alone, and leave under its result alone.
    ///
    /// Unscoped, `goalcup doctor` reports every shipped connector and exits 1 if any of them
    /// failed. "Is this machine healthy" and "can I start this harness" are different
    /// questions; the flag picks the second, and the number follows the report.
    ///
    /// The daemon check is in both readings: a harness cannot be started without one.
    ///
    /// Without the flag the harness in `prep.json` is checked. `all` (or no `prep.json`) checks
    /// every shipped connector.
    pub harness: Option<String>,
}

/// The whole report, for `--json`. One document rather than a rendering a caller has to
/// parse back out of a text block.
#[derive(Debug, Serialize)]
struct DoctorReport<'a> {
    daemon: &'a Check,
    connectors: &'a [PreflightReport],
}

/// `goalcup doctor`, with everything it reads handed in.
///
/// `client` is `None` where `HttpDaemonClient::from_config_dir` refused (no `control.json`,
/// or a control API this build does not speak), which is why the no-client branch reads
/// `control.json` itself rather than reporting a failure it has not actually observed.
///
/// `env` is the environment `goalcup start` would launch from: its `PATH` is where each
/// harness executable is looked for.
///
/// Returns the exit code rather than taking it, so the caller builds the one `ExitCode`.
/// Nothing this command prints goes to standard error, so it takes no stream for it.
pub fn run_with(
    client: Option<&dyn DaemonClient>,
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &DoctorOptions,
    out: &mut dyn Write,
) -> Result<u8, CliError> {
    // Before anything is checked: a `.goalcup-preflight-<pid>` in the participant's
    // configuration directory is litter from an older doctor killed mid-check, and the
    // diagnostic that goes looking at this directory is the right place to take it away.
    preflight::sweep_stale_probes(cfg);
    let daemon = daemon_check(client, cfg);
    let mut reports = connector_reports(connectors, env)?;
    // `--harness <id>` narrows both the report and the number. Everything is still
    // loaded first — `connector_reports` is what refuses two directories declaring one id,
    // and that refusal is about the installation rather than about the harness asked after —
    // and then the list is narrowed to the one connector.
    // No flag: the harness `goalcup prep` settled, so a harness never installed cannot fail
    // the run of one that is. `--harness all`, or no `prep.json` yet, checks every connector.
    let mut from_prep = opts.harness.is_none();
    let scope: Option<String> = match opts.harness.as_deref() {
        Some(ALL_HARNESSES) => None,
        Some(id) => Some(id.to_string()),
        // A prep that names an id no connector here declares (a stale prep after an upgrade)
        // must not make the command that runs when nothing works refuse: check everything.
        None => Some(crate::prep_file::read_prep_lenient(cfg).harness)
            .filter(|id| reports.iter().any(|r| &r.connector_id == id)),
    };
    from_prep &= scope.is_some();
    if let Some(id) = &scope {
        let all: Vec<String> = reports.iter().map(|r| r.connector_id.clone()).collect();
        reports.retain(|r| &r.connector_id == id);
        if reports.is_empty() {
            return Err(CliError::Refused(format!(
                "no connector here declares the id `{id}`. This build ships: {}.",
                all.join(", ")
            )));
        }
    }
    if opts.json {
        let document = DoctorReport {
            daemon: &daemon,
            connectors: &reports,
        };
        put(
            out,
            &format!("{}\n", serde_json::to_string_pretty(&document)?),
        )?;
    } else {
        match &scope {
            Some(id) if from_prep => put(
                out,
                &format!("goalcup doctor — {id} (from goalcup prep)\n\n"),
            )?,
            Some(id) => put(out, &format!("goalcup doctor — {id} only\n\n"))?,
            None => put(out, "goalcup doctor\n\n")?,
        }
        put(out, &daemon.render())?;
        for report in &reports {
            put(out, "\n")?;
            put(out, &report.render())?;
        }
    }
    let failed = daemon.level == Level::Fail || reports.iter().any(PreflightReport::failed);
    Ok(u8::from(failed))
}

/// One report per connector directory (`<dir>/*/harness.toml`), in id order.
///
/// Directories and not ids, and the difference is the whole point: discovery keys on the id a
/// manifest **declares**, an unloadable manifest declares nothing, and a roster built from ids
/// would drop exactly the connector this command exists to find. So each directory is loaded
/// on its own and a load that fails is a `Fail` naming the path.
///
/// One id declared by two directories is the whole roster's refusal rather than one
/// connector's `Fail`: nothing here can say which of the two a `goalcup start` would run.
fn connector_reports(
    connectors: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Vec<PreflightReport>, CliError> {
    let mut out: Vec<(PathBuf, PreflightReport)> = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for dir in connector::harness_dirs(connectors)? {
        let named = dir.display().to_string();
        match goalcup_protocol::harness::load_discovered_dir(
            &dir,
            goalcup_protocol::harness::Origin::Pinned,
        ) {
            Ok(loaded) => {
                let id = loaded.manifest.id.clone();
                if let Some(first) = seen.insert(id.clone(), named.clone()) {
                    return Err(CliError::Refused(format!(
                        "connector {id:?} is declared by two harness directories: {first} and \
                         {named}. Discovery keys on the id a manifest declares."
                    )));
                }
                let decoder = decoder_check(&loaded.manifest);
                let connector = HarnessConnector {
                    dir: dir.clone(),
                    loaded,
                };
                let pin = harness_pin_gate(&connector, false);
                let executable = executable_check(&connector, env);
                out.push((
                    dir,
                    PreflightReport {
                        connector_id: id,
                        checks: vec![pin, executable, decoder],
                    },
                ));
            }
            Err(error) => out.push((
                dir,
                PreflightReport {
                    connector_id: named.clone(),
                    checks: vec![Check::new(
                        "harness_manifest",
                        Level::Fail,
                        format!("the harness connector in {named} could not be loaded: {error}."),
                        "reinstall the shipped connectors directory or remove this directory.",
                    )],
                },
            )),
        }
    }
    out.sort_by(|(a_dir, a), (b_dir, b)| {
        a.connector_id
            .cmp(&b.connector_id)
            .then_with(|| a_dir.cmp(b_dir))
    });
    Ok(out.into_iter().map(|(_, report)| report).collect())
}

/// The harness executable `goalcup start` would run: found on `env`'s `PATH` and checked, by the
/// same function `start` calls, so it also checks that a `#!/usr/bin/env <interpreter>` harness
/// (an npm one) finds its interpreter on the PATH the harness is given (the participant's own,
/// for a chat-mode connector). Any build passes: it is recorded, not pinned (D53). A harness
/// that is not installed, not a regular file, or missing its interpreter is a `Fail`: `start`
/// would refuse it.
pub(crate) fn executable_check(c: &HarnessConnector, env: &BTreeMap<String, String>) -> Check {
    let program = &c.loaded.manifest.capture.executable.program;
    match crate::launch::capture_executable(c, &c.loaded.manifest.capture, env) {
        Ok(resolved) => Check::pass(
            "harness_executable",
            format!(
                "`{program}` resolves to `{}` and starts on the PATH the harness is given; the \
                 build that runs is recorded, not pinned.",
                resolved.path()
            ),
        ),
        Err(error) => Check::new(
            "harness_executable",
            Level::Fail,
            error.to_string(),
            &format!(
                "install `{program}` on PATH: `goalcup start` runs whichever build it finds and \
                 records it."
            ),
        ),
    }
}

fn decoder_check(manifest: &goalcup_protocol::harness::HarnessManifest) -> Check {
    use goalcup_protocol::capture::{CaptureSource, StreamKey};
    use goalcup_verifier::capture_analysis::api::StreamOpen;
    use goalcup_verifier::capture_analysis::registry::DecoderRegistry;

    let registry = DecoderRegistry::builtin();
    let key = StreamKey {
        producer: "doctor".into(),
        stream: 1,
    };
    let mut accepted = BTreeSet::new();
    let mut undecoded = BTreeSet::new();
    for host in &manifest.capture.hosts {
        if let Some(hint) = host.protocol_hint.as_deref() {
            let open = StreamOpen::new(&key, CaptureSource::WireRequest, Some(hint), None);
            if registry.accepting(&open).is_empty() {
                undecoded.insert(hint.to_string());
            } else {
                accepted.insert(hint.to_string());
            }
        }
    }
    for native in &manifest.capture.native {
        if let Some(hint) = native.protocol_hint.as_deref() {
            let open = StreamOpen::new(&key, CaptureSource::Native, Some(hint), None);
            if registry.accepting(&open).is_empty() {
                undecoded.insert(hint.to_string());
            } else {
                accepted.insert(hint.to_string());
            }
        }
    }
    if undecoded.is_empty() {
        Check::pass(
            "decoders",
            if accepted.is_empty() {
                "the harness declares no protocol hint requiring a decoder.".into()
            } else {
                format!(
                    "the built-in decoder registry accepted: {}.",
                    accepted.into_iter().collect::<Vec<_>>().join(", ")
                )
            },
        )
    } else {
        Check::new(
            "decoders",
            Level::Warn,
            format!(
                "no decoder accepts {} for the declared stream source; those streams remain \
                 capture-only.",
                undecoded
                    .into_iter()
                    .map(|hint| format!("`{hint}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "add a measured decoder, or retain the streams as capture-only.",
        )
    }
}

/// Is the daemon there, and if not, can `goalcup start` start one.
///
/// With a client, one `health()` call answers it. Without one, `control.json` is the only
/// thing a second process can read to find out where a running daemon is listening, and the
/// check names that path. A daemon that is not running is the normal state between runs, so
/// it passes when the `goalcup-daemon` binary a run would start can be found, and fails
/// naming where it was looked for when it cannot.
fn daemon_check(client: Option<&dyn DaemonClient>, cfg: &Path) -> Check {
    let path = config::control_path(cfg);
    let unreachable = |e: String| match crate::daemon::spawn::daemon_binary() {
        Ok(binary) => Check::pass(
            "daemon",
            // `{e}` and no prefix of this command's own: `ControlError::Unreachable` and
            // `read_control_file`'s "no daemon" both already say what happened.
            format!(
                "{e} None is needed until a run: `goalcup start` starts {} for it.",
                binary.display()
            ),
        ),
        Err(cannot) => Check::new(
            "daemon",
            Level::Fail,
            format!("{e} {cannot}"),
            &format!(
                "put goalcup-daemon beside goalcup; a daemon writes {} when it is up.",
                path.display()
            ),
        ),
    };
    let Some(client) = client else {
        return match config::read_control_file(cfg) {
            // The file is there and this run has no client to ask with: a control file this
            // build cannot make a client from, and the one such file is a daemon speaking
            // another version of the API — a `Fail` naming both numbers, because the fix is to
            // install one of the two again.
            Ok(file) if file.control_api_version != CONTROL_API_VERSION => Check::new(
                "daemon",
                Level::Fail,
                format!(
                    "{} says daemon v{} speaks control API v{}, and this goalcup speaks \
                     v{CONTROL_API_VERSION}. Nothing here can ask it anything.",
                    path.display(),
                    file.daemon_version,
                    file.control_api_version
                ),
                "upgrade whichever of the daemon and the CLI is older, and start the daemon \
                 again.",
            ),
            // The file is there, the version matches, and the caller still handed over no
            // client — which is a test driving this check, not a machine. A `Fail` would
            // report a daemon down that may be perfectly up.
            Ok(file) => Check::new(
                "daemon",
                Level::Warn,
                format!(
                    "{} says daemon v{} is listening on {}; this run has no control client \
                     to ask it anything.",
                    path.display(),
                    file.daemon_version,
                    file.address
                ),
                "",
            ),
            Err(e) => unreachable(e.to_string()),
        };
    };
    // Through the retry like every other daemon call a command makes, and
    // `tests/it/start_launch.rs` sweeps `src/` for one that is not. `health` is a read and the
    // daemon has no reason to answer `busy` on it; the helper costs nothing when it does not,
    // and an exemption is a rule nobody can grep for.
    match crate::retry::retry_busy(|| client.health()) {
        Ok(h) => {
            let up = format!(
                "daemon v{} is up (control API v{}, {} entr{} open).",
                h.daemon_version,
                h.control_api_version,
                h.entries_open,
                if h.entries_open == 1 { "y" } else { "ies" }
            );
            if h.reopen_failed.is_empty() {
                Check::pass("daemon", up)
            } else {
                reopen_failed_warning(&up, &h.reopen_failed)
            }
        }
        Err(e) => unreachable(e.to_string()),
    }
}

/// The daemon is up but could not reopen some entries at start: a `Warn`, since the daemon
/// itself works and the run can go on. Such an entry has no status and routes nothing, and
/// `goalcup seal` refuses it. `goalcup continue` closes it from the tail of its chain as an
/// incomplete segment (nothing repaired) and opens the next one.
fn reopen_failed_warning(up: &str, ids: &[String]) -> Check {
    let one = ids.len() == 1;
    Check::new(
        "daemon",
        Level::Warn,
        format!(
            "{up} It could not reopen {}: {}. {} no status and route{} nothing.",
            if one { "this entry" } else { "these entries" },
            ids.join(", "),
            if one { "It has" } else { "They have" },
            if one { "s" } else { "" },
        ),
        "run `goalcup continue` to close the entry from the tail of its chain as an incomplete \
         segment and go on; `goalcup seal` cannot finish it. Its trace bytes stay as they are, \
         for `goalcup-verify --capture-integrity`.",
    )
}

/// Every byte this command prints goes through here, so a closed pipe is a [`CliError::Io`]
/// naming the stream rather than an `unwrap` in a command a participant is watching.
fn put(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    put_named(out, "standard output", text)
}

/// [`put`] with the stream named, for the one line that does not go to stdout.
fn put_named(out: &mut dyn Write, name: &str, text: &str) -> Result<(), CliError> {
    out.write_all(text.as_bytes())
        .map_err(|source| CliError::Io {
            path: name.to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use goalcup_protocol::control::{
        ControlError, DaemonHealth, EntrySummary, SealResponse, SignedStatus,
    };
    use goalcup_protocol::process::ProcessRegistration;

    /// A daemon that answers `health` and nothing else.
    struct HealthOnly(DaemonHealth);

    impl DaemonClient for HealthOnly {
        fn status(&self, _: &str) -> Result<SignedStatus, ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
        fn entries(&self) -> Result<Vec<EntrySummary>, ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
        fn seal(&self, _: &str) -> Result<SealResponse, ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
        fn register_process(&self, _: &str, _: &ProcessRegistration) -> Result<(), ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
        fn health(&self) -> Result<DaemonHealth, ControlError> {
            Ok(self.0.clone())
        }
        fn current_entry(&self) -> Result<Option<String>, ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
        fn abandon(&self, _: &str, _: &str) -> Result<(), ControlError> {
            Err(ControlError::Unsupported("test".to_string()))
        }
    }

    fn health(reopen_failed: &[&str]) -> DaemonHealth {
        DaemonHealth {
            control_api_version: CONTROL_API_VERSION,
            daemon_version: "a-test-daemon".to_string(),
            uptime_seconds: 1,
            entries_open: 2,
            cycles: Default::default(),
            reopen_failed: reopen_failed.iter().map(ToString::to_string).collect(),
        }
    }

    /// A daemon that could not reopen an entry is a warning that names the entry ids and the
    /// command to run, and it is not a failure: the daemon works and the run can go on. With
    /// none failed, the check is the plain pass it was.
    #[test]
    fn a_daemon_that_could_not_reopen_entries_is_a_warning_naming_their_ids() {
        let cfg = Path::new("/nonexistent-goalcup-cfg");

        let clean = HealthOnly(health(&[]));
        let check = daemon_check(Some(&clean), cfg);
        assert_eq!(check.level, Level::Pass, "{check:?}");

        let lost = HealthOnly(health(&["entry-aaa", "entry-bbb"]));
        let check = daemon_check(Some(&lost), cfg);
        assert_eq!(check.level, Level::Warn, "{check:?}");
        assert!(check.detail.contains("entry-aaa, entry-bbb"), "{check:?}");
        assert!(
            check.detail.contains("daemon va-test-daemon is up"),
            "{check:?}"
        );
        assert!(check.fix.contains("goalcup continue"), "{check:?}");
        let text = check.render();
        assert!(text.contains("WARN"), "{text}");
        assert!(text.contains("entry-aaa"), "{text}");

        let one = HealthOnly(health(&["entry-aaa"]));
        let check = daemon_check(Some(&one), cfg);
        assert!(check.detail.contains("this entry: entry-aaa"), "{check:?}");
        assert!(check.detail.contains("It has no status"), "{check:?}");
    }
}
