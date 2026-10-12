//! Offline harness-contract conformance cases.
//!
//! Each immediate directory under `fixtures/harness-kit/` becomes a case when it contains
//! `harness.toml`, `stand-in.py` and `expect.toml`. The runner stages only the manifest and its
//! declared `files/**` seeds as a development connector; the Python stand-in remains a test
//! executable outside that connector. No case reaches a provider or reads the participant's
//! harness homes.
//!
//! A case whose `expect.toml` names `credential_env` is a keyed case: the runner puts the
//! invented [`KIT_CREDENTIAL`] (shared through `support`) in that variable of `goalcup start`'s own environment, so the
//! launcher's child-secret path hands it to the stand-in. The runner then checks that the
//! stand-in received exactly that value, and that no fragment of it is in the sealed trace, the
//! run directory or the launcher's forwarded stdout and stderr.

use crate::common::m2_vertical_support;
use crate::common::support;

use goalcup_protocol::capture::{CaptureRecord, CaptureSource};
use goalcup_protocol::harness::{Binding, IdentitySource, NativeForm, Origin};
use goalcup_verifier::capture_analysis::api::{Disposition, Observation};
use goalcup_verifier::capture_analysis::registry::DecoderRegistry;
use goalcup_verifier::capture_analysis::session::decode_records;
use m2_vertical_support::{RESERVE, Vertical, files_under};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, UNIX_EPOCH};
use support::{KIT_CREDENTIAL, KIT_CREDENTIAL_MARKER_PREFIX};

#[derive(Debug, Clone)]
struct Expect {
    id: String,
    identity_source: String,
    identity_binding: String,
    identity_id: Option<String>,
    native_source: String,
    native_path: String,
    native_form: String,
    native_g1: String,
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
    /// The admitted credential variable a keyed case is handed [`KIT_CREDENTIAL`] in.
    credential_env: Option<String>,
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
        identity_id: optional("identity_id"),
        native_source: get("native_source"),
        native_path: get("native_path"),
        native_form: get("native_form"),
        native_g1: get("native_g1"),
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
        credential_env: optional("credential_env"),
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

fn modified_stamp(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| format!("{}.{:09}", duration.as_secs(), duration.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// The app-wide files an escaped harness would write: its config and its credentials. These
/// are what the rule "never write into ~/.claude or ~/.codex" protects, so these are what the
/// guard watches. Session files, caches, backups and the desktop apps' own state churn
/// continuously while the owner's Claude and Codex apps run, and they are not watched.
/// `~/.claude.json` is left out too: Claude Code rewrites it every few minutes, so watching it
/// would fail the kit at random.
const WATCHED_APP_WIDE_FILES: [&str; 3] = [
    ".codex/config.toml",
    ".codex/auth.json",
    ".claude/settings.json",
];

/// Hermes is not an app the owner runs, so anything created under `~/.hermes` is a harness
/// write. Names are walked this many levels down.
const HERMES_WALK_DEPTH: usize = 4;

/// One line per watched path: absent, or its type, mode, size, mtime and symlink target. File
/// contents are never read.
fn append_real_home_node(path: &Path, relative: &Path, listing: &mut Vec<String>) -> bool {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            listing.push(format!("{relative:?}\tabsent"));
            return false;
        }
        Err(error) => {
            listing.push(format!("{relative:?}\terror={:?}", error.kind()));
            return false;
        }
    };
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        "symlink"
    } else if file_type.is_dir() {
        "directory"
    } else if file_type.is_file() {
        "file"
    } else {
        "other"
    };
    // A directory's own size and mtime move with its entries, which are listed one by one.
    let identity = if file_type.is_dir() {
        format!("mode={:o}", metadata.mode() & 0o7777)
    } else {
        format!(
            "mode={:o};size={};mtime={}",
            metadata.mode() & 0o7777,
            metadata.len(),
            modified_stamp(&metadata),
        )
    };
    let symlink_target = if file_type.is_symlink() {
        std::fs::read_link(path)
            .map(|target| format!(";target={target:?}"))
            .unwrap_or_else(|error| format!(";target-error={:?}", error.kind()))
    } else {
        String::new()
    };
    listing.push(format!(
        "{relative:?}\ttype={kind};{identity}{symlink_target}"
    ));
    file_type.is_dir()
}

