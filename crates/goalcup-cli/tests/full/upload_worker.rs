//! The same end-to-end runs as `upload_e2e`, against the real Worker (`server/worker/`) under
//! `wrangler dev` on loopback: local R2 simulation, the local state backend, no Cloudflare
//! account and no network.
//!
//! `#[ignore]`d: they need Node and `npm install` in `server/worker`. Run them with
//! `cargo nextest run --offline -p goalcup-cli --test full --run-ignored ignored-only
//! -E 'test(/upload_worker/)'`.

use crate::common::m2_cli_support::{Hooked, Rig};
use goalcup_cli::server::{ServerConfig, write_server_config};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_verifier::capture_analysis::reader::{CaptureIntegrity, scan_capture_trace};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const GOAL: &str = "write hello.txt, then stop";
const RESERVE: u64 = 4 * 1024 * 1024;
const TOKEN: &str = "gct_worker_test_0123456789";
const REPO: &str = "https://github.com/goal-cup/demo";
/// The Worker's test seed: 32 bytes of 7, the seed the Rust vectors use.
const SEED_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

/// `server/worker`, from this crate's directory.
fn worker_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/worker")
}

/// One `wrangler dev`, killed (with its process group) when dropped. Its state lives in
/// `state`, so a second one on the same port and state is the same server, restarted.
struct Wrangler {
    child: Child,
    url: String,
}

impl Wrangler {
    fn start(state: &Path, port: u16) -> Wrangler {
        let dir = worker_dir();
        let wrangler = dir.join("node_modules/wrangler/bin/wrangler.js");
        assert!(
            wrangler.exists(),
            "run `npm install` in {} first",
            dir.display()
        );
        let inspector = free_port();
        let vars = [
            ("DB_BACKEND", "local".to_string()),
            ("GITHUB_CHECK", "off".to_string()),
            ("EVENT_ID", "practice".to_string()),
            ("RECEIPT_SIGNING_KEY", SEED_HEX.to_string()),
            ("DEV_TOKENS", format!("{TOKEN}:member-1")),
        ];
        let mut command = Command::new("node");
        command
            .arg(&wrangler)
            .args(["dev", "--ip", "127.0.0.1", "--port", &port.to_string()])
            .args(["--inspector-port", &inspector.to_string()])
            .arg("--persist-to")
            .arg(state.join("r2"))
            .arg("--show-interactive-dev-session=false");
        for (name, value) in vars {
            command.args(["--var", &format!("{name}:{value}")]);
        }
        use std::os::unix::process::CommandExt;
        let child = command
            .current_dir(&dir)
            // wrangler's own files stay in the test's directory, never the user's config.
            .env("XDG_CONFIG_HOME", state.join("config"))
            .env("WRANGLER_LOG_PATH", state.join("logs"))
            .env("WRANGLER_REGISTRY_PATH", state.join("registry"))
            .env("WRANGLER_SEND_METRICS", "false")
            .env("CLOUDFLARE_API_TOKEN", "")
            .env("CLOUDFLARE_ACCOUNT_ID", "")
            .env("CI", "1")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("node runs");
        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let answer = ureq::get(&format!("{url}/v1/ready-check")).call();
            if let Err(ureq::Error::Status(404, _)) = answer {
                break;
            }
            assert!(Instant::now() < deadline, "wrangler dev never answered");
            std::thread::sleep(Duration::from_millis(250));
        }
        Wrangler { child, url }
    }

    fn get(&self, path: &str, token: &str) -> serde_json::Value {
        ureq::get(&format!("{}{path}", self.url))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .unwrap()
            .into_string()
            .map(|text| serde_json::from_str(&text).unwrap())
            .unwrap()
    }
}

