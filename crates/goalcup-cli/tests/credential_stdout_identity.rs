//! A keyed child whose session identity is a stdout JSON event.
//!
//! One stdout reader feeds both the stdout JSON identity matcher and the key-fragment scan, so a
//! credential no longer forbids `stdout_json` identity. Each test spawns a real Python child with
//! the synthetic key (`secret_support::KEY`) in its environment, the way the launcher hands it
//! over, and reads its streams through the same tee the launcher uses.
#![cfg(unix)]

mod secret_support;

use goalcup_cli::launch::{
    ChildSecret, LaunchScope, Spawned, StdoutJsonRule, StdoutUse, StreamPlan,
    child_secret_for_harness, tee_marked_capture_with_identity_to, wait_native_scope,
};
use goalcup_protocol::harness::{CaptureSpec, Origin};
use secret_support::KEY;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "T-5f0c2a8e-8a0e-4c1f-9d6b-2b7d2b0e9a11";
const PATIENCE: Duration = Duration::from_secs(10);
const WITHHELD: &str = "[goalcup: output withheld: a fragment of the provider key]";

/// The child: an init event first, then either a clean line or the key it was handed, echoed
/// the way a harness might on an auth error. The key comes from its environment, never argv.
/// The `*-id` branches put the key in the init event's `session_id` instead: whole, as a
/// fragment inside a longer id, or whole but spelled entirely in JSON `\u` escapes so the line
/// itself holds no scan form and only the decoded id does.
const CHILD: &str = r#"
import json, os, sys
key = os.environ["DEEPSEEK_API_KEY"]
session = {
    "key-id": key,
    "fragment-id": "sess-" + key[-8:] + "-0001",
}.get(sys.argv[2], sys.argv[1])
if sys.argv[2] == "escaped-id":
    escaped = "".join("\\u%04x" % ord(c) for c in key)
    print('{"type": "system", "subtype": "init", "session_id": "' + escaped + '"}', flush=True)
else:
    print(json.dumps({"type": "system", "subtype": "init", "session_id": session}), flush=True)
del key
if sys.argv[2] == "echo":
    print("Invalid API key: " + os.environ["DEEPSEEK_API_KEY"], flush=True)
else:
    print("hello from a keyed child", flush=True)
print("stderr is scanned too", file=sys.stderr, flush=True)
"#;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// The shipped Codex profile (`stdout_json` identity) with a credential and a per-run home: the
/// combination this change admits.
fn keyed_stdout_json_profile() -> CaptureSpec {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let text = std::fs::read_to_string(dir.join("codex-cli/harness.toml")).unwrap();
    // Two edits to the shipped text: a per-run config home, and a credential table placed
    // before `[capture.identity]` so it is a sibling of the other `[capture.*]` tables.
    let keyed = text
        .replacen("config_home = \"shared\"", "config_home = \"per_run\"", 1)
        .replacen(
            "[capture.identity]",
            "[capture.credential]\nenv = \"DEEPSEEK_API_KEY\"\n\n[capture.identity]",
            1,
        );
    assert_ne!(keyed, text);
    let manifest = goalcup_protocol::harness::HarnessManifest::from_toml_str(&keyed).unwrap();
    manifest
        .validate(Origin::Pinned)
        .expect("a credential with stdout_json identity and a per-run home is admitted");
    manifest.capture
}

fn secret(profile: &CaptureSpec) -> ChildSecret {
    let env = std::collections::BTreeMap::from([("DEEPSEEK_API_KEY".to_string(), KEY.to_string())]);
    child_secret_for_harness(profile, &env).unwrap().unwrap()
}

struct Run {
    scope: Option<String>,
    key_hit: bool,
    out: String,
    err: String,
}

