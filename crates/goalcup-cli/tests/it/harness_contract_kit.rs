//! Offline harness-contract conformance cases.
//!
//! Each immediate directory under `fixtures/harness-kit/` becomes a case when it contains
//! `harness.toml`, `stand-in.py` and `expect.toml`. The runner stages only the manifest and its
//! declared `files/**` seeds as a development connector; the Python stand-in remains a test
//! executable outside that connector. No case reaches a provider or reads the participant's
//! harness homes.
//!
//! Every case runs with an invented provider key ([`KIT_PROVIDER_KEY`]) in `goalcup start`'s own
//! environment, as a participant's shell might export one. Goal Cup Proxy never hands one to the
//! harness (D61), so the runner checks that no fragment of it is in the sealed trace, the run
//! directory or the launcher's forwarded stdout and stderr.

use crate::common::m2_vertical_support;
use crate::common::real_home_guard::{self, home_listing, with_real_home_guard};
use crate::common::support;

use goalcup_protocol::capture::{CaptureRecord, CaptureSource, is_artifact_hint};
use goalcup_protocol::harness::{IdentitySource, NativeForm, Origin};
use goalcup_verifier::capture_analysis::api::{Disposition, Observation};
use goalcup_verifier::capture_analysis::registry::DecoderRegistry;
use goalcup_verifier::capture_analysis::session::decode_records;
use m2_vertical_support::{RESERVE, Vertical, files_under};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

/// The provider key every case puts in `goalcup start`'s environment. Invented for the kit and
/// never a real key; no fragment of it is in a kit fixture, a case id or a shipped connector file.
const KIT_PROVIDER_KEY: &str = "Qv7Rk2Wm9Xp4Tn6Lj3Hb8Zc5Fd_0eYs";

#[derive(Debug, Clone)]
struct Expect {
    id: String,
    identity_source: String,
    identity_binding: String,
    native_source: String,
    native_path: String,
    native_form: String,
    native_g1: String,
    /// What the first native stream holds once the stand-in's first write is recorded and its
    /// second is not yet made. Left out, it is `native_g1`. An append-only case names it,
    /// because there `native_g1` is everything appended.
    native_gate: Option<String>,
    native_g2: Option<String>,
    native_decoy_path: Option<String>,
    native_decoy_text: Option<String>,
    wire_host: String,
    wire_path: String,
    wire_request: String,
    wire_response: String,
    capture_only_hint: String,
    seed_path: Option<String>,
    dirs: Vec<String>,
}

#[derive(Debug, Clone)]
struct Case {
    name: String,
    dir: PathBuf,
    expect: Expect,
}

struct Staged {
    world: Vertical,
    case: Case,
    connector_dir: PathBuf,
    marker: PathBuf,
    /// The file a gated stand-in waits for before it rewrites its snapshot. `Some` when the
    /// case's manifest names `@GO@`.
    go: Option<PathBuf>,
}

struct Completed {
    staged: Staged,
    entry: String,
    output: String,
}

fn kit_root() -> PathBuf {
    support::fixture("harness-kit")
}

fn cases() -> Vec<Case> {
    let root = kit_root();
    cases_at(&root)
}

fn cases_at(root: &Path) -> Vec<Case> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot enumerate {}: {error}", root.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("cannot enumerate {}: {error}", root.display()))
                .path()
        })
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    assert!(
        !dirs.is_empty(),
        "the harness-contract kit found no complete case under {}",
        root.display()
    );
    for dir in &dirs {
        for name in ["harness.toml", "stand-in.py", "expect.toml"] {
            assert!(
                dir.join(name).is_file(),
                "harness-contract kit case {} is missing {name}",
                dir.display()
            );
        }
    }
    dirs.into_iter()
        .map(|dir| {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(dir.join("expect.toml")).unwrap();
            Case {
                name,
                dir,
                expect: parse_expect(&text),
            }
        })
        .collect()
}

