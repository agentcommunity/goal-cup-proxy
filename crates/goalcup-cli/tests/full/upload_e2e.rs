//! `goalcup start` against the local stand-in of the upload server: a real in-process daemon,
//! the fake harness, and the server on loopback (docs/UPLOAD-API.md).

use crate::common::m2_cli_support::{Hooked, Rig};
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_cli::server::{ServerConfig, write_server_config};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_verifier::capture_analysis::reader::{CaptureIntegrity, scan_capture_trace};

const GOAL: &str = "write hello.txt, then stop";
const RESERVE: u64 = 4 * 1024 * 1024;
const TOKEN: &str = "gct_test_0123456789abcdef";
const REPO: &str = "https://github.com/goal-cup/demo";

fn server(store: &std::path::Path) -> UploadServer {
    UploadServer::start(
        store,
        ServerOptions {
            tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
            ..ServerOptions::default()
        },
    )
}

fn log_in(rig: &Rig, url: &str, token: &str) {
    write_server_config(
        &rig.cfg,
        &ServerConfig {
            url: url.to_string(),
            account: None,
            token: token.to_string(),
        },
    )
    .unwrap();
}

/// A logged-in start opens the run before anything is created, runs under the server's entry
/// id, and keeps the signed start receipt: the sealed trace verifies with a signed anchor.
#[test]
fn a_logged_in_start_runs_under_the_servers_entry_and_keeps_its_receipt() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let server = server(&store);
    log_in(&rig, &server.url(), TOKEN);
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(format!("{REPO}.git"));
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));

    let runs = server.run_ids();
    assert_eq!(runs.len(), 1, "{out}");
    let run_id = &runs[0];
    let receipt = server.receipt(run_id).expect("a receipt was issued");
    assert_eq!(outcome.entry_id, receipt.claims.entry_id, "{out}");
    assert!(out.contains(&format!("run {run_id} opened on")), "{out}");
    assert_eq!(
        client.book().creates[0].entry_id.as_deref(),
        Some(outcome.entry_id.as_str()),
        "the daemon was asked for the server's entry id"
    );

    // The receipt the server signed is the trace's, and the trace verifies under its key.
    let trace = rig.trace(&outcome.entry_id);
    let kept: goalcup_protocol::receipt::StartReceipt =
        serde_json::from_slice(&std::fs::read(trace.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(kept, receipt);
    let options = goalcup_verifier::VerifyOptions {
        extra_server_keys: vec![server.public_key_hex()],
        ..Default::default()
    };
    let report = scan_capture_trace(&trace, &options, &mut |_| Ok(())).unwrap();
    assert_eq!(report.integrity, CaptureIntegrity::Valid, "{report:?}");
    assert_eq!(report.anchor, AnchorState::Signed, "{report:?}");

    // What a later `goalcup upload` needs is kept beside it, and the repo is normalised.
    let record = goalcup_cli::upload::read_record(&rig.cfg, &outcome.entry_id)
        .unwrap()
        .expect("an upload record");
    assert_eq!(&record.run_id, run_id);
    assert_eq!(record.trace_dir, trace.display().to_string());
}

/// A token the server does not know refuses the start before the daemon is asked anything:
/// no entry, no run directory.
#[test]
fn a_refused_token_starts_nothing() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), "gct_not_a_token_the_server_knows");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    let error = result.expect_err("the start was not refused");
    assert_eq!(error.exit_code(), 1, "{error}");
    assert!(error.to_string().contains("goalcup login"), "{error}");
    assert!(!error.to_string().contains("gct_not_a_token"), "{error}");
    assert!(client.book().creates.is_empty(), "{out}");
    assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
    assert!(!rig.harness_started());
    assert!(server.run_ids().is_empty());
}

/// No server answering refuses the start the same way, and so does a missing `--repo`.
#[test]
fn an_unreachable_server_or_a_missing_repo_starts_nothing() {
    let rig = Rig::new();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    log_in(&rig, &url, TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    for repo in [Some(REPO.to_string()), None] {
        let mut opts = rig.options(&goal, RESERVE);
        opts.repo = repo.clone();
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let error = result.expect_err("the start was not refused");
        assert_eq!(error.exit_code(), 1, "{error}");
        let said = error.to_string();
        match repo {
            Some(_) => assert!(said.contains("could not be reached"), "{said}"),
            None => assert!(said.contains("--repo"), "{said}"),
        }
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty());
    }
}