/// Every name under `~/.hermes`, so any creation there shows.
fn append_hermes_tree(path: &Path, relative: &Path, listing: &mut Vec<String>, depth: usize) {
    if !append_real_home_node(path, relative, listing) || depth == 0 {
        return;
    }
    let mut children = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>(),
        Err(error) => {
            listing.push(format!("{relative:?}\tread-dir-error={:?}", error.kind()));
            return;
        }
    };
    children.sort();
    for name in children {
        append_hermes_tree(&path.join(&name), &relative.join(&name), listing, depth - 1);
    }
}

/// The contract kit launches stand-ins, so bracket every launching test with the same
/// real-HOME boundary that guards the H-17 refusal regression. The snapshot records the
/// app-wide config and credential files and every name under `~/.hermes`, by file type,
/// symlink target and metadata only; it never reads file contents. The test body is caught so
/// this comparison runs before any unrelated failure is resumed.
fn home_listing(home: &Path) -> String {
    let mut listing = Vec::new();
    for relative in WATCHED_APP_WIDE_FILES {
        append_real_home_node(&home.join(relative), Path::new(relative), &mut listing);
    }
    append_hermes_tree(
        &home.join(".hermes"),
        Path::new(".hermes"),
        &mut listing,
        HERMES_WALK_DEPTH,
    );
    listing.join("\n")
}

fn real_home_listing() -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return "no HOME".to_string();
    };
    home_listing(&home)
}

