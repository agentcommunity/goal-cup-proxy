//! `goalcup start` of a per-run connector (a per-run home made from its seed files), through the
//! real daemon in process, the real proxy and sender inside `start_with`, and a fake harness
//! (`fixtures/m2-fake-per-run.sh`) that writes its own session snapshot and exits. The
//! connector is the fixture `tests/fixtures/connectors/chat-per-run`, never a shipped one.
//!
//! The one provider key here is the synthetic one in `secret_support`. Nothing reads a credential
//! file or resolves a real `HOME`; every directory is a temporary one.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS). Every test holds the
//! process-table lock: each runs a daemon and spawns a harness.

use crate::common::m2_cli_support;
use crate::common::secret_support;
use crate::common::support;

use crate::common::table_lock;

use goalcup_cli::CliError;
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, GapReason};
use m2_cli_support::{Hooked, Rig};
use secret_support::KEY;
use std::path::PathBuf;

const RESERVE: u64 = 4 * 1024 * 1024;

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("the start thread panicked")
}

/// Run `goalcup start` of the per-run connector to its end.
fn run(
    rig: &Rig,
    client: &Hooked<'_>,
) -> (
    Result<goalcup_cli::commands::start::StartOutcome, CliError>,
    String,
) {
    let goal = rig.run_folder();
    std::thread::scope(|s| joined(rig.start(s, client, rig.per_run_options(&goal, RESERVE), None)))
}

fn run_dir(rig: &Rig) -> PathBuf {
    let dirs = rig.run_dirs();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    dirs[0].clone()
}

fn gaps(records: &[CaptureRecord], reason: GapReason) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { stream: None, reason: g, .. } if *g == reason))
        .count()
}

fn native_labels(records: &[CaptureRecord]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                source: CaptureSource::Native,
                metadata,
                ..
            } => Some(metadata.label.clone()),
            _ => None,
        })
        .collect()
}

fn session_id(rig: &Rig) -> String {
    rig.harness_file("session-id")
        .expect("the fake wrote its session id")
        .trim()
        .to_string()
}