/// A start that fails after the run opened and before the launch gives the run back, so the
/// account is not left holding an open run with nothing in it.
#[test]
fn a_start_that_fails_before_the_launch_gives_the_run_back() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    // The proxy cannot write its CA file, after the daemon made the entry.
    client.book().occupy_ca_file = true;
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    result.expect_err("the start did not fail");
    let runs = server.run_ids();
    assert_eq!(runs.len(), 1, "{out}");
    assert_eq!(
        server.status(&runs[0]).as_deref(),
        Some("abandoned"),
        "{out}"
    );
    assert!(!rig.harness_started());
    assert_nothing_kept(&rig);
}

/// The client speaks TLS with this crate's own rustls: a server whose certificate chains to a
/// trusted root answers, and the same server is refused by a client trusting only the public
/// roots, before any request (and so before the token) is sent, as a failure that asking again
/// does not cure.
#[test]
fn the_client_speaks_tls_and_refuses_an_untrusted_certificate() {
    use goalcup_cli::upload::http::ServerClient;
    use rustls::pki_types::pem::PemObject;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let upstream = crate::common::m2_cli_support::Upstream::start(&runtime);
    let config = ServerConfig {
        url: format!("https://localhost:{}", upstream.port),
        account: None,
        token: TOKEN.to_string(),
    };
    let root =
        rustls::pki_types::CertificateDer::from_pem_slice(upstream.root_pem().as_bytes()).unwrap();
    let trusted = ServerClient::with_roots(&config, [root]);
    let answer: serde_json::Value = trusted
        .json("GET", "/v1/runs/r0000000000000000", None::<&()>)
        .unwrap();
    assert_eq!(answer, serde_json::json!({}));

    let public = ServerClient::new(&config);
    let refused = public
        .json::<serde_json::Value>("GET", "/v1/runs/r0000000000000000", None::<&()>)
        .unwrap_err();
    assert!(!refused.is_retryable(), "{refused}");
    assert!(refused.to_string().contains("TLS"), "{refused}");
}

/// Nothing of a given-back run stays on this machine: no upload record, no receipt copy, no
/// note of a pending open.
fn assert_nothing_kept(rig: &Rig) {
    let uploads = rig.cfg.join("uploads");
    let left: Vec<_> = std::fs::read_dir(&uploads)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?} left in {}", uploads.display());
}

/// A failure after the fork and before the harness runs (here the daemon refuses the
/// registration) gives the run back too.
#[test]
fn a_launch_refused_after_the_fork_gives_the_run_back() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().registration = Some(crate::common::m2_cli_support::RegistrationFault::Refused);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    result.expect_err("the start did not fail");
    let runs = server.run_ids();
    assert_eq!(runs.len(), 1, "{out}");
    assert_eq!(
        server.status(&runs[0]).as_deref(),
        Some("abandoned"),
        "{out}"
    );
    assert!(!rig.harness_started());
    assert_nothing_kept(&rig);
}

/// A start killed between the open and the launch cannot give its run back; it leaves a note,
/// and the next start gives that run back first instead of meeting "one open run" refused.
#[test]
fn a_run_left_open_by_a_killed_start_is_given_back_by_the_next() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), TOKEN);
    let config = goalcup_cli::server::read_server_config(&rig.cfg)
        .unwrap()
        .unwrap();
    let client = goalcup_cli::upload::http::ServerClient::new(&config);
    let request = goalcup_cli::upload::api::OpenRunRequest {
        connector_id: "m2-fake".into(),
        manifest_sha256: "a".repeat(64),
        connector_dir_sha256: "b".repeat(64),
        workspace: "/nowhere/workspace".into(),
        session_id: "00000000-0000-4000-8000-000000000001".into(),
        harness_version: "1".into(),
        tracer_version: "0.1.0".into(),
        repo_url: REPO.into(),
    };
    let killed = goalcup_cli::upload::open_run(&client, &request).unwrap();
    // What the killed start left: the note, and nothing that gives the run back.
    std::mem::forget(goalcup_cli::upload::GiveBack::opened(&client, &rig.cfg, &killed).unwrap());

    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let hooked = Hooked::new(&http, &rig);
    let mut opts = rig.options(&goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    let (result, out) = std::thread::scope(|s| rig.start(s, &hooked, opts, None).join().unwrap());
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(
        server.status(&killed.run_id).as_deref(),
        Some("abandoned"),
        "{out}"
    );
    assert_eq!(server.run_ids().len(), 2, "{out}");
    assert!(!goalcup_cli::upload::pending_path(&rig.cfg).exists());
}

