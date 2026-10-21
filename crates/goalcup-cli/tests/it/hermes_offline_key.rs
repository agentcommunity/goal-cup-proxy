//! P1 for Hermes in chat mode from its own home (D61): no fragment of the participant's
//! provider key reaches what goalcup records or uploads.
//!
//! Goal Cup Proxy never sees, passes or stores that key. The participant's own setup puts it in
//! `$HERMES_HOME/.env` (Hermes's own file, in the one home every run of the connector shares),
//! and Hermes reads it from there. The promise is about what can still go wrong: the agent prints
//! that file into its session, and the session file is recorded and uploaded.
//!
//! The first two tests run the shipped `hermes-cli` connector, byte for byte as pinned, through
//! the real `goalcup start` on a terminal of its own, against the rig's daemon and the stand-in
//! upload server. The harness is a stand-in for Hermes's classic REPL ([`STAND_IN`]): it answers
//! the connector's `config set` command as Hermes does, and only once that setting is on does it
//! write a session file, so a captured session also shows that the start put the setting into a
//! home that did not have it (review focus 2). That session holds the home's `.env` three ways,
//! as an agent's tool output would: raw, `base64` wrapped at 76 columns, and as JSON inside a
//! JSON tool result. The synthetic key is `sk-` and 32 random hex digits, made per test and held
//! in memory; nothing reads a real key, and nothing is sent anywhere but loopback.
//!
//! What is searched is every 8-byte window of the key, in the chain's files, in every stream's
//! decoded bytes, in every upload chunk the server stored (decompressed, and its segments
//! decoded) and in the uploaded baseline. A failure names the window's offset, never its bytes.
//!
//! What this does not claim: the `base64` print stays in the session as base64 text. The daemon
//! removes a key by its shape, and a key goalcup never sees cannot be matched once encoded
//! (D61, docs/CHEAT-CHECKLIST.md). No 8-byte window of the key is in that text, which is all the
//! search below can show of it.
//!
//! The last test is `#[ignore]`d and run by name: the real installed Hermes 0.19.0, offline, in
//! a fresh home that holds only a synthetic key in `.env`:
//!
//! ```text
//! GOALCUP_TEST_HERMES_EXE=/path/to/hermes \
//!   cargo test --offline -p goalcup-cli --test it hermes_offline_key:: -- --ignored --test-threads=1
//! ```

use crate::common::m2_cli_support::{Launcher, OnPty, Rig};
use crate::common::m2_common::TestPki;
use crate::common::support;
use crate::common::upload_server::{ServerOptions, UploadServer};

use goalcup_cli::launch::{CaptureLaunch, PrepLaunch, build_capture_launch, configure_harness};
use goalcup_protocol::capture::CaptureRecord;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The connector under test.
const ID: &str = "hermes-cli";

/// The session id the stand-in reports, in Hermes's grammar.
const SESSION: &str = "20261016_120000_abcdef";

/// What the participant types.
const PROMPT: &str = "print the env file";

/// The session id the stand-in moves to when a compaction rotates it.
const ROTATED: &str = "20261016_120500_fedcba";

/// A stand-in for Hermes's classic REPL, installed as `hermes`. Its `config set` appends the key
/// to `config.yaml` as `key: value`. Its chat prints a banner, and for each line typed, once
/// `sessions.write_json_snapshots` is on, rewrites `sessions/session_<id>.json` with that line
/// and the home's `.env` printed raw, base64 wrapped at 76 columns and JSON inside JSON. A
/// `/compress` line is a compaction: with `compression.in_place` off it starts a new session id,
/// as Hermes does, so the next line goes to a second file; with it on the id stays.
const STAND_IN: &str = r#"#!/bin/sh
home="$HERMES_HOME"
if [ "$1" = config ] && [ "$2" = set ]; then
  printf '%s: %s\n' "$3" "$4" >> "$home/config.yaml"
  echo "stand-in: set $3"
  exit 0