/// End to end: the launcher stops the recording proxy first, the finish request carries no scope
/// (chat mode names no session, D57), and the run ends with its own session snapshot captured
/// and no gap. The child saw the private homes and the loopback
/// proxy. A provider key exported in the launcher's own environment never reaches it (D61), and
/// is in nothing this machine kept.
#[test]
fn a_per_run_run_ends_with_its_snapshot_captured() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::per_run();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    rig.with_env("OPENAI_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        outcome.exit_status.expect("a harness ran").success(),
        "{out}"
    );

    let entry = client.created().entry_id;
    let id = session_id(&rig);
    let seals = client.book().seals.clone();
    assert_eq!(seals.len(), 1);
    assert_eq!(seals[0].native_scope, None);
    assert!(seals[0].remaining_finish_ms.is_some());

    let (report, records) = rig.read_trace(&entry);
    assert_eq!(native_labels(&records).len(), 1, "{records:#?}");
    assert!(
        native_labels(&records)[0].contains(&format!("session_{id}.json")),
        "{:?}",
        native_labels(&records)
    );
    assert_eq!(gaps(&records, GapReason::SourceChanged), 0, "{records:#?}");
    assert!(!report.coverage_incomplete, "{:?}", report.findings);

    // The run directory's shape: no key-scan verdict, since no run holds a key to scan for.
    assert!(out.contains("harness exited: exit code 0."), "{out}");
    let run = run_dir(&rig);
    assert!(!run.join("key-scan.txt").exists());
    assert!(
        run.join(format!("perrun/sessions/session_{id}.json"))
            .is_file()
    );
    let seeds = support::fixture_connectors_dir().join("chat-per-run/files");
    for name in ["config.yaml", "auth.json"] {
        assert_eq!(
            std::fs::read(run.join("perrun").join(name)).unwrap(),
            std::fs::read(seeds.join(name)).unwrap(),
            "{name}"
        );
    }

    // What the child was started with.
    let argv = rig.harness_file("argv").unwrap();
    assert_eq!(argv, "chat\n-Q\n", "the args alone: no goal (D57)");
    let env = rig.harness_file("env").unwrap();
    let has = |line: &str| env.lines().any(|l| l == line);
    assert!(
        !env.contains(KEY) && !env.contains("_API_KEY="),
        "the child was handed a provider key: {env}"
    );
    assert!(has(&format!(
        "PERRUN_HOME={}",
        run.join("perrun").display()
    )));
    assert!(has(&format!("HOME={}", run.join("home").display())));
    assert!(has("TOOL_HOME_MODE=profile"));
    assert!(
        env.lines()
            .any(|l| l.starts_with("HTTPS_PROXY=http://goalcup:") && l.contains("@127.0.0.1:")),
        "{env}"
    );
    // `goalcup start` hands the harness's own CA variable the recording proxy's CA alone (D41), in
    // replace mode too: one certificate, never the roots file a shared row gets.
    assert!(env.lines().any(|l| l.starts_with("PERRUN_CA_BUNDLE=")));
    let handed = rig
        .harness_file("ca.pem")
        .expect("the fake copied its CA file");
    assert_eq!(
        goalcup_cli::ca_bundle::certificates(handed.as_bytes()).len(),
        1,
        "the own CA variable names a file of exactly one certificate"
    );
    assert!(
        !env.lines()
            .any(|l| l.starts_with("OPENAI") || l.contains("BASE_URL")),
        "{env}"
    );

    // The key is in nothing this machine kept, and nothing the CLI printed.
    let fragments = [KEY, &KEY[..8], &KEY[KEY.len() - 8..]];
    for (what, text) in [
        ("the CLI's own output", out.clone()),
        ("the entry records", entries(&rig)),
        ("the trace records", format!("{records:?}")),
    ] {
        for fragment in fragments {
            assert!(!text.contains(fragment), "{what} shows the key");
        }
    }
    // The order the design fixes: the launch is recorded, then the finish (proxy stopped first).
    let names = client.names();
    let at = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert!(
        at("record_capture_launch") < at("seal_capture"),
        "{names:?}"
    );
}

/// Every `entries/*.json` this machine wrote, as one string.
fn entries(rig: &Rig) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(rig.cfg.join("entries"))
        .into_iter()
        .flatten()
        .flatten()
    {
        text.push_str(&std::fs::read_to_string(entry.path()).unwrap());
    }
    text
}

/// On the wire: a run folder inside a Git work tree is refused before the daemon is asked for
/// anything.
#[test]
fn a_run_folder_inside_a_git_work_tree_refuses_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::per_run();
    let goal = rig.run_folder();
    std::fs::create_dir(goal.join(".git")).unwrap();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, _out) = std::thread::scope(|s| {
        joined(rig.start(s, &client, rig.per_run_options(&goal, RESERVE), None))
    });
    let error = result.expect_err("a work tree is refused");
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(error.to_string().contains("Git work tree"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    // The run folder holds the `.git` this test made, and no run directory.
    assert!(
        rig.run_dirs().iter().all(|d| !d
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("goalcup-"))),
        "{:?}",
        rig.run_dirs()
    );
    assert!(!rig.harness_started());
}

/// A stdout that goes away at the seal report: every write before the report succeeds, and the
/// first write of the report's first line, and every one after it, fails with a broken pipe.
struct StdoutGoneAtTheSealReport(std::sync::Arc<std::sync::Mutex<String>>);