// ------------------------------------------------------------------ the uploader

/// A cadence a test can watch: an "hour" of 200 ms, polled every 25 ms, backoff from 100 ms to
/// 400 ms, up to 10 s to finish after the seal, and 3 s for one chunk.
fn quick_policy() -> goalcup_cli::upload::uploader::Policy {
    goalcup_cli::upload::uploader::Policy::from_test_spec("200,25,100,400,10000,268435456,3000")
        .unwrap()
}

/// End a fake harness that is already running: it reads `stop` once, so one that got there
/// first is waiting on its one `sleep`, which is ended here.
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what} never happened"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// The server's copy of an entry: its chunks, decompressed and joined in offset order.
fn server_copy(server: &UploadServer, run_id: &str) -> Vec<u8> {
    let mut chunks = server.chunks(run_id);
    chunks.sort_by_key(|c| c.offset);
    let mut joined = Vec::new();
    for chunk in chunks {
        assert_eq!(chunk.offset, joined.len() as u64, "chunks leave no hole");
        let body = std::fs::read(&chunk.body_path).unwrap();
        let raw = zstd::decode_all(body.as_slice()).unwrap();
        assert_eq!(raw.len() as u64, chunk.length);
        joined.extend_from_slice(&raw);
    }
    joined
}

/// The server holds the whole chain and the seal, the trace's `acks.jsonl` holds a verified
/// acknowledgement of its last record, and the trace verifies under the server's key.
fn assert_fully_uploaded(rig: &Rig, server: &UploadServer, run_id: &str, entry_id: &str) {
    let trace = rig.trace(entry_id);
    let chain = std::fs::read(trace.join("chain.jsonl")).unwrap();
    assert_eq!(
        server_copy(server, run_id),
        chain,
        "the server's copy is the chain"
    );
    let (seal_json, baseline_json) = server.seal(run_id).expect("the seal was sent");
    assert_eq!(
        seal_json.as_bytes(),
        std::fs::read(trace.join("seal.json")).unwrap()
    );
    assert_eq!(
        baseline_json.as_bytes(),
        std::fs::read(trace.join("baseline.json")).unwrap()
    );
    let options = goalcup_verifier::VerifyOptions {
        extra_server_keys: vec![server.public_key_hex()],
        ..Default::default()
    };
    let report = scan_capture_trace(&trace, &options, &mut |_| Ok(())).unwrap();
    assert_eq!(report.integrity, CaptureIntegrity::Valid, "{report:?}");
    assert_eq!(report.anchor, AnchorState::Signed, "{report:?}");
    let last_seq = chain
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .count() as u64
        - 1;
    let acked = report
        .verified_ack
        .as_ref()
        .expect("a verified acknowledgement");
    assert_eq!(acked.seq, last_seq, "the last record is acknowledged");
    // Each acknowledgement is kept once.
    let acks = std::fs::read_to_string(trace.join("acks.jsonl")).unwrap();
    let seqs: Vec<u64> = acks
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["seq"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
}

fn logged_in_options(
    rig: &Rig,
    goal: &std::path::Path,
) -> goalcup_cli::commands::start::StartOptions {
    let mut opts = rig.options(goal, RESERVE);
    opts.repo = Some(REPO.to_string());
    opts.upload_policy = quick_policy();
    opts
}

/// A normal run: the first chunk goes at the launch, more follow, and at the end the rest and
/// the seal; everything is acknowledged.
#[test]
fn a_run_is_uploaded_as_it_records_and_sealed_at_the_end() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let opts = logged_in_options(&rig, &goal);
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        // The first chunk lands while the harness is still running.
        wait_until("a first chunk", || {
            server
                .run_ids()
                .first()
                .is_some_and(|run| !server.chunks(run).is_empty())
        });
        end_harness(&rig);
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("uploaded: "), "{out}");
    let run_id = server.run_ids()[0].clone();
    assert!(server.chunks(&run_id).len() >= 2, "{out}");
    assert_eq!(server.status(&run_id).as_deref(), Some("sealed"));
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
}

