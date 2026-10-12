//! `goalcup doctor` — is the daemon there, and what can it see.
//!
//! This command is [`crate::preflight`] run over every shipped connector with the daemon's
//! reachability added, and it leaves under the CLI's own ladder: **0** when nothing failed,
//! **1** when anything did — including on a machine with no daemon running. A sealed capture
//! is checked by `goalcup-verify --capture-integrity`, not here; the V1 `--verify` mode was
//! removed with the rest of the V1 path (D48).
//!
//! # `--harness <id>` narrows the report and the number
//!
//! Unscoped, this command reports every shipped connector and exits 1 if any of them failed.
//! "Is this machine healthy" and "can I start this harness" are two questions with two right
//! answers, and a participant about to run one harness is asking the second. The flag scopes
//! the report and the number. The daemon check is in both, because no harness starts
//! without one. A harness id no connector here declares is rung 1 naming the ids that are
//! there, not an empty report that exits 0.

use crate::preflight::{self, Check, Level, PreflightReport, connector_checks};
use crate::{CliError, config, connector};
use goalcup_protocol::control::{CONTROL_API_VERSION, DaemonClient};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

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
/// Returns the exit code rather than taking it, so the caller builds the one `ExitCode`.
/// `_err` is unused: nothing this command prints goes to standard error.
pub fn run_with(
    client: Option<&dyn DaemonClient>,
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
    opts: &DoctorOptions,
    out: &mut dyn Write,
    _err: &mut dyn Write,
) -> Result<u8, CliError> {
    // Before anything is checked, and before `required_paths_gate` creates one of its own: a
    // `.goalcup-preflight-<pid>` in the participant's configuration directory is litter from
    // a process killed mid-check, and the diagnostic that goes looking at this directory is
    // the right place to take it away.
    preflight::sweep_stale_probes(cfg);
    let daemon = daemon_check(client, cfg);
    let mut reports = connector_reports(connectors, cfg, env)?;
    // `--harness <id>` narrows both the report and the number. Everything is still
    // loaded first — `connector::list` above is what refuses two directories declaring one
    // id, and that refusal is about the installation rather than about the harness asked
    // after — and then the list is narrowed to the one connector.
    if let Some(id) = &opts.harness {
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
        match &opts.harness {
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

/// One report per connector directory, in path order.
///
/// Directories and not `connector::list`'s ids, and the difference is the whole point:
/// discovery keys on the id a manifest **declares**, an unparseable manifest declares
/// nothing, and a roster built from ids therefore drops exactly the connector this command
/// exists to find.
///
/// So each directory is loaded on its own and a load that fails is a `Fail` naming the path.
/// The path and not an id, because a manifest that did not parse has no id to name.
///
/// One id declared by two directories is still the whole roster's refusal rather than one
/// connector's `Fail` — nothing here can say which of the two a `goalcup start` would run —
/// and `connector::list` is where that refusal is made, before a line is printed.
fn connector_reports(
    connectors: &Path,
    cfg: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Vec<PreflightReport>, CliError> {
    connector::list(connectors)?;
    let mut out: Vec<(PathBuf, PreflightReport)> = Vec::new();
    for dir in connector::dirs(connectors)? {
        let named = dir.display().to_string();
        match connector::load_dir(&dir) {
            Ok(c) => out.push((dir, connector_checks(&c, env, cfg))),
            Err(e) => out.push((
                dir,
                PreflightReport {
                    connector_id: named.clone(),
                    // Nothing was probed: the manifest did not parse, so there is no
                    // `detection.version_command` to run.
                    harness_version: None,
                    checks: vec![Check::new(
                        "manifest",
                        Level::Fail,
                        format!(
                            "the connector in {named} could not be loaded: {e}. This build \
                             cannot read it, so nothing can be started under it."
                        ),
                        "reinstall the shipped `connectors/` beside the binary, or remove this \
                         directory.",
                    )],
                },
            )),
        }
    }

    // Harness-contract inventory is additive during the transition. A directory that also has
    // a V1 manifest is one connector, not two reports; its existing V1 probe stays only where
    // V1 can safely run it. A harness-only connector is described from the manifest and never
    // spawned by doctor.
    let mut seen_harness: BTreeMap<String, String> = BTreeMap::new();
    for dir in connector::harness_dirs(connectors)? {
        let named = dir.display().to_string();
        match goalcup_protocol::harness::load_discovered_dir(
            &dir,
            goalcup_protocol::harness::Origin::Pinned,
        ) {
            Ok(loaded) => {
                let id = loaded.manifest.id.clone();
                if let Some(first) = seen_harness.insert(id.clone(), named.clone()) {
                    return Err(CliError::Refused(format!(
                        "connector {id:?} is declared by two harness directories: {first} and \
                         {named}. Discovery keys on the id a manifest declares."
                    )));
                }
                let decoder = decoder_check(&loaded.manifest);
                match out.iter_mut().find(|(report_dir, _)| report_dir == &dir) {
                    Some((_, report)) => report.checks.push(decoder),
                    None => out.push((
                        dir,
                        PreflightReport {
                            connector_id: id.clone(),
                            harness_version: None,
                            checks: vec![harness_only_check(&id), decoder],
                        },
                    )),
                }
            }
            Err(error) => {
                // A dual-manifest directory already has a V1 report; add one harness failure to
                // that report rather than producing a second connector row.
                if let Some(report) = out
                    .iter_mut()
                    .find(|(report_dir, _)| report_dir == &dir)
                    .map(|(_, report)| report)
                {
                    report.checks.push(Check::new(
                        "harness_manifest",
                        Level::Fail,
                        format!("the harness manifest in {named} could not be loaded: {error}."),
                        "restore the shipped harness.toml and connector files.",
                    ));
                } else {
                    out.push((
                        dir,
                        PreflightReport {
                            connector_id: named.clone(),
                            harness_version: None,
                            checks: vec![Check::new(
                                "harness_manifest",
                                Level::Fail,
                                format!(
                                    "the harness connector in {named} could not be loaded: \
                                     {error}."
                                ),
                                "reinstall the shipped connectors directory or remove this \
                                 directory.",
                            )],
                        },
                    ));
                }
            }
        }
    }
    out.sort_by(|(a_dir, a), (b_dir, b)| {
        a.connector_id
            .cmp(&b.connector_id)
            .then_with(|| a_dir.cmp(b_dir))
    });
    Ok(out.into_iter().map(|(_, report)| report).collect())
}

fn harness_only_check(id: &str) -> Check {
    Check::new(
        "harness_manifest",
        Level::Warn,
        format!(
            "`{id}` uses the newer connector format, which has no install checks yet; \
             nothing was probed."
        ),
        "nothing is required for the capture path.",
    )
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

/// Is the daemon there.
///
/// With a client, one `health()` call answers it. Without one, `control.json` is the only
/// thing a second process can read to find out where a running daemon is listening — so a
/// missing file is the answer `goalcup doctor` came for, and the check names the path,
/// because "there is no daemon" without a path is not something a participant can act on.
fn daemon_check(client: Option<&dyn DaemonClient>, cfg: &Path) -> Check {
    let path = config::control_path(cfg);
    let unreachable = |e: String| {
        Check::new(
            "daemon",
            Level::Fail,
            // `{e}` and no prefix of this command's own: `ControlError::Unreachable` and
            // `read_control_file`'s "no daemon" both already say what happened.
            e,
            &format!(
                "start `goalcup-daemon`; it writes {} when it is up.",
                path.display()
            ),
        )
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
        Ok(h) => Check::pass(
            "daemon",
            format!(
                "daemon v{} is up (control API v{}, {} entr{} open, upload {}).",
                h.daemon_version,
                h.control_api_version,
                h.entries_open,
                if h.entries_open == 1 { "y" } else { "ies" },
                if h.upload_ok { "ok" } else { "failing" }
            ),
        ),
        Err(e) => unreachable(e.to_string()),
    }
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