impl std::io::Write for StdoutGoneAtTheSealReport {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut seen = self.0.lock().unwrap();
        if seen.contains("finished locally")
            || String::from_utf8_lossy(bytes).contains("finished locally")
        {
            seen.push_str("<broken>");
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        seen.push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A seal that has committed stays committed when this command's own stdout is gone (a pager
/// quit, a closed pipe): the run is reported as it ended and its trace is sealed (CONTRACTS: this
/// command's own output failing ends nothing).
#[test]
fn a_committed_seal_stays_committed_when_stdout_fails() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::per_run();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let goal = rig.run_folder();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let result = std::thread::scope(|s| {
        joined(rig.start_writing(
            s,
            &client,
            rig.per_run_options(&goal, RESERVE),
            StdoutGoneAtTheSealReport(seen.clone()),
        ))
    });
    let shown = seen.lock().unwrap().clone();
    assert!(
        shown.contains("<broken>"),
        "the writer was never cut: {shown}"
    );
    let outcome =
        result.unwrap_or_else(|e| panic!("a sealed run exits as the harness did: {e}\n{shown}"));
    assert!(outcome.exit_status.expect("a harness ran").success());

    let entry = client.created().entry_id;
    let (report, _records) = rig.read_trace(&entry);
    assert!(!report.coverage_incomplete, "{:?}", report.findings);
    assert!(
        rig.trace(&entry).join("seal.json").is_file(),
        "the committed seal is on disk"
    );
}

/// A run folder that a sync client uploads is refused before anything is created.
#[test]
fn a_run_folder_in_a_synced_folder_refuses_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::per_run();
    let dir = rig.root.join("Dropbox/runs");
    std::fs::create_dir_all(&dir).unwrap();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, _out) = std::thread::scope(|s| {
        joined(rig.start(s, &client, rig.per_run_options(&dir, RESERVE), None))
    });
    let error = result.expect_err("a synced folder is refused");
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(error.to_string().contains("synced"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(!rig.harness_started());
}

/// End to end (D60): the run's own proxy credential, which the harness holds in `HTTPS_PROXY`,
/// reaches the daemon on the capture create and is removed by exact value from what it records.
/// The fake writes it into its session file raw, in GNU `base64` wrapped at 76 columns, and with
/// that base64 inside a doubly escaped JSON tool result (as Hermes records one): the snapshot is
/// captured, and neither the trace's text nor its decoded streams hold any form of it.
#[test]
fn the_proxy_credential_a_harness_records_is_redacted_by_exact_value() {
    use base64::Engine as _;
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::per_run();
    // The snapshot's content, built by the shell from the harness's own `HTTPS_PROXY`.
    rig.tell(
        "steps.sh",
        r#"b64=$(printf 'PAD=0123456789012\nHTTPS_PROXY=%s\n' "$HTTPS_PROXY" | base64 | tr -d '\n' | fold -w 76 | awk '{printf "%s\\\\n", $0}')
printf '{"session_id":"%s","messages":[{"role":"tool","content":"{\\"output\\": \\"%s\\"}"},{"role":"tool","content":"HTTPS_PROXY=%s"}]}' "$sid" "$b64" "$HTTPS_PROXY" > "$snap.tmp" && mv "$snap.tmp" "$snap"
: > "$out/no-snapshot"
"#,
    );
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (_result, out) = run(&rig, &client);
    let env = rig
        .harness_file("env")
        .expect("the fake recorded its environment");
    let proxy = env
        .lines()
        .find_map(|l| l.strip_prefix("HTTPS_PROXY=http://goalcup:"))
        .and_then(|rest| rest.split('@').next())
        .expect("the proxy credential")
        .to_string();
    assert_eq!(proxy.len(), 32, "{out}");
    let entry = client.created().entry_id;
    let (_, records) = rig.read_trace(&entry);
    let mut decoded = Vec::new();
    for record in &records {
        if let CaptureRecord::Segment { data_b64, .. } = record {
            decoded.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
        }
    }
    let text = std::fs::read(rig.trace(&entry).join("chain.jsonl")).unwrap();
    let forms = goalcup_protocol::run_secret::WrappedForms::of(proxy.as_bytes()).unwrap();
    let decoded_text = String::from_utf8_lossy(&decoded);
    assert!(
        decoded_text.contains("output"),
        "the snapshot was captured: {decoded_text}"
    );
    assert!(!forms.contains(&decoded), "a form survived: {decoded_text}");
    assert!(
        !forms.contains(&text),
        "a form survived in the chain's text"
    );
    assert!(
        decoded_text.matches("<redacted:run_secret>").count() >= 2,
        "{decoded_text}"
    );
}