/// The server goes away mid-run and comes back: nothing is lost, and the run ends uploaded.
#[test]
fn a_server_that_goes_away_mid_run_gets_everything_once_it_is_back() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let first = server(&store);
    let port = first.port();
    log_in(&rig, &first.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let opts = logged_in_options(&rig, &goal);
    let (result, out, server) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        wait_until("a first chunk", || {
            first
                .run_ids()
                .first()
                .is_some_and(|run| !first.chunks(run).is_empty())
        });
        first.stop();
        // Long enough for the uploader to meet the closed port more than once.
        std::thread::sleep(std::time::Duration::from_millis(600));
        let back = UploadServer::start(
            &store,
            ServerOptions {
                tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
                port: Some(port),
                ..ServerOptions::default()
            },
        );
        end_harness(&rig);
        let (result, out) = running.join().unwrap();
        (result, out, back)
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("uploaded: "), "{out}");
    let run_id = server.run_ids()[0].clone();
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
}

/// The server is away at the end: `goalcup start` says the upload is incomplete, and a later
/// `goalcup upload` asks the server where it is and finishes, sealing it.
#[test]
fn an_upload_left_incomplete_is_finished_by_goalcup_upload() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let first = server(&store);
    let port = first.port();
    log_in(&rig, &first.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    // A wrap-up the closed server outlasts.
    opts.upload_policy.wrap_up = std::time::Duration::from_millis(300);
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        wait_until("a first chunk", || {
            first
                .run_ids()
                .first()
                .is_some_and(|run| !first.chunks(run).is_empty())
        });
        first.stop();
        end_harness(&rig);
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("upload incomplete"), "{out}");
    assert!(
        out.contains(&format!("goalcup upload {}", outcome.entry_id)),
        "{out}"
    );

    let server = UploadServer::start(
        &store,
        ServerOptions {
            tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
            port: Some(port),
            ..ServerOptions::default()
        },
    );
    let run_id = server.run_ids()[0].clone();
    assert_ne!(server.status(&run_id).as_deref(), Some("sealed"));
    let mut said = Vec::new();
    goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut said,
    )
    .unwrap();
    let said = String::from_utf8(said).unwrap();
    assert!(said.contains("and its seal"), "{said}");
    assert!(
        server
            .calls()
            .iter()
            .any(|c| c.starts_with("GET /v1/runs/")),
        "the upload began by asking the server where it is"
    );
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
    // And again: nothing is sent twice.
    let chunks = server.chunks(&run_id).len();
    goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(server.chunks(&run_id).len(), chunks);
}

/// A reply lost after the server stored a chunk: the retry gets the stored acknowledgement,
/// the chunk is stored once, and the acknowledgement is kept once.
#[test]
fn a_lost_reply_is_retried_without_a_second_copy() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url(), TOKEN);
    server.drop_next_chunk_reply();
    server.fail_next_chunks(1);
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let opts = logged_in_options(&rig, &goal);
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("uploaded: "), "{out}");
    let run_id = server.run_ids()[0].clone();
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
}

/// A token revoked after the run opened stops the upload, says why, and leaves the run
/// complete on this machine; a local chain that does not continue the server's copy is
/// refused, not uploaded.
#[test]
fn a_revoked_token_or_a_diverged_chain_stops_the_upload() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let first = server(&store);
    let port = first.port();
    log_in(&rig, &first.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    opts.upload_policy.wrap_up = std::time::Duration::from_millis(300);
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        wait_until("a first chunk", || {
            first
                .run_ids()
                .first()
                .is_some_and(|run| !first.chunks(run).is_empty())
        });
        first.stop();
        end_harness(&rig);
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));

    // The token is no longer known: refused, and the token is not repeated.
    let server = UploadServer::start(
        &store,
        ServerOptions {
            tokens: vec![(
                "gct_some_other_token_000000".to_string(),
                "member-1".to_string(),
            )],
            port: Some(port),
            ..ServerOptions::default()
        },
    );
    let refused = goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(refused.exit_code(), 1, "{refused}");
    assert!(refused.to_string().contains("401"), "{refused}");
    assert!(!refused.to_string().contains(TOKEN));
    server.stop();

    // Back with the token, but the local chain was edited past what the server holds.
    let server = UploadServer::start(
        &store,
        ServerOptions {
            tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
            port: Some(port),
            ..ServerOptions::default()
        },
    );
    let run_id = server.run_ids()[0].clone();
    let held = server_copy(&server, &run_id).len();
    let chain_path = rig.trace(&outcome.entry_id).join("chain.jsonl");
    let chain = std::fs::read(&chain_path).unwrap();
    let (kept, rest) = chain.split_at(held);
    let rest = String::from_utf8(rest.to_vec()).unwrap();
    let next: serde_json::Value = serde_json::from_str(rest.lines().next().unwrap()).unwrap();
    let prev = next["prev"].as_str().unwrap();
    let edited = rest.replacen(
        &format!("\"prev\":\"{prev}\""),
        &format!("\"prev\":\"{}\"", "1".repeat(64)),
        1,
    );
    assert_ne!(edited, rest, "the next record's link was found");
    std::fs::write(&chain_path, [kept, edited.as_bytes()].concat()).unwrap();
    let refused = goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        refused.to_string().contains("does not continue"),
        "{refused}"
    );
    assert_eq!(
        server_copy(&server, &run_id).len(),
        held,
        "nothing more was stored"
    );
}