fi
sid="@SESSION@"
echo "Session: $sid"
echo "stand-in: ready"
while IFS= read -r line; do
  if [ "$line" = /exit ]; then
    echo "stand-in: bye"
    exit 0
  fi
  if [ "$line" = /compress ]; then
    if grep -qx 'compression.in_place: false' "$home/config.yaml" 2>/dev/null; then
      sid="@ROTATED@"
    fi
    echo "stand-in: compressed"
    continue
  fi
  if grep -qx 'sessions.write_json_snapshots: true' "$home/config.yaml" 2>/dev/null; then
    mkdir -p "$home/sessions"
    raw=$(awk '{printf "%s\\n", $0}' "$home/.env")
    b64=$(base64 < "$home/.env" | tr -d '\n' | fold -w 76 | awk '{printf "%s\\n", $0}')
    nested=$(awk '{printf "%s\\\\n", $0}' "$home/.env")
    snap="$home/sessions/session_$sid.json"
    printf '{"session_id":"%s","messages":[{"role":"user","content":"%s"},{"role":"tool","content":"%s"},{"role":"tool","content":"%s"},{"role":"tool","content":"{\\"output\\": \\"%s\\"}"}]}' "$sid" "$line" "$raw" "$b64" "$nested" > "$snap.tmp" && mv "$snap.tmp" "$snap"
  fi
  echo "stand-in: noted $line"
done
"#;

/// `sk-` and 32 random hex digits, from this process's generator and nothing else.
fn synthetic_key() -> String {
    format!("sk-{:032x}", rand::random::<u128>())
}

/// The offset of the first 8-byte window of `key` that `haystack` holds, if any. Only the
/// offset is ever shown.
fn fragment_in(haystack: &[u8], key: &str) -> Option<usize> {
    let key = key.as_bytes();
    (0..=key.len() - 8).find(|&at| {
        let window = &key[at..at + 8];
        haystack.windows(8).any(|w| w == window)
    })
}

#[track_caller]
fn assert_no_fragment(what: &str, haystack: &[u8], key: &str) {
    if let Some(at) = fragment_in(haystack, key) {
        panic!("{what} holds the key's 8 bytes at offset {at}");
    }
}

/// Every segment's bytes in `records`, decoded and joined.
fn decoded(records: &[CaptureRecord]) -> Vec<u8> {
    use base64::Engine as _;
    let mut out = Vec::new();
    for record in records {
        if let CaptureRecord::Segment { data_b64, .. } = record {
            out.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
        }
    }
    out
}