fn with_real_home_guard(test: impl FnOnce()) {
    let before = real_home_listing();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));

    assert_eq!(
        real_home_listing(),
        before,
        "the real HOME's app-wide harness directories changed while the contract kit ran"
    );
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
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

        let python = Path::new("/usr/bin/python3");
        let version = Command::new(python)
            .env_clear()
            .args([
                "-I",
                "-S",
                "-c",
                "import os, platform, sys; assert 'HOME' not in os.environ; assert 'PYTHONPATH' not in os.environ; assert sys.flags.isolated and sys.flags.no_site; print(platform.python_version())",
            ])
            .output()
            .unwrap();
        assert!(version.status.success());
        let sha256 = goalcup_daemon::workspace::sha256_hex(&std::fs::read(python).unwrap());
        let manifest = std::fs::read_to_string(case.dir.join("harness.toml"))
            .unwrap()
            .replace("@PYTHON3_SHA256@", &sha256)
            .replace(
                "@PYTHON3_VERSION@",
                String::from_utf8_lossy(&version.stdout).trim(),
            )
            .replace("@STAND_IN@", &toml_fragment(&stand_in))
            .replace("@MARKER@", &toml_fragment(&marker))
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
        }
    }

    fn start(&self, dev: bool) -> Output {
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
        // The launcher reads the key from its own environment and hands it to the child alone.
        if let Some(name) = self.case.expect.credential_env.as_deref() {
            command.env(name, KIT_CREDENTIAL);
        }
        if dev {
            command.arg("--dev");
        }
        command
            .arg("--goal-file")
            .arg(&self.world.goal)
            .args(["--capture-reserve-bytes", &RESERVE.to_string()])
            .process_group(0)
            .output()
            .unwrap()
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

fn expected_identity(expect: &Expect) -> (IdentitySource, Option<Binding>) {
    let source = match expect.identity_source.as_str() {
        "none" => IdentitySource::None,
        "preassigned" => IdentitySource::Preassigned,
        "stdout_json" => IdentitySource::StdoutJson,
        "stderr_line" => IdentitySource::StderrLine,
        other => panic!("unsupported identity source {other:?}"),
    };
    let binding = match expect.identity_binding.as_str() {
        "none" => None,
        "at_launch" => Some(Binding::AtLaunch),
        "deferred" => Some(Binding::Deferred),
        other => panic!("unsupported identity binding {other:?}"),
    };
    (source, binding)
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
    let identity_id = match expect.identity_source.as_str() {
        "preassigned" => {
            assert!(
                !entry_record.session_id.is_empty(),
                "a preassigned run persisted no session id"
            );
            Some(entry_record.session_id.as_str())
        }
        "stdout_json" | "stderr_line" => {
            let id = expect.identity_id.as_deref().expect("the emitted identity");
            assert!(
                run.output.contains(id),
                "{} did not emit its expected identity {id:?}: {}",
                run.staged.case.name,
                run.output
            );
            Some(id)
        }
        "none" => None,
        other => panic!("unsupported identity source {other:?}"),
    };
    let native_path = identity_id.map_or_else(
        || expect.native_path.clone(),
        |id| expect.native_path.replace("{id}", id),
    );

    let streams = chain.streams();
    let requests: Vec<_> = streams
        .values()
        .filter(|stream| {
            stream.source == CaptureSource::WireRequest
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
    let (source, binding) = expected_identity(expect);
    assert_eq!(loaded.manifest.capture.identity.source, source);
    assert_eq!(loaded.manifest.capture.identity.binding, binding);

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
    assert_eq!(
        run.staged.world.upload_dials(),
        0,
        "an offline kit case dialled the daemon upload URL"
    );
    assert_credential_kept_to_the_child(run);
}

/// For a keyed case: the stand-in received [`KIT_CREDENTIAL`], the launcher said its key scan
/// was clean, and no fragment of the value is in the trace, the run directory or the forwarded
/// output. A case without `credential_env` is not keyed and this checks nothing.
fn assert_credential_kept_to_the_child(run: &Completed) {
    use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome, scan_tree};
    let Some(name) = run.staged.case.expect.credential_env.as_deref() else {
        return;
    };
    let case = &run.staged.case.name;
    assert!(
        goalcup_protocol::capture_policy::CREDENTIAL_ENV.contains(&name),
        "{case}: `{name}` is not an admitted credential variable"
    );
    let marker = std::fs::read_to_string(&run.staged.marker).unwrap();
    let received = format!("{KIT_CREDENTIAL_MARKER_PREFIX}{KIT_CREDENTIAL}");
    assert_eq!(
        marker.trim(),
        goalcup_daemon::workspace::sha256_hex(received.as_bytes()),
        "{case}: the stand-in did not receive the kit credential in `{name}`"
    );
    assert!(
        run.output.contains("key scan: clean"),
        "{case}: the launcher did not report a clean key scan: {}",
        run.output
    );
    let n = KIT_CREDENTIAL.len();
    for form in [
        KIT_CREDENTIAL,
        &KIT_CREDENTIAL[..8],
        &KIT_CREDENTIAL[n - 8..],
    ] {
        assert!(
            !run.output.contains(form),
            "{case}: the forwarded stdout or stderr holds a fragment of the credential"
        );
    }
    let matcher = FragmentMatcher::new(KIT_CREDENTIAL.as_bytes()).expect("a usable credential");
    let trace = run.staged.world.trace(&run.entry);
    let run_dir = run.staged.world.run_dir().expect("one run directory");
    assert!(
        trace.join("chain.jsonl").is_file() && trace.join("seal.json").is_file(),
        "{case}: the sealed trace is missing"
    );
    match scan_tree(&matcher, &[trace.as_path(), run_dir.as_path()], 1 << 30) {
        ScanOutcome::Clean => {}
        other => {
            panic!("{case}: the trace or run directory is not clean of the credential: {other:?}")
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

    // What an escaped harness would do: each one trips it.
    let trips: [(&str, &dyn Fn()); 4] = [
        ("~/.codex/config.toml changed", &|| {
            std::fs::write(codex.join("config.toml"), b"after!").unwrap()
        }),
        ("~/.codex/auth.json created", &|| {
            std::fs::write(codex.join("auth.json"), b"{}").unwrap()
        }),
        ("~/.claude/settings.json created", &|| {
            std::fs::write(claude.join("settings.json"), b"{}").unwrap()
        }),
        ("a file created under ~/.hermes", &|| {
            std::fs::create_dir_all(home.path().join(".hermes").join("sessions")).unwrap();
            std::fs::write(home.path().join(".hermes/sessions/s.json"), b"{}").unwrap();
        }),
    ];
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

#[test]
fn every_kit_case_requires_the_dev_door() {
    with_real_home_guard(|| {
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
            let completed = staged.complete();
            assert!(
                completed.staged.marker.is_file(),
                "{}: {}",
                case.name,
                completed.output
            );
        }
    });
}

#[test]
fn every_kit_case_seals_and_matches_expectations() {
    with_real_home_guard(|| {
        for case in cases() {
            let run = Staged::new(&case).complete();
            assert_completed(&run);
        }
    });
}

#[test]
fn a_denied_argv_is_refused_before_spawn() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "synthetic-stdout-json")
        .expect("the stdout-json case");
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

#[test]
fn nested_duplicate_and_implicit_home_paths_are_created_with_private_modes() {
    with_real_home_guard(|| {
        let case = cases()
            .into_iter()
            .find(|case| case.name == "synthetic-stdout-json")
            .expect("the stdout-json case");
        let run = Staged::new(&case).complete();
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
    });
}