impl Drop for Wrangler {
    fn drop(&mut self) {
        let group = self.child.id() as i32;
        // SAFETY: signals the process group this test spawned and nothing else.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: as above.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn log_in(rig: &Rig, url: &str, token: &str) {
    write_server_config(
        &rig.cfg,
        &ServerConfig {
            url: url.to_string(),
            token: token.to_string(),
        },
    )
    .unwrap();
}

fn quick_policy() -> goalcup_cli::upload::uploader::Policy {
    goalcup_cli::upload::uploader::Policy::from_test_spec("200,25,100,400,20000,268435456,5000")
        .unwrap()
}

/// End a fake harness that is already running (see `upload_e2e::end_harness`).
fn end_harness(rig: &Rig) {
    rig.tell("stop", "");
    wait_until("the harness to see stop or start its sleep", || {
        rig.harness_file("done").is_some() || rig.harness_file("sleeper").is_some()
    });
    if let Some(pid) = rig
        .harness_file("sleeper")
        .and_then(|pid| pid.trim().parse::<i32>().ok())
    {
        // SAFETY: two integers; the pid is the fake harness's own `sleep`.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The daemon made the entry and its trace holds an acknowledgement from the Worker.
fn acknowledged(rig: &Rig, client: &Hooked<'_>) -> bool {
    let created = client.book().created.clone();
    created.is_some_and(|info| {
        std::fs::read_to_string(rig.trace(&info.entry_id).join("acks.jsonl"))
            .is_ok_and(|acks| !acks.is_empty())
    })
}

/// The run the Worker opened for the one test: its id, from the upload record.
fn run_id(rig: &Rig, entry_id: &str) -> String {
    goalcup_cli::upload::read_record(&rig.cfg, entry_id)
        .unwrap()
        .unwrap()
        .run_id
}

/// The Worker holds the whole chain and is sealed, and the trace verifies under its key with a
/// verified acknowledgement of the last record.
fn assert_fully_uploaded(rig: &Rig, worker: &Wrangler, entry_id: &str) {
    let trace = rig.trace(entry_id);
    let chain = std::fs::read(trace.join("chain.jsonl")).unwrap();
    let position = worker.get(&format!("/v1/runs/{}", run_id(rig, entry_id)), TOKEN);
    assert_eq!(position["offset"], chain.len() as u64, "{position}");
    assert_eq!(position["status"], "sealed", "{position}");
    let public_key = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
    let options = goalcup_verifier::VerifyOptions {
        extra_server_keys: vec![public_key.to_string()],
        ..Default::default()
    };
    let report = scan_capture_trace(&trace, &options, &mut |_| Ok(())).unwrap();
    assert_eq!(report.integrity, CaptureIntegrity::Valid, "{report:?}");
    assert_eq!(report.anchor, AnchorState::Signed, "{report:?}");
    let records = chain
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .count() as u64;
    assert_eq!(
        report.verified_ack.as_ref().map(|a| a.seq),
        Some(records - 1),
        "{report:?}"
    );
}

#[test]
#[ignore = "needs Node and `npm install` in server/worker"]
fn upload_worker_a_run_is_uploaded_and_sealed_and_survives_a_restart() {
    let rig = Rig::new();
    let state = rig.root.join("worker");
    let port = free_port();
    let first = Wrangler::start(&state, port);
    log_in(&rig, &first.url, TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    opts.upload_policy = quick_policy();
    let (result, out, worker) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        // The first chunk, then the Worker goes away mid-run and comes back.
        wait_until("a first acknowledgement", || acknowledged(&rig, &client));
        drop(first);
        std::thread::sleep(Duration::from_millis(600));
        let back = Wrangler::start(&state, port);
        end_harness(&rig);
        let (result, out) = running.join().unwrap();
        (result, out, back)
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("uploaded: "), "{out}");
    assert_fully_uploaded(&rig, &worker, &outcome.entry_id);
}

#[test]
#[ignore = "needs Node and `npm install` in server/worker"]
fn upload_worker_an_incomplete_upload_is_finished_by_goalcup_upload() {
    let rig = Rig::new();
    let state = rig.root.join("worker");
    let port = free_port();
    let first = Wrangler::start(&state, port);
    log_in(&rig, &first.url, TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    opts.upload_policy = quick_policy();
    opts.upload_policy.finish_budget = Duration::from_millis(300);
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        wait_until("a first acknowledgement", || acknowledged(&rig, &client));
        drop(first);
        end_harness(&rig);
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("upload incomplete"), "{out}");

    let worker = Wrangler::start(&state, port);
    let mut said = Vec::new();
    goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut said,
    )
    .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&said)));
    assert_fully_uploaded(&rig, &worker, &outcome.entry_id);
}

#[test]
#[ignore = "needs Node and `npm install` in server/worker"]
fn upload_worker_a_wrong_token_starts_nothing() {
    let rig = Rig::new();
    let worker = Wrangler::start(&rig.root.join("worker"), free_port());
    log_in(&rig, &worker.url, "gct_not_a_token_the_worker_knows");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    let error = result.expect_err("the start was not refused");
    assert!(
        error.to_string().contains("goalcup login"),
        "{error}\n{out}"
    );
    assert!(client.book().creates.is_empty(), "{out}");
}