/// Every `data_b64` value anywhere in the JSON lines of `body`, decoded and joined: the
/// segments of a chain or an upload chunk, whatever envelope carries them.
fn decoded_lines(body: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    fn walk(value: &serde_json::Value, out: &mut Vec<u8>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    match (k.as_str(), v) {
                        ("data_b64", serde_json::Value::String(text)) => out.extend(
                            base64::engine::general_purpose::STANDARD
                                .decode(text)
                                .unwrap(),
                        ),
                        _ => walk(v, out),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for line in body.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let value: serde_json::Value = serde_json::from_slice(line).unwrap();
        walk(&value, &mut out);
    }
    out
}

/// What one run left: the rig (its daemon and trace), the stand-in server, the server's run
/// id, the entry id and the shared harness home.
struct Ran {
    rig: Rig,
    server: UploadServer,
    run_id: String,
    entry_id: String,
    home: PathBuf,
}

/// One participant-shaped run of the pinned connector on the stand-in, with the shared home
/// holding `.env` (the key) and `auth.json` (a login) before it starts.
fn run_with_key_in_home(key: &str, auth_json: &str, lines: &[&str]) -> Ran {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::chat();
    support::copy_tree(
        &support::connectors_dir().join(ID),
        &rig.connectors.join(ID),
    );
    let program = rig.root.join("bin").join("hermes");
    std::fs::write(
        &program,
        STAND_IN
            .replace("@SESSION@", SESSION)
            .replace("@ROTATED@", ROTATED),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

    // The home as a participant's own setup left it: a key in `.env` (mode 0600, as Hermes
    // writes it), a login in `auth.json`, and no session setting.
    let homes = rig.cfg.join("capture-homes");
    let home = homes.join(ID);
    for dir in [&homes, &home] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (name, text) in [
        (".env", format!("DEEPSEEK_API_KEY={key}\nHERMES_LOG=info\n")),
        ("auth.json", auth_json.to_string()),
    ] {
        std::fs::write(home.join(name), text).unwrap();
        std::fs::set_permissions(home.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // The model service Hermes's setup saved beside them (D62).
    let from = goalcup_protocol::harness::load_dir(
        &rig.connectors.join(ID),
        goalcup_protocol::harness::Origin::Pinned,
    )
    .unwrap()
    .manifest
    .capture
    .model_host_from
    .expect("Hermes reads its model host");
    support::write_model_host_config(&home, &from, "https://api.deepseek.com/v1");

    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut start = OnPty::spawn(
        rig.chat_command_for(
            ID,
            &[
                // No provider is reached: the host probe is skipped, and the stand-in calls none.
                (goalcup_cli::commands::start::TEST_OFFLINE_UPSTREAMS, "1"),
                ("GOALCUP_TEST_DISK", "4194304,0,50"),
                (
                    "GOALCUP_TEST_UPLOAD_POLICY",
                    "200,25,100,400,10000,268435456,3000,100",
                ),
            ],
            Launcher::Direct,
        ),
        None,
    );
    start.wait_shown("stand-in: ready");
    for line in lines {
        start.type_line(line);
        start.wait_shown(&if *line == "/compress" {
            "stand-in: compressed".to_string()
        } else {
            format!("stand-in: noted {line}")
        });
    }
    start.type_line("/exit");
    let status = start.wait();
    let shown = start.shown();
    assert!(status.success(), "{status:?}\n{shown}");
    let runs = server.run_ids();
    assert_eq!(runs.len(), 1, "{shown}");
    let entry_id = goalcup_cli::entries::latest_entry(&rig.cfg)
        .unwrap()
        .expect("an entry was recorded")
        .entry_id;
    Ran {
        run_id: runs[0].clone(),
        rig,
        server,
        entry_id,
        home,
    }
}

/// The agent prints the home's `.env` into its session raw, base64-wrapped and JSON-escaped.
/// The session is recorded (so the start turned the setting on) and uploaded, and no 8-byte
/// window of the key is in the chain, its decoded streams, the upload chunks or the baseline.
#[test]
fn an_agent_that_prints_the_homes_env_leaves_no_key_fragment_in_the_trace_or_the_upload() {
    let key = synthetic_key();
    let ran = run_with_key_in_home(&key, "{\"version\": 1, \"providers\": {}}\n", &[PROMPT]);
    let (report, records) = ran.rig.read_trace(&ran.entry_id);
    assert!(report.sealed, "{:?}", report.findings);

    // The session was captured: the start ran the connector's `config set`, and the stand-in
    // wrote its session only once the setting was on.
    let streams = decoded(&records);
    let text = String::from_utf8_lossy(&streams);
    assert!(text.contains(PROMPT), "the session file was not recorded");
    assert!(
        std::fs::read_to_string(ran.home.join("config.yaml"))
            .unwrap()
            .contains("write_json_snapshots: true")
    );
    // The raw and the nested print were each removed by shape.
    assert!(text.matches("<redacted:").count() >= 2, "{text}");
    assert_no_fragment("the decoded streams", &streams, &key);

    // Every file of the trace: the chain's parts, the seal and the baseline.
    let trace = ran.rig.trace(&ran.entry_id);
    let mut files = 0;
    for entry in std::fs::read_dir(&trace).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            files += 1;
            let bytes = std::fs::read(entry.path()).unwrap();
            assert_no_fragment(&entry.file_name().to_string_lossy(), &bytes, &key);
        }
    }
    assert!(files >= 2, "the trace holds {files} files");

    // What the server was sent: each chunk, decompressed, and the segments it carries.
    let chunks = ran.server.chunks(&ran.run_id);
    assert!(!chunks.is_empty(), "nothing was uploaded");
    let mut uploaded = Vec::new();
    for (n, chunk) in chunks.iter().enumerate() {
        let body = zstd::decode_all(std::fs::read(&chunk.body_path).unwrap().as_slice()).unwrap();
        assert_no_fragment(&format!("upload chunk {n}"), &body, &key);
        uploaded.extend(decoded_lines(&body));
    }
    // The chunks carry the session: the search covered what it is about.
    assert!(
        String::from_utf8_lossy(&uploaded).contains(PROMPT),
        "the session file was not uploaded"
    );
    assert_no_fragment("the upload chunks' decoded segments", &uploaded, &key);
    let (seal, baseline) = ran.server.seal(&ran.run_id).expect("the seal was sent");
    assert_no_fragment("the uploaded seal", seal.as_bytes(), &key);
    assert_no_fragment("the uploaded baseline", baseline.as_bytes(), &key);
}

/// A compaction mid-run (D61): with the connector's `compression.in_place false`, Hermes moves
/// to a new session id and so a second `session_<id>.json`. Both files are recorded, the text
/// before the compaction from the first and the text after it from the second.
#[test]
fn a_compaction_mid_run_starts_a_second_session_file_and_both_are_recorded() {
    const AFTER: &str = "the turn after the compaction";
    let key = synthetic_key();
    let ran = run_with_key_in_home(
        &key,
        "{\"version\": 1, \"providers\": {}}\n",
        &[PROMPT, "/compress", AFTER],
    );
    assert!(
        std::fs::read_to_string(ran.home.join("config.yaml"))
            .unwrap()
            .lines()
            .any(|l| l == "compression.in_place: false"),
        "the start set the compaction to start a new session"
    );
    for sid in [SESSION, ROTATED] {
        assert!(
            ran.home
                .join("sessions")
                .join(format!("session_{sid}.json"))
                .is_file(),
            "session_{sid}.json"
        );
    }
    let (report, records) = ran.rig.read_trace(&ran.entry_id);
    assert!(report.sealed, "{:?}", report.findings);
    let opened: Vec<&str> = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open { metadata, .. } => Some(metadata.label.as_str()),
            _ => None,
        })
        .collect();
    for sid in [SESSION, ROTATED] {
        assert!(
            opened
                .iter()
                .any(|l| l.contains(&format!("session_{sid}.json"))),
            "no stream for session_{sid}.json: {opened:?}"
        );
    }
    let text = String::from_utf8_lossy(&decoded(&records)).into_owned();
    assert!(text.contains(PROMPT) && text.contains(AFTER), "{text}");
    assert_no_fragment("the decoded streams", text.as_bytes(), &key);
}