fn run(branch: &str) -> Run {
    let profile = keyed_stdout_json_profile();
    let plan = StreamPlan::for_spec(&profile).unwrap();
    assert!(plan.scan_for_key);
    assert_eq!(
        LaunchScope::for_spec(&profile).unwrap(),
        LaunchScope::FromStdout
    );
    let pipes = plan.pipes();
    assert!(pipes.stdout && pipes.stderr, "both streams are piped");
    assert!(
        matches!(plan.stdout, StdoutUse::IdentityJson(_)),
        "stdout is the identity reader"
    );
    // The shipped profile's rule is Codex's; this child speaks the shape Amp documents.
    let rule = StdoutJsonRule {
        match_field: "subtype".into(),
        match_value: "init".into(),
        id_field: "session_id".into(),
    };
    let secret = secret(&profile);
    let mut command = std::process::Command::new("/usr/bin/python3");
    command
        .args(["-I", "-S", "-c", CHILD, ID, branch])
        .env_clear()
        .env(secret.name(), KEY)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut spawned = Spawned {
        child: command.spawn().unwrap(),
        program: "/usr/bin/python3".into(),
    };
    let (out, err) = (Sink::default(), Sink::default());
    let (stdout, tee) = tee_marked_capture_with_identity_to(
        &mut spawned,
        Some(&secret),
        Some(rule),
        None,
        out.clone(),
        err.clone(),
    )
    .unwrap();
    let stdout = stdout.expect("an identity rule yields a scope receiver");
    let scope = wait_native_scope(&mut spawned.child, &stdout.scope, Instant::now() + PATIENCE);
    spawned.child.wait().unwrap();
    assert!(stdout.finish(Instant::now() + PATIENCE).unwrap());
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    Run {
        scope,
        key_hit: tee.key_hit(),
        out: out.text(),
        err: err.text(),
    }
}

#[test]
fn one_stdout_reader_binds_the_identity_and_forwards_clean_output_verbatim() {
    let run = run("clean");
    assert_eq!(
        run.scope.as_deref(),
        Some(ID),
        "the init event bound the identity"
    );
    assert!(!run.key_hit);
    assert_eq!(
        run.out,
        format!(
            "{{\"type\": \"system\", \"subtype\": \"init\", \"session_id\": \"{ID}\"}}\n\
             hello from a keyed child\n"
        )
    );
    assert_eq!(run.err, "stderr is scanned too\n");
}

#[test]
fn a_key_echoed_on_stdout_is_a_hit_and_withheld_while_the_identity_still_binds() {
    let run = run("echo");
    assert_eq!(
        run.scope.as_deref(),
        Some(ID),
        "the init event bound the identity"
    );
    assert!(
        run.key_hit,
        "the key on stdout is a live hit, which fails the row"
    );
    let n = KEY.len();
    for form in [KEY, &KEY[..8], &KEY[n - 8..]] {
        assert!(
            !run.out.contains(form),
            "stdout forwarded a fragment of the key"
        );
    }
    assert!(run.out.contains(WITHHELD), "{}", run.out);
}

/// A key the child put in the init event's `session_id` is never bound as the identity (the
/// daemon would write it into the chain as the scope), is never forwarded, and is a key hit.
fn assert_key_in_the_identity_is_refused(branch: &str) {
    let run = run(branch);
    assert_eq!(
        run.scope, None,
        "{branch}: an identity holding a fragment of the key was bound"
    );
    assert!(
        run.key_hit,
        "{branch}: a fragment of the key in the identity event is a hit, which fails the row"
    );
    let n = KEY.len();
    for form in [KEY, &KEY[..8], &KEY[n - 8..]] {
        assert!(
            !run.out.contains(form) && !run.err.contains(form),
            "{branch}: a fragment of the key was forwarded"
        );
    }
}

#[test]
fn an_init_event_whose_session_id_is_the_key_binds_nothing_and_is_a_hit() {
    assert_key_in_the_identity_is_refused("key-id");
}

#[test]
fn an_init_event_whose_session_id_holds_a_key_fragment_binds_nothing_and_is_a_hit() {
    assert_key_in_the_identity_is_refused("fragment-id");
}

#[test]
fn an_init_event_whose_escaped_session_id_decodes_to_the_key_binds_nothing_and_is_a_hit() {
    assert_key_in_the_identity_is_refused("escaped-id");
}