// ------------------------------------------------------------------ the deadline

/// A server whose runs last `seconds`.
fn short_server(store: &std::path::Path, seconds: u64, port: Option<u16>) -> UploadServer {
    UploadServer::start(
        store,
        ServerOptions {
            tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
            run_seconds: Some(seconds),
            port,
            ..ServerOptions::default()
        },
    )
}

/// [`quick_policy`] with a heartbeat every 100 ms.
fn beating_policy() -> goalcup_cli::upload::uploader::Policy {
    goalcup_cli::upload::uploader::Policy::from_test_spec("200,25,100,400,10000,268435456,3000,100")
        .unwrap()
}

/// At the deadline the server says stop: the tracer stops the agent there and then (the fake
/// harness would otherwise sleep a minute), seals, commits the chain's end in a final
/// heartbeat, and uploads up to it.
#[test]
fn the_server_stops_the_run_at_its_deadline() {
    let rig = Rig::new();
    // Long enough for the fake harness to start and the agent to commit under a loaded test
    // machine; short enough for a test.
    let server = short_server(&rig.root.join("server"), 8, None);
    log_in(&rig, &server.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    opts.upload_policy = beating_policy();
    let started = std::time::Instant::now();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        // The agent commits in its workspace while the run records.
        wait_until("the harness to start", || {
            rig.harness_pid().is_some() || running.is_finished()
        });
        if running.is_finished() {
            let (result, out) = running.join().unwrap();
            panic!("start ended before the harness ran: {result:?}\n{out}");
        }
        let workspace = std::path::PathBuf::from(&client.book().creates[0].workspace);
        let git = workspace.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), format!("{commit}\n")).unwrap();
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "the run was not stopped at its deadline:\n{out}"
    );
    assert!(
        out.contains("deadline: the server says the run is over"),
        "{out}"
    );
    assert!(
        rig.harness_file("done").is_none(),
        "the harness ended on its own"
    );
    let run_id = server.run_ids()[0].clone();
    assert!(!server.heartbeats(&run_id).is_empty());
    let committed = server.final_commit(&run_id).expect("a final heartbeat");
    assert_eq!(
        committed.repo_head.as_deref(),
        Some(commit),
        "the final heartbeat names the workspace's commit as the run stopped"
    );
    let chain = std::fs::read(rig.trace(&outcome.entry_id).join("chain.jsonl")).unwrap();
    let last = chain
        .split(|b| *b == b'\n')
        .rfind(|l| !l.is_empty())
        .unwrap();
    let last: serde_json::Value = serde_json::from_slice(last).unwrap();
    assert_eq!(committed.seq, last["seq"].as_u64().unwrap());
    assert_eq!(committed.head, last["hash"].as_str().unwrap());
    assert!(out.contains("uploaded: "), "{out}");
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
}

/// With the server out of reach, the tracer stops on its own count of the run's length; the
/// final heartbeat and the rest go later with `goalcup upload`, and the server takes them up to
/// the committed end although its deadline has passed.
#[test]
fn without_a_server_the_run_stops_on_its_own_clock_and_is_finished_later() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let first = short_server(&store, 3, None);
    let port = first.port();
    log_in(&rig, &first.url(), TOKEN);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    opts.upload_policy = beating_policy();
    opts.upload_policy.wrap_up = std::time::Duration::from_millis(300);
    let (result, out) = std::thread::scope(|s| {
        let running = rig.start(s, &client, opts, None);
        wait_until("a first chunk", || {
            first
                .run_ids()
                .first()
                .is_some_and(|run| !first.chunks(run).is_empty())
        });
        first.stop();
        running.join().unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("by this machine's clock"), "{out}");
    assert!(out.contains("upload incomplete"), "{out}");

    let server = short_server(&store, 3, Some(port));
    let run_id = server.run_ids()[0].clone();
    assert!(server.final_commit(&run_id).is_none());
    goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(server.final_commit(&run_id).is_some());
    assert_fully_uploaded(&rig, &server, &run_id, &outcome.entry_id);
}