/// Review focus 4: a shared home holding `.env` (a key) and `auth.json` (a token) gives the
/// uploaded baseline one row for each with its name, size and SHA-256 and nothing else, as
/// Codex's `auth.json` has today (D56); neither file's content is in the baseline.
#[test]
fn the_uploaded_baseline_holds_only_the_name_size_and_digest_of_a_key_file() {
    let key = synthetic_key();
    let token = format!("tok_{:032x}", rand::random::<u128>());
    let auth = format!(
        "{{\"version\": 1, \"providers\": {{\"nous\": {{\"access_token\": \"{token}\"}}}}}}\n"
    );
    let ran = run_with_key_in_home(&key, &auth, &[PROMPT]);
    let (_, baseline) = ran.server.seal(&ran.run_id).expect("the seal was sent");
    let manifest: serde_json::Value = serde_json::from_str(&baseline).unwrap();
    let home = manifest["home"]
        .as_array()
        .expect("the baseline lists the harness home");
    for name in [".env", "auth.json"] {
        let content = std::fs::read(ran.home.join(name)).unwrap();
        let row = home
            .iter()
            .find(|row| row["path"] == name)
            .unwrap_or_else(|| panic!("no baseline row for {name}: {home:?}"));
        let mut fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["bytes", "path", "sha256"], "{name}");
        assert_eq!(row["bytes"], content.len() as u64, "{name}");
        assert_eq!(
            row["sha256"],
            hex::encode(Sha256::digest(&content)),
            "{name}"
        );
    }
    assert_no_fragment("the uploaded baseline", baseline.as_bytes(), &key);
    assert_no_fragment("the uploaded baseline", baseline.as_bytes(), &token);
    // The trace's own copy is the same document.
    let local =
        std::fs::read_to_string(ran.rig.trace(&ran.entry_id).join("baseline.json")).unwrap();
    assert_eq!(local, baseline);
}