fn parse_expect(text: &str) -> Expect {
    let values: BTreeMap<String, String> = text
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once('=').expect("expect.toml key = value");
            let value = serde_json::from_str::<String>(value.trim())
                .unwrap_or_else(|_| panic!("expect.toml values are JSON-quoted strings: {line}"));
            Some((key.trim().to_string(), value))
        })
        .collect();
    let get = |key: &str| {
        values
            .get(key)
            .unwrap_or_else(|| panic!("expect.toml has no {key}"))
            .clone()
    };
    let optional = |key: &str| values.get(key).filter(|v| !v.is_empty()).cloned();
    Expect {
        id: get("id"),
        identity_source: get("identity_source"),
        identity_binding: get("identity_binding"),
        native_source: get("native_source"),
        native_path: get("native_path"),
        native_form: get("native_form"),
        native_g1: get("native_g1"),
        native_gate: optional("native_gate"),
        native_g2: optional("native_g2"),
        native_decoy_path: optional("native_decoy_path"),
        native_decoy_text: optional("native_decoy_text"),
        wire_host: get("wire_host"),
        wire_path: get("wire_path"),
        wire_request: get("wire_request"),
        wire_response: get("wire_response"),
        capture_only_hint: get("capture_only_hint"),
        seed_path: optional("seed_path"),
        dirs: optional("dirs")
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

fn toml_fragment(value: &Path) -> String {
    value
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

impl Staged {
    fn new(case: &Case) -> Self {
        let world = Vertical::new("plain", serde_json::json!({}));
        let endpoint: serde_json::Value =
            serde_json::from_slice(&std::fs::read(world.root.join("workload.json")).unwrap())
                .unwrap();
        let stand_ins = world.root.join("stand-ins");
        let markers = world.root.join("markers");
        std::fs::create_dir_all(&stand_ins).unwrap();
        std::fs::create_dir_all(&markers).unwrap();
        let stand_in = stand_ins.join(format!("{}.py", case.name));
        std::fs::copy(case.dir.join("stand-in.py"), &stand_in).unwrap();
        let marker = markers.join(&case.name);
        let go_path = markers.join(format!("{}.go", case.name));

        let template = std::fs::read_to_string(case.dir.join("harness.toml")).unwrap();
        let go = template.contains("@GO@").then(|| go_path.clone());
        let manifest = template
            .replace("@STAND_IN@", &toml_fragment(&stand_in))
            .replace("@MARKER@", &toml_fragment(&marker))
            .replace("@GO@", &toml_fragment(&go_path))
            .replace("@PORT@", &endpoint["port"].as_u64().unwrap().to_string())
            .replace("@TEST_ID@", endpoint["test_id"].as_str().unwrap());
        let connector_dir = world.connectors.join(&case.expect.id);
        std::fs::create_dir_all(&connector_dir).unwrap();
        std::fs::write(connector_dir.join("harness.toml"), manifest).unwrap();
        if case.dir.join("files").is_dir() {
            copy_tree(&case.dir.join("files"), &connector_dir.join("files"));
        }
        Self {
            world,
            case: case.clone(),
            connector_dir,
            marker,
            go,
        }
    }

    fn start(&self, dev: bool) -> Output {
        match self.go.as_deref() {
            Some(go) if dev => self.start_gated(go),
            _ => self.command(dev).output().unwrap(),
        }
    }

    /// Run a case whose stand-in waits for `go` between its first and its second snapshot.
    /// The runner writes `go` once the sealed-to-be chain holds the first snapshot, so the
    /// gap between the two writes is the daemon's own progress, never a sleep that a loaded
    /// host can outrun.
    fn start_gated(&self, go: &Path) -> Output {
        use std::process::Stdio;
        let expect = &self.case.expect;
        // Said at once, not after a silent 60-second stall: for an append-only store,
        // `native_g1` is both writes, so the first write alone never matches it.
        assert!(
            expect.native_form != "append_only" || expect.native_gate.is_some(),
            "kit case {}: an append_only case whose stand-in waits for `@GO@` needs `native_gate` \
             in expect.toml: what the native stream holds after the stand-in's first write",
            expect.id
        );
        let child = self
            .command(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Match the source prefix and the first generation: exactly one native stream can.
        let source_prefix = format!("{}:", self.case.expect.native_source);
        let first_bytes = self
            .case
            .expect
            .native_gate
            .as_deref()
            .unwrap_or(&self.case.expect.native_g1)
            .as_bytes();
        // The releaser stops when the child is gone; the stand-in's own wait is bounded too.
        let finished = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let until = std::time::Instant::now() + Duration::from_secs(60);
                while std::time::Instant::now() < until && !finished.load(Ordering::SeqCst) {
                    if let Ok(Some(entry)) = goalcup_cli::entries::latest_entry(&self.world.cfg) {
                        if !self
                            .world
                            .trace(&entry.entry_id)
                            .join("chain.jsonl")
                            .is_file()
                        {
                            std::thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                        let chain = self.world.chain(&entry.entry_id);
                        let held = chain.streams().values().any(|stream| {
                            stream.source == CaptureSource::Native
                                && stream.meta.label.starts_with(&source_prefix)
                                && stream.meta.label.ends_with("#g1")
                                && stream.data == first_bytes
                        });
                        if held {
                            std::fs::write(go, b"").unwrap();
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            let output = child.wait_with_output().unwrap();
            finished.store(true, Ordering::SeqCst);
            output
        })
    }

    fn command(&self, dev: bool) -> Command {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.world.root.join("home"))
            .env(goalcup_cli::config::CONFIG_DIR_ENV, &self.world.cfg)
            .env(
                goalcup_cli::config::CONNECTORS_DIR_ENV,
                &self.world.connectors,
            )
            .env(
                "GOALCUP_TEST_UPSTREAM_ROOTS",
                self.world.root.join("upstream-ca.pem"),
            )
            .args(["start", "--harness", &self.case.expect.id]);
        // A provider key the participant's shell exported: the launcher hands it to no one.
        for name in ["DEEPSEEK_API_KEY", "AMP_API_KEY", "OPENAI_API_KEY"] {
            command.env(name, KIT_PROVIDER_KEY);
        }
        if dev {
            command.arg("--dev");
        }
        // Run from the world's run folder, as a participant runs it from theirs; the test build
        // has no terminal to hand the chat-mode stand-in (D57).
        command
            .current_dir(&self.world.runs)
            .env("GOALCUP_TEST_CHAT_WITHOUT_TERMINAL", "1")
            .env("GOALCUP_TEST_DISK", format!("{RESERVE},0,50"))
            .process_group(0);
        command
    }

    fn complete(self) -> Completed {
        let output = self.start(true);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{}: {text}", self.case.name);
        let entry = self.world.entry_id();
        Completed {
            staged: self,
            entry,
            output: text,
        }
    }
}

fn assert_completed(run: &Completed) {
    let expect = &run.staged.case.expect;
    let trace = run.staged.world.trace(&run.entry);
    assert!(trace.join("seal.json").is_file(), "{}", run.output);
    let chain = run.staged.world.chain(&run.entry);
    assert!(chain.lines.iter().all(|line| line.protocol_version == 5));
    assert_eq!(
        chain.count(|record| matches!(record, CaptureRecord::LocalFinish { .. })),
        1
    );
    let launch = chain.records().find_map(|record| match record {
        CaptureRecord::Launch {
            profile_id,
            connector_dir_sha256,
            ..
        } => Some((profile_id, connector_dir_sha256)),
        _ => None,
    });
    let (profile_id, connector_digest) = launch.expect("one launch record");
    assert_eq!(profile_id, &expect.id);
    let staged_digest =
        goalcup_protocol::connector_dir::connector_dir_sha256(&run.staged.connector_dir)
            .expect("the staged connector directory hashes");
    assert_eq!(
        connector_digest.as_deref(),
        Some(staged_digest.as_str()),
        "Launch did not bind the directory the runner staged"
    );

    let entry_record = goalcup_cli::entries::read_entry(&run.staged.world.cfg, &run.entry)
        .expect("goalcup start persisted its local entry record");
    let signed = run.staged.world.status(&run.entry);
    signed
        .verify_with(&entry_record.run_public_key)
        .expect("status verifies with the key persisted before the run");
    assert_eq!(signed.payload.entry_id, run.entry);
    assert_eq!(signed.payload.connector_id, expect.id);
    // Chat mode names no session (D57): the native path is the case's own.
    assert_eq!(
        (
            expect.identity_source.as_str(),
            expect.identity_binding.as_str()
        ),
        ("none", "none"),
        "{}",
        run.staged.case.name
    );
    let native_path = expect.native_path.clone();

    let streams = chain.streams();
    let requests: Vec<_> = streams
        .values()
        .filter(|stream| {
            // Header streams (http-head-v1) share the exchange's host and path; the body
            // stream is the request this check means.
            stream.source == CaptureSource::WireRequest
                && !stream
                    .meta
                    .protocol_hint
                    .as_deref()
                    .is_some_and(is_artifact_hint)
                && stream.meta.host.as_deref() == Some(&expect.wire_host)
                && stream.meta.path.as_deref() == Some(&expect.wire_path)
        })
        .collect();
    assert_eq!(requests.len(), 1, "expected one matching request stream");
    assert!(requests[0].clean());
    assert!(!requests[0].segments.is_empty());
    assert!(!expect.wire_request.is_empty());
    assert_eq!(requests[0].meta.method.as_deref(), Some("POST"));
    assert_eq!(requests[0].data, expect.wire_request.as_bytes());
    let responses: Vec<_> = streams
        .values()
        .filter(|stream| {
            stream.source == CaptureSource::WireResponse
                && !stream
                    .meta
                    .protocol_hint
                    .as_deref()
                    .is_some_and(is_artifact_hint)
                && stream.meta.host.as_deref() == Some(&expect.wire_host)
                && stream.meta.path.as_deref() == Some(&expect.wire_path)
        })
        .collect();
    assert_eq!(responses.len(), 1, "expected one matching response stream");
    assert!(responses[0].clean());
    assert!(!responses[0].segments.is_empty());
    assert!(!expect.wire_response.is_empty());
    assert_eq!(responses[0].meta.status, Some(200));
    assert_eq!(responses[0].data, expect.wire_response.as_bytes());

    let mut native: Vec<_> = streams
        .values()
        .filter(|stream| stream.source == CaptureSource::Native)
        .collect();
    native.sort_by(|left, right| left.meta.label.cmp(&right.meta.label));
    let mut expected_native = vec![expect.native_g1.as_str()];
    if let Some(g2) = expect.native_g2.as_deref() {
        expected_native.push(g2);
    }
    assert_eq!(
        native.len(),
        expected_native.len(),
        "{} captured the wrong native stream shape: {:?}",
        run.staged.case.name,
        native
            .iter()
            .map(|stream| &stream.meta.label)
            .collect::<Vec<_>>()
    );
    for (index, (stream, expected_bytes)) in native.iter().zip(expected_native).enumerate() {
        assert_eq!(
            stream.meta.label,
            format!("{}:{}#g{}", expect.native_source, native_path, index + 1)
        );
        assert!(stream.clean(), "{:?}", stream.meta.label);
        assert!(!stream.segments.is_empty(), "{:?}", stream.meta.label);
        assert_eq!(stream.data, expected_bytes.as_bytes());
    }
    if let (Some(decoy_path), Some(decoy_text)) = (
        expect.native_decoy_path.as_deref(),
        expect.native_decoy_text.as_deref(),
    ) {
        let run_dir = run.staged.world.run_dir().expect("one run directory");
        let decoy = files_under(&run_dir)
            .into_iter()
            .find(|(path, _)| path.ends_with(Path::new(decoy_path)))
            .unwrap_or_else(|| panic!("the negative-control file {decoy_path:?} was not written"));
        assert_eq!(decoy.1, decoy_text.as_bytes());
        assert!(
            native
                .iter()
                .all(|stream| !stream.meta.label.contains(decoy_path)),
            "the wrong-id native negative control was captured"
        );
    }

    let loaded = goalcup_protocol::harness::load_dir(&run.staged.connector_dir, Origin::Dev)
        .expect("the staged development connector");
    let native_spec = &loaded.manifest.capture.native[0];
    let expected_form = match expect.native_form.as_str() {
        "append_only" => NativeForm::AppendOnly,
        "snapshot" => NativeForm::Snapshot,
        other => panic!("unknown expected native form {other}"),
    };
    assert_eq!(native_spec.form, expected_form);
    assert_eq!(
        loaded.manifest.capture.identity.source,
        IdentitySource::None
    );

    let records: Vec<CaptureRecord> = chain.records().cloned().collect();
    let (decoded, _) = decode_records(
        records.into_iter().map(Ok),
        &DecoderRegistry::builtin(),
        |_: &'static str, _: Observation| Ok(Disposition::Kept),
    )
    .unwrap();
    assert!(
        decoded
            .undecoded
            .iter()
            .any(|group| group.protocol_hint.as_deref() == Some(&expect.capture_only_hint)),
        "{} did not report hint {:?} as capture-only: {:?}",
        run.staged.case.name,
        expect.capture_only_hint,
        decoded.undecoded
    );

    let (exit, integrity) = run.staged.world.verify(&run.entry);
    assert_eq!(exit, 0, "{integrity}");
    assert_eq!(integrity["integrity"], "valid");
    assert_eq!(integrity["sealed"], true);
    assert_eq!(integrity["terminal"]["kind"], "local_finish");

    let (exit, research) = run.staged.world.research(&run.entry);
    assert_eq!(exit, 0, "{research}");
    assert_eq!(research["capture"]["integrity"], "valid");
    assert_eq!(research["capture"]["sealed"], true);
    assert_eq!(research["delivery"]["state"], "local_only");
    assert_eq!(research["analysis"]["state"], "unavailable");
    assert_no_provider_key_kept(run);
}

/// No fragment of [`KIT_PROVIDER_KEY`], which was in `goalcup start`'s environment, is in the
/// forwarded output, the sealed trace or the run directory (D61).
fn assert_no_provider_key_kept(run: &Completed) {
    let case = &run.staged.case.name;
    let n = KIT_PROVIDER_KEY.len();
    let forms = [
        KIT_PROVIDER_KEY,
        &KIT_PROVIDER_KEY[..8],
        &KIT_PROVIDER_KEY[n - 8..],
    ];
    for form in forms {
        assert!(
            !run.output.contains(form),
            "{case}: the forwarded stdout or stderr holds a fragment of the provider key"
        );
    }
    let trace = run.staged.world.trace(&run.entry);
    let run_dir = run.staged.world.run_dir().expect("one run directory");
    assert!(
        trace.join("chain.jsonl").is_file() && trace.join("seal.json").is_file(),
        "{case}: the sealed trace is missing"
    );
    for root in [&trace, &run_dir] {
        for (path, bytes) in files_under(root) {
            for form in forms {
                assert!(
                    !bytes.windows(form.len()).any(|w| w == form.as_bytes()),
                    "{case}: {} holds a fragment of the provider key",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn home_listing_detects_nonvolatile_in_place_mutations_without_reading_contents() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join(".codex");
    let claude = home.path().join(".claude");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    let codex_config = codex.join("config.toml");
    let claude_settings = claude.join("settings.json");
    std::fs::write(&codex_config, b"before").unwrap();
    std::fs::write(&claude_settings, b"before").unwrap();
    let fixed_past = UNIX_EPOCH + Duration::from_secs(1);
    for path in [&codex_config, &claude_settings] {
        let times = std::fs::FileTimes::new()
            .set_accessed(fixed_past)
            .set_modified(fixed_past);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(times)
            .unwrap();
    }

    let before = home_listing(home.path());
    assert!(before.contains("config.toml"));
    assert!(before.contains("settings.json"));

    std::fs::write(&codex_config, b"after!").unwrap();
    let after_codex = home_listing(home.path());
    assert_ne!(
        after_codex, before,
        "Codex config mtime mutation was invisible"
    );

    let before_claude = home_listing(home.path());
    std::fs::write(&claude_settings, b"after!").unwrap();
    assert_ne!(
        home_listing(home.path()),
        before_claude,
        "Claude settings mtime mutation was invisible"
    );
}

#[test]
fn home_listing_watches_app_wide_config_and_credentials_not_session_churn() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join(".codex");
    let claude = home.path().join(".claude");
    std::fs::create_dir_all(codex.join("sessions")).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(codex.join("config.toml"), b"before").unwrap();
    std::fs::write(codex.join("session_index.jsonl"), b"").unwrap();
    let before = home_listing(home.path());

    // What the owner's live apps do while a test runs: session files written, the desktop
    // app's state and Claude Code's ~/.claude.json rewritten, backups and locks created.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(codex.join("session_index.jsonl"), b"{\"id\":1}\n").unwrap();
    std::fs::write(codex.join("sessions").join("rollout.jsonl"), b"{}").unwrap();
    std::fs::write(codex.join(".codex-global-state.json"), b"{}").unwrap();
    std::fs::create_dir_all(codex.join("thread-writer-locks")).unwrap();
    std::fs::create_dir_all(claude.join("backups")).unwrap();
    std::fs::write(home.path().join(".claude.json"), b"{\"rewritten\":true}").unwrap();
    assert_eq!(
        home_listing(home.path()),
        before,
        "session and app-state churn must not trip the guard"
    );

    // What an escaped harness would do: each one trips it, in every app-wide home a shipped
    // connector names that the owner's apps do not churn.
    let mut trips: Vec<(String, Box<dyn Fn()>)> = vec![
        (
            "~/.codex/config.toml changed".into(),
            Box::new(|| std::fs::write(codex.join("config.toml"), b"after!").unwrap()),
        ),
        (
            "~/.codex/auth.json created".into(),
            Box::new(|| std::fs::write(codex.join("auth.json"), b"{}").unwrap()),
        ),
        (
            "~/.claude/settings.json created".into(),
            Box::new(|| std::fs::write(claude.join("settings.json"), b"{}").unwrap()),
        ),
    ];
    for walked in real_home_guard::walked_homes() {
        let root = home.path().join(&walked);
        trips.push((
            format!("a file created under ~/{walked}"),
            Box::new(move || {
                std::fs::create_dir_all(root.join("sessions")).unwrap();
                std::fs::write(root.join("sessions/s.json"), b"{}").unwrap();
            }),
        ));
    }
    for (what, write) in trips {
        let before = home_listing(home.path());
        write();
        assert_ne!(home_listing(home.path()), before, "{what} was invisible");
    }
}

#[test]
fn every_immediate_kit_directory_must_be_complete() {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &kit_root().join("controlled-workload"),
        &root.path().join("complete"),
    );
    let incomplete = root.path().join("missing-expectation");
    std::fs::create_dir(&incomplete).unwrap();
    std::fs::write(incomplete.join("harness.toml"), "schema_version = 1\n").unwrap();
    std::fs::write(incomplete.join("stand-in.py"), "pass\n").unwrap();

    let result = std::panic::catch_unwind(|| cases_at(root.path()));
    assert!(result.is_err(), "an incomplete case was silently skipped");
}

/// Each kit case runs to completion once. Without `--dev` it is refused before its stand-in
/// spawns; with it, the same staged case spawns, seals and matches its expectations. The
/// `synthetic-per-run` run also shows the home paths it declares created with private
/// modes.
#[test]
fn every_kit_case_seals_and_matches_expectations() {
    with_real_home_guard(|| {
        let mut modes_checked = false;
        for case in cases() {
            let staged = Staged::new(&case);
            let refused = staged.start(false);
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&refused.stdout),
                String::from_utf8_lossy(&refused.stderr)
            );
            assert!(!refused.status.success(), "{}: {text}", case.name);
            assert!(text.contains("--dev"), "{}: {text}", case.name);
            assert!(
                !staged.marker.exists(),
                "{} spawned without --dev",
                case.name
            );
            let run = staged.complete();
            assert!(run.staged.marker.is_file(), "{}: {}", case.name, run.output);
            if case.name == "synthetic-per-run" {
                assert_private_home_modes(&run, &case);
                modes_checked = true;
            }
            assert_completed(&run);
        }
        assert!(modes_checked, "the synthetic-per-run case ran");
    });
}

#[test]
fn a_denied_argv_is_refused_before_spawn() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "synthetic-per-run")
        .expect("the per-run case");
    let staged = Staged::new(&case);
    let path = staged.connector_dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replacen("args = [", "args = [\"--pro\", ", 1);
    assert_ne!(text, edited);
    std::fs::write(path, edited).unwrap();
    let refused = staged.start(true);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(!refused.status.success(), "{text}");
    assert!(text.contains("option the harness acts on"), "{text}");
    assert!(
        !staged.marker.exists(),
        "the denied argv reached the stand-in"
    );
}

/// Nested, duplicate and implicit home paths are created with private modes: the nested seed
/// is 0400 with its bytes, every implicit parent of it and every declared directory 0700.
fn assert_private_home_modes(run: &Completed, case: &Case) {
    let run_dir = run.staged.world.run_dir().expect("one run directory");
    let home = run_dir.join("stdout-home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let seed = case.expect.seed_path.as_ref().expect("the nested seed");
        assert_eq!(
            std::fs::metadata(home.join(seed))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        assert_eq!(
            std::fs::read(home.join(seed)).unwrap(),
            b"{\"network\":\"offline\",\"updates\":false}\n"
        );
        let mut parent = Path::new(seed).parent();
        while let Some(relative) = parent.filter(|path| !path.as_os_str().is_empty()) {
            assert_eq!(
                std::fs::metadata(home.join(relative))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "implicit seed parent {}",
                relative.display()
            );
            parent = relative.parent();
        }
        for dir in &case.expect.dirs {
            assert_eq!(
                std::fs::metadata(home.join(dir))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "{dir}"
            );
        }
    }
}