// ------------------------------------------------------------------ the real Hermes

/// The Hermes executable under test, taken only from `GOALCUP_TEST_HERMES_EXE`. There is no
/// default path: an unset variable fails the test with the way to set it.
fn hermes_exe() -> PathBuf {
    let Some(path) = std::env::var_os("GOALCUP_TEST_HERMES_EXE").filter(|value| !value.is_empty())
    else {
        panic!(
            "GOALCUP_TEST_HERMES_EXE is not set. Point it at the installed hermes-agent \
             executable, for example: GOALCUP_TEST_HERMES_EXE=/path/to/hermes cargo test \
             --offline -p goalcup-cli --test it hermes_offline_key:: -- --ignored --test-threads=1"
        );
    };
    PathBuf::from(path)
}

/// What the real `~/.hermes` looks like: present or absent, and its entry names and mtimes.
/// Names and times only, never contents.
fn real_hermes_listing() -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return "no HOME".to_string();
    };
    let dir = Path::new(&home).join(".hermes");
    let Ok(meta) = std::fs::symlink_metadata(&dir) else {
        return "absent".to_string();
    };
    let mut lines = vec![format!("present {:?}", meta.modified().ok())];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            lines.push(format!(
                "{} {:?}",
                e.file_name().to_string_lossy(),
                e.metadata().and_then(|m| m.modified()).ok()
            ));
        }
    }
    lines.sort();
    lines.join("\n")
}

/// The real Hermes 0.19.0 in a fresh home that holds only a synthetic key in `.env`, as a
/// participant's setup leaves one: the connector's `configure` command, run as `goalcup start`
/// runs it, turns the session file on; Hermes then opens its chat on the production capture
/// plan (its proxy a closed loopback port, so no request leaves the machine) and, after one
/// message, writes `sessions/session_<id>.json`, which the connector's native pattern reads
/// and which holds no 8-byte window of the key. The request dump the failed call leaves beside
/// it does hold one, and the pattern does not read it. The real `~/.hermes` is not touched.
#[test]
#[ignore = "runs the real installed Hermes offline; run by name"]
fn the_real_hermes_writes_its_session_file_once_configured_and_keeps_its_key_out_of_it() {
    use std::os::unix::fs::PermissionsExt;
    let exe = hermes_exe();
    assert!(
        exe.is_file(),
        "the installed Hermes is not at {}",
        exe.display()
    );
    let before = real_hermes_listing();

    let tmp = tempfile::tempdir().unwrap();
    let dir = std::fs::canonicalize(tmp.path()).unwrap();
    let (run_home, hermes_home, workspace) =
        (dir.join("home"), dir.join("hermes"), dir.join("workspace"));
    for d in [&run_home, &hermes_home, &workspace, &dir.join("ca")] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let key = synthetic_key();
    std::fs::write(
        hermes_home.join(".env"),
        format!("DEEPSEEK_API_KEY={key}\n"),
    )
    .unwrap();

    let connector = support::connector(ID);
    // The launcher runs the file a symlink points at (an installer's `bin` holds a link).
    let resolved =
        goalcup_cli::launch::resolve_harness_executable(&std::fs::canonicalize(&exe).unwrap())
            .unwrap();
    let path = format!(
        "{}:/usr/bin:/bin:/usr/sbin:/sbin",
        exe.parent().unwrap().display()
    );
    let parent: BTreeMap<String, String> = [
        ("PATH", path.as_str()),
        ("TERM", "xterm-256color"),
        ("LANG", "en_US.UTF-8"),
        ("USER", "participant"),
        ("LOGNAME", "participant"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let log_path = dir.join("host.log");
    let log = std::fs::File::create(&log_path).unwrap();
    configure_harness(
        &connector,
        &PrepLaunch {
            executable: &resolved,
            run_home: &run_home,
            config_home: &hermes_home,
            parent_env: &parent,
        },
        &workspace,
        &log,
        "(test)",
    )
    .expect("the connector's configure command");
    let config = std::fs::read_to_string(hermes_home.join("config.yaml")).unwrap();
    assert!(config.contains("write_json_snapshots: true"), "{config}");
    assert!(config.contains("in_place: false"), "{config}");

    // A loopback port nothing listens on: every call Hermes makes is refused at once.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let proxy_url = format!(
        "http://goalcup:{:032x}@127.0.0.1:{closed}",
        rand::random::<u128>()
    );
    let ca = dir.join("ca/proxy-ca.pem");
    std::fs::write(&ca, TestPki::new("goal cup test CA").root_pem()).unwrap();
    let mut vars = goalcup_protocol::template::TemplateVars::new();
    vars.set("workspace", workspace.to_str().unwrap());
    let plan = build_capture_launch(
        &connector,
        &vars,
        &workspace,
        &CaptureLaunch {
            goal: None,
            executable: &resolved,
            run_home: &run_home,
            config_home: &hermes_home,
            proxy_url: &proxy_url,
            ca_certificate: &ca,
            ca_bundle: None,
            parent_env: &parent,
        },
    )
    .expect("the capture plan");
    assert!(plan.env_set.keys().all(|k| !k.contains("API_KEY")));
    let mut command = std::process::Command::new(&plan.program);
    command
        .args(&plan.argv)
        .current_dir(&plan.cwd)
        .env_clear()
        .envs(&plan.env_set);
    let mut hermes = OnPty::spawn(command, None);
    hermes.wait_shown("Session:");
    hermes.type_line("hello");
    let sessions = hermes_home.join("sessions");
    let names = || -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&sessions)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    };
    let until = Instant::now() + Duration::from_secs(120);
    while !names().iter().any(|n| n.starts_with("session_")) {
        assert!(Instant::now() < until, "no session file: {:?}", names());
        std::thread::sleep(Duration::from_millis(200));
    }
    // Let the failed call finish before the chat is ended.
    std::thread::sleep(Duration::from_secs(5));
    hermes.type_line("/exit");
    let status = hermes.wait();
    assert!(status.success(), "{status:?}\n{}", hermes.shown());

    let native = &connector.loaded.manifest.capture.native[0];
    let (prefix, suffix) = native.pattern.split_once('*').expect("a one-star pattern");
    let read = |n: &str| n.starts_with(prefix) && n.ends_with(suffix);
    let all = names();
    let snapshots: Vec<&String> = all.iter().filter(|n| read(n)).collect();
    assert_eq!(snapshots.len(), 1, "{all:?}");
    let snapshot = std::fs::read(sessions.join(snapshots[0])).unwrap();
    assert!(String::from_utf8_lossy(&snapshot).contains("hello"));
    assert_no_fragment("the session file", &snapshot, &key);
    let dumps: Vec<&String> = all
        .iter()
        .filter(|n| n.starts_with("request_dump_"))
        .collect();
    assert!(!dumps.is_empty(), "no request dump: {all:?}");
    for dump in dumps {
        assert!(!read(dump), "the native pattern reads {dump}");
        let bytes = std::fs::read(sessions.join(dump)).unwrap();
        assert!(
            fragment_in(&bytes, &key).is_some(),
            "the positive control: the dump holds a fragment"
        );
    }
    assert_eq!(
        real_hermes_listing(),
        before,
        "the real ~/.hermes changed while Hermes ran"
    );
}
