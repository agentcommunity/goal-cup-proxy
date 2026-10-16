//! `goalcup start` against the local stand-in of the upload server: a real in-process daemon,
//! the fake harness, and the server on loopback (docs/UPLOAD-API.md).

use crate::common::m2_cli_support::{Hooked, Rig};
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_verifier::capture_analysis::reader::{CaptureIntegrity, scan_capture_trace};

const GOAL: &str = "write hello.txt, then stop";
const RESERVE: u64 = 4 * 1024 * 1024;
const REPO: &str = crate::common::m2_cli_support::PREP_REPO;

fn server(store: &std::path::Path) -> UploadServer {
    UploadServer::start(store, ServerOptions::default())
}

/// The participant has logged in to `url` and run `goalcup prep` (`Rig::prepare_for`); each
/// start then has its run approved by the stand-in, which approves at once.
fn log_in(rig: &Rig, url: &str) {
    rig.prepare_for(url);
}

/// The credential a run saved in its own directory, for a test to hand the stand-in. Reading
/// the file is how a test gets at it: the tracer's own type never gives the value out.
fn saved_credential(rig: &Rig, run_id: &str) -> String {
    let path = goalcup_cli::account::credential::credential_path(&goalcup_cli::run::run_dir(
        &rig.cfg, run_id,
    ));
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    stored["credential"].as_str().unwrap().to_string()
}

/// A logged-in start opens the run before anything is created, runs under the server's entry
/// id, and keeps the signed start receipt: the sealed trace verifies with a signed anchor.
#[test]
fn a_logged_in_start_runs_under_the_servers_entry_and_keeps_its_receipt() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let server = server(&store);
    log_in(&rig, &server.url());
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let opts = rig.options(&goal, RESERVE);
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

/// An approval the participant denies, or lets expire, starts nothing: no run on the server, no
/// entry, no run directory, no clone, no credential.
#[test]
fn a_denied_or_expired_approval_starts_nothing() {
    use crate::common::upload_server::Approval;
    for (approval, said) in [(Approval::Deny, "denied"), (Approval::Expire, "expired")] {
        let rig = Rig::new();
        let server = server(&rig.root.join("server"));
        log_in(&rig, &server.url());
        server.set_approval(approval);
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let error = result.expect_err("the start was not refused");
        assert_eq!(error.exit_code(), 1, "{error}");
        assert!(error.to_string().contains(said), "{error}");
        assert!(error.to_string().contains("Nothing was created"), "{error}");
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(rig.clones().is_empty(), "{:?}", rig.clones());
        assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
        assert!(!rig.harness_started());
        assert!(server.run_ids().is_empty());
    }
}

/// No server answering refuses the start the same way, before anything is created.
#[test]
fn an_unreachable_server_starts_nothing() {
    let rig = Rig::new();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    log_in(&rig, &url);
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let opts = rig.options(&goal, RESERVE);
    let (result, out) = std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
    let error = result.expect_err("the start was not refused");
    assert_eq!(error.exit_code(), 1, "{error}");
    let said = error.to_string();
    assert!(said.contains("could not be reached"), "{said}");
    assert!(said.contains("Nothing was created"), "{said}");
    assert!(client.book().creates.is_empty(), "{out}");
    assert!(rig.run_dirs().is_empty());
    assert!(rig.clones().is_empty());
}

/// A start that fails after the run opened and before the launch gives the run back, so the
/// account is not left holding an open run with nothing in it.
#[test]
fn a_start_that_fails_before_the_launch_gives_the_run_back() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    // The proxy cannot write its CA file, after the daemon made the entry.
    client.book().occupy_ca_file = true;
    let opts = rig.options(&goal, RESERVE);
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
/// roots, before any request is sent, as a failure that asking again
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
    let url = format!("https://localhost:{}", upstream.port);
    let root =
        rustls::pki_types::CertificateDer::from_pem_slice(upstream.root_pem().as_bytes()).unwrap();
    let trusted = ServerClient::with_roots(&url, [root]);
    let answer: serde_json::Value = trusted
        .json("GET", "/v1/runs/r0000000000000000", None::<&()>)
        .unwrap();
    assert_eq!(answer, serde_json::json!({}));

    let public = ServerClient::anonymous(&url);
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
    log_in(&rig, &server.url());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().registration = Some(crate::common::m2_cli_support::RegistrationFault::Refused);
    let opts = rig.options(&goal, RESERVE);
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

/// A start killed between the open and the launch cannot give its run back; it leaves a note and
/// the credential in the run's directory, and the next start gives that run back first instead
/// of meeting "one open run" refused.
#[test]
fn a_run_left_open_by_a_killed_start_is_given_back_by_the_next() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url());
    // What the killed start had: an approved credential, a run opened with it, and the note and
    // the credential it kept.
    let secret = server.mint_credential("member-1");
    let credential = goalcup_cli::account::credential::RunCredential::new(
        secret,
        "2099-01-01T00:00:00Z".to_string(),
    )
    .unwrap();
    let client =
        goalcup_cli::upload::http::ServerClient::with_credential(&server.url(), &credential);
    let request = goalcup_cli::upload::api::OpenRunRequest {
        connector_id: "m2-fake".into(),
        manifest_sha256: "a".repeat(64),
        connector_dir_sha256: "b".repeat(64),
        workspace: "/nowhere/workspace".into(),
        session_id: "00000000-0000-4000-8000-000000000001".into(),
        harness_version: "1".into(),
        tracer_version: "0.1.0".into(),
        repo_url: REPO.into(),
        machine: None,
    };
    let killed = goalcup_cli::upload::open_run(&client, &request).unwrap();
    goalcup_cli::account::credential::store(
        &goalcup_cli::run::run_dir(&rig.cfg, &killed.run_id),
        &credential,
    )
    .unwrap();
    std::mem::forget(goalcup_cli::upload::GiveBack::opened(&client, &rig.cfg, &killed).unwrap());

    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let hooked = Hooked::new(&http, &rig);
    let opts = rig.options(&goal, RESERVE);
    let (result, out) = std::thread::scope(|s| rig.start(s, &hooked, opts, None).join().unwrap());
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(
        server.status(&killed.run_id).as_deref(),
        Some("abandoned"),
        "{out}"
    );
    assert_eq!(server.run_ids().len(), 2, "{out}");
    assert!(!goalcup_cli::upload::pending_path(&rig.cfg).exists());
    assert!(
        !goalcup_cli::run::run_dir(&rig.cfg, &killed.run_id).exists(),
        "the given-back run's directory and credential are gone"
    );
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
    opts.upload_policy = quick_policy();
    opts
}

/// A normal run: the first chunk goes at the launch, more follow, and at the end the rest and
/// the seal; everything is acknowledged.
#[test]
fn a_run_is_uploaded_as_it_records_and_sealed_at_the_end() {
    let rig = Rig::new();
    let server = server(&rig.root.join("server"));
    log_in(&rig, &server.url());
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
    log_in(&rig, &first.url());
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
    log_in(&rig, &first.url());
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
    log_in(&rig, &server.url());
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

/// A credential that ended after the run opened stops the upload, says why, and leaves the run
/// complete on this machine; a local chain that does not continue the server's copy is
/// refused, not uploaded.
#[test]
fn an_ended_credential_or_a_diverged_chain_stops_the_upload() {
    let rig = Rig::new();
    let store = rig.root.join("server");
    let first = server(&store);
    let port = first.port();
    log_in(&rig, &first.url());
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

    // Back up, but the local chain was edited past what the server holds.
    let server = UploadServer::start(
        &store,
        ServerOptions {
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

    // The credential has ended: refused, and the credential is not repeated.
    let credential = saved_credential(&rig, &run_id);
    server.expire_credential(&credential);
    let refused = goalcup_cli::commands::upload::upload_with(
        &rig.cfg,
        &outcome.entry_id,
        quick_policy(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(refused.exit_code(), 1, "{refused}");
    assert!(refused.to_string().contains("401"), "{refused}");
    assert!(!refused.to_string().contains(&credential));
}

// ------------------------------------------------------------------ the final heartbeat

/// Make the workspace a Git repository with one commit, as an agent would, and answer the
/// commit. A real one: the seal asks `git` about the workspace.
fn commit_in(workspace: &std::path::Path) -> String {
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
            .args(args)
            .current_dir(workspace)
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "the agent's commit"]);
    git(&["rev-parse", "HEAD"]).trim().to_string()
}

/// Whether the agent left on its own or the run was stopped at its deadline, the upload's finish
/// sends the final heartbeat, with the workspace's commit and what the heartbeats say, before it
/// sends the server the seal; the stand-in refuses a seal that comes first.
#[test]
fn the_final_heartbeat_with_the_commit_reaches_the_server_before_the_server_seal() {
    for stopped_by_the_deadline in [false, true] {
        let rig = Rig::new();
        let server = if stopped_by_the_deadline {
            short_server(&rig.root.join("server"), 8, None)
        } else {
            server(&rig.root.join("server"))
        };
        log_in(&rig, &server.url());
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let mut opts = logged_in_options(&rig, &goal);
        if stopped_by_the_deadline {
            opts.upload_policy = beating_policy();
        }
        let (commit, result, out) = std::thread::scope(|s| {
            let running = rig.start(s, &client, opts, None);
            wait_until("the harness to start", || {
                rig.harness_pid().is_some() || running.is_finished()
            });
            let workspace = std::path::PathBuf::from(&client.book().creates[0].workspace);
            let commit = commit_in(&workspace);
            if !stopped_by_the_deadline {
                end_harness(&rig);
            }
            let (result, out) = running.join().unwrap();
            (commit, result, out)
        });
        result.unwrap_or_else(|e| panic!("deadline: {stopped_by_the_deadline}: {e}\n{out}"));
        let run_id = server.run_ids()[0].clone();
        let committed = server.final_commit(&run_id).expect("a final heartbeat");
        assert_eq!(
            committed.repo_head.as_deref(),
            Some(commit.as_str()),
            "deadline: {stopped_by_the_deadline}"
        );
        let last_final = server
            .heartbeats(&run_id)
            .into_iter()
            .rfind(|h| h.final_)
            .expect("a final heartbeat");
        assert!(
            last_final.recording.is_some() && last_final.model_requests.is_some(),
            "the final heartbeat says how the recording was and how many requests it saw"
        );
        let calls = server.calls();
        let at = |what: &str| calls.iter().rposition(|c| c.contains(what));
        let seal = at("/seal").unwrap_or_else(|| panic!("no seal in {calls:?}"));
        let heartbeat = at("/heartbeat").expect("a heartbeat");
        assert!(
            heartbeat < seal,
            "deadline: {stopped_by_the_deadline}: the last heartbeat came after the seal: {calls:?}"
        );
    }
}

// ------------------------------------------------------------------ the deadline

/// A server whose runs last `seconds`.
fn short_server(store: &std::path::Path, seconds: u64, port: Option<u16>) -> UploadServer {
    UploadServer::start(
        store,
        ServerOptions {
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
    log_in(&rig, &server.url());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    opts.upload_policy = beating_policy();
    let started = std::time::Instant::now();
    let (commit, result, out) = std::thread::scope(|s| {
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
        let commit = commit_in(&workspace);
        let (result, out) = running.join().unwrap();
        (commit, result, out)
    });
    let commit = commit.as_str();
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

/// At the deadline the harness can still hold a connection open through the proxy (a model's
/// long streaming answer, a kept-alive WebSocket), and that connection does not end while the
/// harness runs. The stop does not wait for it: the harness is asked to stop within seconds of
/// the deadline, not after the proxy's whole drain share (a Codex chat run was asked 176 s late).
#[test]
fn a_connection_held_open_at_the_deadline_does_not_delay_the_stop() {
    use goalcup_protocol::capture::{
        CaptureRecord, CaptureSource, ConnectionEvent, ConnectionOutcome,
    };
    let rig = Rig::new();
    let upstream = crate::common::m2_cli_support::Upstream::start_holding(&rig.runtime);
    rig.tell("steps.sh", &upstream.curl_waiting(r#"{"n":1}"#));
    let server = short_server(&rig.root.join("server"), 8, None);
    log_in(&rig, &server.url());
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let mut opts = logged_in_options(&rig, &goal);
    opts.upload_policy = beating_policy();
    let (result, out) = std::thread::scope(|s| {
        rig.start(s, &client, opts, Some(upstream.roots()))
            .join()
            .unwrap()
    });
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(out.contains("deadline: "), "{out}");
    // The request went out through the proxy and was never answered: it was open at the stop.
    let (_, records) = rig.read_trace(&outcome.entry_id);
    let opened = |wanted: CaptureSource| {
        records
            .iter()
            .any(|r| matches!(r, CaptureRecord::Open { source, .. } if *source == wanted))
    };
    assert!(
        opened(CaptureSource::WireRequest),
        "no request was held open\n{out}"
    );
    assert!(
        !opened(CaptureSource::WireResponse),
        "the request was answered\n{out}"
    );
    // The cut is honest: the held connection is recorded closed `Interrupted`, not complete.
    assert!(
        records.iter().any(|r| matches!(
            r,
            CaptureRecord::Connection {
                event: ConnectionEvent::Close {
                    outcome: ConnectionOutcome::Interrupted
                },
                ..
            }
        )),
        "the held connection was not recorded cut\n{out}"
    );
    // Both whole seconds of UTC: the run's deadline, and when the daemon asked the harness to stop.
    let at = |after: &str| {
        let from = out
            .find(after)
            .unwrap_or_else(|| panic!("no `{after}` in\n{out}"))
            + after.len();
        goalcup_protocol::control::parse_z(&out[from..from + 20])
            .unwrap_or_else(|| panic!("no time after `{after}` in\n{out}"))
    };
    let deadline = at(", deadline ");
    let asked = at("the harness was asked to stop at ");
    // The test's finish budget is 10 s: a stop that waited for the connection would be asked
    // its drain share (8 s) after the deadline; a prompt one about 3 s after it.
    assert!(
        asked <= deadline + 6,
        "the harness was asked to stop {} s after the deadline\n{out}",
        asked.saturating_sub(deadline)
    );
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
    log_in(&rig, &first.url());
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
    assert!(out.contains("by the server's count"), "{out}");
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

// ------------------------------------------------------------------ segments and `continue`

/// A run in chat mode on a terminal, its agent ending before the run's end, and `goalcup
/// continue` restarting it as the next segment: the binaries, a terminal each, the stand-in.
mod segments {
    use super::{REPO, wait_until};
    use crate::common::m2_cli_support::Rig;
    use crate::common::upload_server::{ServerOptions, UploadServer};
    use goalcup_cli::run::reserve::{artifact_root, held_by_another, reserve_path};
    use goalcup_protocol::capture_status::AnchorState;
    use goalcup_verifier::capture_analysis::reader::{CaptureIntegrity, scan_capture_trace};
    use std::path::{Path, PathBuf};

    /// A test build's disk sizes: the reservation, no margin, a 50 ms watch.
    const DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "4194304,0,50");
    /// The uploader's cadence a test can watch, with a heartbeat every 100 ms.
    const BEATING: (&str, &str) = (
        "GOALCUP_TEST_UPLOAD_POLICY",
        "200,25,100,400,10000,268435456,3000,100",
    );

    fn server(rig: &Rig, options: ServerOptions) -> UploadServer {
        let server = UploadServer::start(&rig.root.join("server"), options);
        rig.prepare_for(&server.url());
        server
    }

    fn harness(rig: &Rig, name: &str) -> String {
        rig.harness_file(name)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// End the fake agent the way a crash does.
    fn kill_agent(rig: &Rig) {
        let pid: i32 = harness(rig, "tui.pid").parse().expect("the agent's pid");
        // SAFETY: two integers; the pid is the fake harness's own.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    }

    fn the_run(server: &UploadServer) -> String {
        let runs = server.run_ids();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }

    /// The run as the server's API answers for it, with the credential the run saved.
    fn api_run(rig: &Rig, server: &UploadServer, run_id: &str) -> serde_json::Value {
        let dir = goalcup_cli::run::run_dir(&rig.cfg, run_id);
        let credential = goalcup_cli::account::credential::load(&dir).unwrap();
        goalcup_cli::upload::http::ServerClient::with_credential(&server.url(), &credential)
            .json("GET", &format!("/v1/runs/{run_id}"), None::<&()>)
            .unwrap()
    }

    fn reserve_dir(rig: &Rig) -> PathBuf {
        artifact_root(&rig.cfg).join("reserve")
    }

    fn reserve_files(rig: &Rig) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(reserve_dir(rig))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        files.sort();
        files
    }

    /// The server's copy of an entry's chain.
    fn copy_of(server: &UploadServer, entry_id: &str) -> Vec<u8> {
        let mut chunks = server.chunks_of(entry_id);
        chunks.sort_by_key(|c| c.offset);
        let mut joined = Vec::new();
        for chunk in chunks {
            assert_eq!(chunk.offset, joined.len() as u64, "chunks leave no hole");
            let raw =
                zstd::decode_all(std::fs::read(&chunk.body_path).unwrap().as_slice()).unwrap();
            joined.extend_from_slice(&raw);
        }
        joined
    }

    /// The server holds all of the entry's chain and its seal, and the trace verifies under the
    /// server's key with a signed anchor.
    fn assert_entry_uploaded(rig: &Rig, server: &UploadServer, entry_id: &str) {
        assert_entry_sent(rig, server, entry_id);
        let report = scan(rig, server, entry_id);
        assert_eq!(report.integrity, CaptureIntegrity::Valid, "{report:?}");
    }

    fn scan(
        rig: &Rig,
        server: &UploadServer,
        entry_id: &str,
    ) -> goalcup_verifier::capture_analysis::reader::CaptureIntegrityReport {
        let options = goalcup_verifier::VerifyOptions {
            extra_server_keys: vec![server.public_key_hex()],
            ..Default::default()
        };
        scan_capture_trace(&rig.trace(entry_id), &options, &mut |_| Ok(())).unwrap()
    }

    /// [`assert_entry_uploaded`] without asking the recording to have been whole: a segment
    /// whose host was killed has a gap, and is still sealed, sent and signed.
    fn assert_entry_sent(rig: &Rig, server: &UploadServer, entry_id: &str) {
        let trace = rig.trace(entry_id);
        let chain = std::fs::read(trace.join("chain.jsonl")).unwrap();
        assert_eq!(copy_of(server, entry_id), chain, "{entry_id}: the chain");
        let (seal, baseline) = server
            .seal_of(entry_id)
            .unwrap_or_else(|| panic!("{entry_id} was not sealed on the server"));
        assert_eq!(
            seal.as_bytes(),
            std::fs::read(trace.join("seal.json")).unwrap()
        );
        assert_eq!(
            baseline.as_bytes(),
            std::fs::read(trace.join("baseline.json")).unwrap()
        );
        assert!(
            server.heartbeats_of(entry_id).iter().any(|h| h.final_),
            "{entry_id}: a final heartbeat"
        );
        let report = scan(rig, server, entry_id);
        assert_eq!(report.anchor, AnchorState::Signed, "{report:?}");
        assert!(report.sealed, "{report:?}");
    }

    fn lock_is_free(path: &Path) -> bool {
        use std::os::fd::AsRawFd;
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
        else {
            return true;
        };
        // SAFETY: `flock` on a descriptor this function owns.
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }

    /// The agent of a run is killed: the host seals the segment, sends its final heartbeat and
    /// uploads it, says the run stays open, and exits 0. `goalcup continue` then restarts the
    /// agent with no second approval, with the harness's resume arguments, as segment 1 of the
    /// same run; both segments end up on the server, sealed.
    #[test]
    fn a_killed_agent_leaves_the_run_open_and_continue_makes_segment_one() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        kill_agent(&rig);
        let status = first.wait();
        let shown = first.shown();
        assert!(status.success(), "{status:?}\n{shown}");

        let run_id = the_run(&server);
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        let deadline_at = record.deadline_at.clone().unwrap();
        // The facts the participant needs: how the agent ended, that the run is open and until
        // when (in their local time), and the command that goes on, with the run's id.
        let until = goalcup_cli::participant::local_clock(
            goalcup_protocol::control::parse_z(&deadline_at).unwrap(),
        );
        // Before the harness opened: the approval, then what to type and when the run ends.
        let ready = shown
            .find(goalcup_cli::participant::READY_MARK)
            .unwrap_or_else(|| panic!("{shown}"));
        let before = &shown[..ready];
        assert!(
            before.contains(goalcup_cli::participant::APPROVED),
            "{shown}"
        );
        assert!(
            before.contains(goalcup_cli::participant::KEEP_GOING),
            "{shown}"
        );
        assert!(before.contains(&until), "{shown}");
        assert!(shown.contains("(ended by signal 9)"), "{shown}");
        assert!(
            shown.contains(&format!("{} {until}", goalcup_cli::participant::STILL_OPEN)),
            "{shown}"
        );
        let hint = shown
            .find(&format!(
                "{} {run_id}",
                goalcup_cli::participant::CONTINUE_HINT
            ))
            .unwrap_or_else(|| panic!("{shown}"));
        // Said at the seal, before the wait for the upload: continue works from then on.
        let waiting = shown
            .find(goalcup_cli::participant::UPLOADING_REST)
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(hint < waiting, "{shown}");
        assert!(
            shown.find(goalcup_cli::participant::STILL_OPEN).unwrap() < waiting,
            "{shown}"
        );
        // A chat-mode run's ids and folders are not on the terminal before the harness: they
        // are in its host.log.
        assert!(!before.contains("run directory:"), "{shown}");
        let log =
            std::fs::read_to_string(goalcup_cli::run::run_dir(&rig.cfg, &run_id).join("host.log"))
                .unwrap();
        assert!(log.contains("run directory:"), "{log}");
        assert!(log.contains(&format!("run {run_id} opened on")), "{log}");
        // The run is open on the server; its one segment is sealed and sent.
        assert_eq!(api_run(&rig, &server, &run_id)["status"], "open");
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 1);
        assert_entry_uploaded(&rig, &server, &entries[0]);
        assert_eq!(server.device_requests().len(), 1);
        assert_eq!(reserve_files(&rig).len(), 1, "the reserve stays");
        let workspace = record.workspace.clone();
        let first_pid = harness(&rig, "tui.pid");

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        assert_ne!(harness(&rig, "tui.pid"), first_pid, "a new agent");
        let argv: Vec<String> = harness(&rig, "tui.argv")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            argv,
            [
                "chat",
                "-C",
                &workspace.display().to_string(),
                "resume",
                "--last"
            ],
            "the profile's arguments, then its resume arguments, in the run's clone"
        );
        second.type_line("again");
        second.wait_shown("tui: noted again");
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert!(
            shown.contains(goalcup_cli::participant::STILL_OPEN),
            "{shown}"
        );
        // The continued launch said what to type, and when the run ends.
        let before = &shown[..shown
            .find(goalcup_cli::participant::READY_MARK)
            .unwrap_or_else(|| panic!("{shown}"))];
        assert!(
            before.contains(goalcup_cli::participant::KEEP_GOING),
            "{shown}"
        );
        assert!(before.contains(&until), "{shown}");

        // No second approval, no second clone; one run with two segments, both sent.
        assert_eq!(server.device_requests().len(), 1);
        assert_eq!(server.run_ids(), std::slice::from_ref(&run_id));
        assert_eq!(rig.clones().len(), 1);
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        for entry in &entries {
            assert_entry_uploaded(&rig, &server, entry);
        }
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        assert_eq!(
            record
                .entries
                .iter()
                .map(|e| e.entry_id.clone())
                .collect::<Vec<_>>(),
            entries
        );
        assert_eq!(
            record.workspace.display().to_string(),
            workspace.display().to_string()
        );
        assert_eq!(api_run(&rig, &server, &run_id)["status"], "open");
        let (report, _) = rig.read_trace(&entries[1]);
        assert!(report.sealed, "{:?}", report.findings);
    }

    /// The seal is where the shared harness home is let go: the sentence is printed after it,
    /// and while the upload is still being retried (the server is gone) the next segment, or any
    /// other run, may take the home.
    #[test]
    fn the_shared_home_is_let_go_at_the_seal_not_at_the_end_of_the_upload() {
        let rig = Rig::chat();
        let first_server = server(&rig, ServerOptions::default());
        let mut run = rig.start_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        let lock = rig
            .cfg
            .join("capture-homes")
            .join(format!("{}.lock", crate::common::m2_cli_support::CHAT_ID));
        assert!(
            !lock_is_free(&lock),
            "the run holds the home while it records"
        );
        first_server.stop();
        kill_agent(&rig);
        // Said before the wrap-up's upload, which cannot finish with the server gone.
        run.wait_shown(goalcup_cli::participant::UPLOADING_REST);
        assert!(
            matches!(run.child.try_wait(), Ok(None)),
            "the host is still sending its segment"
        );
        assert!(lock_is_free(&lock), "and the home is free");
        run.child.kill().unwrap();
        run.wait();
    }

    /// A segment that follows another reuses the run's one reserve file: it is not taken a
    /// second time and not replaced.
    #[test]
    fn a_continue_reuses_the_runs_reserve_file_and_never_takes_a_second() {
        use std::os::unix::fs::MetadataExt;
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success());
        let run_id = the_run(&server);
        let files = reserve_files(&rig);
        assert_eq!(files, [reserve_path(&artifact_root(&rig.cfg), &run_id)]);
        let identity = |path: &Path| {
            let meta = std::fs::metadata(path).unwrap();
            (meta.dev(), meta.ino())
        };
        let before = identity(&files[0]);

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        assert_eq!(
            reserve_files(&rig),
            files,
            "no second reserve while it runs"
        );
        assert_eq!(identity(&files[0]), before, "the same file");
        second.type_line("/exit");
        assert!(second.wait().success());
        assert_eq!(reserve_files(&rig), files);
        assert_eq!(identity(&files[0]), before);
    }

    /// A run that was over before `continue` is refused, and nothing is made for it: no entry
    /// on the server, none here, no clone, no run directory, no second agent.
    #[test]
    fn continue_after_the_deadline_is_refused_and_creates_nothing() {
        let rig = Rig::chat();
        let server = server(
            &rig,
            ServerOptions {
                run_seconds: Some(6),
                ..ServerOptions::default()
            },
        );
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success());
        let run_id = the_run(&server);
        let deadline = goalcup_protocol::control::parse_z(
            goalcup_cli::run::read(&rig.cfg, &run_id)
                .unwrap()
                .deadline_at
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        wait_until("the run's deadline", || {
            goalcup_cli::run::now_unix() > deadline
        });
        let uploads = || {
            let mut names: Vec<_> = std::fs::read_dir(rig.cfg.join("uploads"))
                .unwrap()
                .flatten()
                .map(|e| e.file_name())
                .collect();
            names.sort();
            names
        };
        let (agent, folders, kept) = (harness(&rig, "tui.pid"), rig.run_dirs(), uploads());

        let mut again = rig.continue_on_pty(&[DISK, BEATING], None);
        let status = again.wait();
        let said = again.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        assert!(said.contains("is over"), "{said}");
        assert!(said.contains(&run_id), "{said}");
        assert_eq!(server.entry_ids(&run_id).len(), 1, "no entry was made");
        assert_eq!(
            goalcup_cli::run::read(&rig.cfg, &run_id)
                .unwrap()
                .entries
                .len(),
            1
        );
        assert_eq!(rig.clones().len(), 1);
        assert_eq!(rig.run_dirs(), folders);
        assert_eq!(uploads(), kept);
        assert_eq!(harness(&rig, "tui.pid"), agent, "no second agent");
    }

    /// A run this machine did not start, or has no credential for, is refused alike, and nothing
    /// is started or asked of the daemon or the server.
    #[test]
    fn continue_of_a_run_not_started_here_is_refused() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let said =
            |output: &std::process::Output| String::from_utf8_lossy(&output.stderr).into_owned();
        // Named, and unknown here.
        let output = rig
            .continue_command(&[], Some("r0123456789abcdef"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{}", said(&output));
        assert!(
            said(&output).contains(goalcup_cli::participant::NOT_STARTED_HERE),
            "{}",
            said(&output)
        );
        // No run at all.
        let output = rig.continue_command(&[], None).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{}", said(&output));
        assert!(said(&output).contains(goalcup_cli::participant::NOT_STARTED_HERE));
        // A record without the run's credential (it was copied from another machine).
        let run_id = "r0123456789abcdef";
        goalcup_cli::run::write(
            &rig.cfg,
            &goalcup_cli::run::RunRecord {
                run_id: run_id.to_string(),
                server_url: Some(server.url()),
                practice: false,
                test: false,
                connector_id: crate::common::m2_cli_support::CHAT_ID.to_string(),
                workspace: rig.root.join("somewhere"),
                started_at_unix: 1,
                deadline_at: Some(goalcup_protocol::control::format_z(
                    goalcup_cli::run::now_unix() + 3600,
                )),
                entries: Vec::new(),
            },
        )
        .unwrap();
        let output = rig.continue_command(&[], Some(run_id)).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{}", said(&output));
        assert!(said(&output).contains(goalcup_cli::participant::NOT_STARTED_HERE));
        assert!(server.calls().is_empty(), "{:?}", server.calls());
        assert!(!rig.harness_started());
        assert!(rig.run_dirs().is_empty());
        assert!(rig.clones().is_empty());
    }

    /// While the run's agent is recording, `continue` is refused, by the run's name, and the
    /// running segment goes on undisturbed.
    #[test]
    fn continue_while_a_run_records_is_refused_by_name() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut run = rig.start_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        let run_id = the_run(&server);

        let mut refused = rig.continue_on_pty(&[DISK, BEATING], None);
        let status = refused.wait();
        let said = refused.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        assert!(
            said.contains(&format!("run {run_id} is still recording")),
            "{said}"
        );
        assert_eq!(server.entry_ids(&run_id).len(), 1, "no segment was made");

        run.type_line("/exit");
        assert!(run.wait().success());
        assert_eq!(harness(&rig, "tui.end"), "exit");
        assert_eq!(server.entry_ids(&run_id).len(), 1);
    }

    /// The host and the daemon die together (a reboot), and the chain's last line is cut. The
    /// daemon cannot reopen the entry, `continue` repairs nothing: it closes the segment from
    /// the last whole record of its chain, as an incomplete segment (no seal), sends it, and
    /// starts segment 1.
    #[test]
    fn continue_after_a_reboot_closes_an_unreopenable_segment_from_its_tail() {
        let mut rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        let run_id = the_run(&server);
        let entry = server.entry_ids(&run_id)[0].clone();
        wait_until("a chunk of the first segment", || {
            !server.chunks_of(&entry).is_empty()
        });
        // The machine goes down: the host, its agent and the daemon.
        // SAFETY: two integers; the pids are this test's own children.
        unsafe { libc::kill(first.child.id() as i32, libc::SIGKILL) };
        kill_agent(&rig);
        first.close();
        first.wait();
        let daemon = rig.daemon.take().expect("the rig's daemon");
        rig.runtime.block_on(daemon.stop());
        // The last line of the chain was being written.
        let trace = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap().entries[0]
            .trace_dir
            .clone();
        let last = goalcup_cli::upload::prune::parts_now(&trace)
            .unwrap()
            .pop()
            .expect("a chain part")
            .path;
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&last)
            .unwrap()
            .write_all(b"{\"torn\":\"a line cut short")
            .unwrap();
        let torn = std::fs::read(&last).unwrap();
        let whole = torn.len() - b"{\"torn\":\"a line cut short".len();
        // The machine is back, and its daemon with it.
        rig.daemon = Some(
            rig.runtime
                .block_on(goalcup_daemon::runtime::Daemon::start(rig.config.clone()))
                .unwrap(),
        );

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        second.type_line("back");
        second.wait_shown("tui: noted back");
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert!(shown.contains("incomplete segment"), "{shown}");

        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        // Nothing was repaired: the torn bytes are still the chain's last.
        assert_eq!(std::fs::read(&last).unwrap(), torn);
        // The first segment ends at its last whole record; it has no seal.
        let end = server
            .final_of(&entries[0])
            .expect("its end was committed in a final heartbeat");
        assert_eq!(
            copy_of(&server, &entries[0]),
            torn[..whole].to_vec(),
            "the server holds the chain up to the last whole line"
        );
        let last_line = torn[..whole]
            .split(|b| *b == b'\n')
            .rfind(|l| !l.is_empty())
            .unwrap();
        let last_line: serde_json::Value = serde_json::from_slice(last_line).unwrap();
        assert_eq!(end.seq, last_line["seq"].as_u64().unwrap());
        assert_eq!(end.head, last_line["hash"].as_str().unwrap());
        assert!(
            server.seal_of(&entries[0]).is_none(),
            "an incomplete segment"
        );
        // The new segment is whole.
        assert_entry_uploaded(&rig, &server, &entries[1]);
        assert_eq!(api_run(&rig, &server, &run_id)["status"], "open");
    }

    /// A second start on the account replaces the run that is recording: the stand-in answers the
    /// first host's next heartbeat with stop and `replaced`; the first host stops the agent,
    /// seals, sends everything, exits 0, and, the run being over, gives its reserve back.
    #[test]
    fn a_second_start_replaces_the_first_run() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        let run_id = the_run(&server);
        let entry = server.entry_ids(&run_id)[0].clone();
        assert_eq!(reserve_files(&rig).len(), 1);

        // The account starts another competition run (here, directly against the server).
        replace_run(&server, &run_id);

        let status = first.wait();
        let shown = first.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert_eq!(
            harness(&rig, "tui.end"),
            "",
            "the agent was ended by the seal"
        );
        assert_entry_uploaded(&rig, &server, &entry);
        let state = goalcup_cli::run::read_state(&goalcup_cli::run::run_dir(&rig.cfg, &run_id))
            .expect("the run's last state");
        assert_eq!(
            state.stopped.as_deref(),
            Some(goalcup_cli::upload::uploader::REPLACED_REASON)
        );
        assert!(state.ended);
        assert!(
            !shown.contains(goalcup_cli::participant::STILL_OPEN)
                && !shown.contains(&format!(
                    "{} {run_id}",
                    goalcup_cli::participant::CONTINUE_HINT
                )),
            "a replaced run is over, not open: {shown}"
        );
        assert!(
            !reserve_path(&artifact_root(&rig.cfg), &run_id).exists(),
            "a replaced run is over: its reserve goes back after the last acknowledgement"
        );
    }

    /// The account starts another competition run, directly against the server: it replaces
    /// `run_id`.
    fn replace_run(server: &UploadServer, run_id: &str) {
        let credential = goalcup_cli::account::credential::RunCredential::new(
            server.mint_credential("member-1"),
            "2099-01-01T00:00:00Z".to_string(),
        )
        .unwrap();
        let other =
            goalcup_cli::upload::http::ServerClient::with_credential(&server.url(), &credential);
        let second = goalcup_cli::upload::open_run(
            &other,
            &goalcup_cli::upload::api::OpenRunRequest {
                connector_id: "m2-fake".into(),
                manifest_sha256: "a".repeat(64),
                connector_dir_sha256: "b".repeat(64),
                workspace: "/nowhere/workspace".into(),
                session_id: "00000000-0000-4000-8000-000000000002".into(),
                harness_version: "1".into(),
                tracer_version: "0.1.0".into(),
                repo_url: REPO.into(),
                machine: None,
            },
        )
        .unwrap();
        assert_ne!(second.run_id, run_id);
        assert!(server.replaced_at(run_id).is_some());
    }

    /// The host and its agent die at once (the machine loses power) while the daemon, which
    /// keeps the entry open, goes on. Nothing is sealed, and nothing is sent.
    fn crashed_host(rig: &Rig, server: &UploadServer) -> (String, String) {
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        let run_id = the_run(server);
        let entry = server.entry_ids(&run_id)[0].clone();
        wait_until("a chunk of the first segment", || {
            !server.chunks_of(&entry).is_empty()
        });
        // SAFETY: two integers; the pid is this test's own child.
        unsafe { libc::kill(first.child.id() as i32, libc::SIGKILL) };
        kill_agent(rig);
        first.close();
        first.wait();
        (run_id, entry)
    }

    fn uploads_dir_names(rig: &Rig) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = std::fs::read_dir(rig.cfg.join("uploads"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        names.sort();
        names
    }

    /// Review finding 1. The first host stops managing the run's reserve and drops its lock at
    /// the segment's seal, keeping the file, so a `goalcup continue` started while that host is
    /// still sending takes the same file at once. The first host goes on and finishes its
    /// upload when the server stores chunks again.
    #[test]
    fn a_continue_during_the_first_hosts_wrap_up_takes_the_handed_over_reserve() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        first.type_line("hello");
        first.wait_shown("tui: noted hello");
        let run_id = the_run(&server);
        let entry = server.entry_ids(&run_id)[0].clone();
        wait_until("a chunk of the first segment", || {
            !server.chunks_of(&entry).is_empty()
        });
        // From here no chunk is stored: the first host cannot finish its upload.
        server.fail_next_chunks(1_000_000);
        first.type_line("more");
        first.wait_shown("tui: noted more");
        kill_agent(&rig);
        first.wait_shown(goalcup_cli::participant::UPLOADING_REST);
        assert!(
            matches!(first.child.try_wait(), Ok(None)),
            "the first host is still in its wrap-up"
        );
        let files = reserve_files(&rig);
        assert_eq!(files, [reserve_path(&artifact_root(&rig.cfg), &run_id)]);
        assert!(
            !held_by_another(&files[0]),
            "the reserve was handed over, not kept through the wrap-up"
        );

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        let shown = second.shown();
        assert!(!shown.contains("holding the reservation"), "{shown}");
        assert_eq!(reserve_files(&rig), files, "one reserve file for the run");
        assert!(held_by_another(&files[0]), "and the second segment has it");
        assert_eq!(server.entry_ids(&run_id).len(), 2);

        // The server stores chunks again: the first host finishes its own upload.
        server.fail_next_chunks(0);
        let status = first.wait();
        let first_shown = first.shown();
        assert!(status.success(), "{status:?}\n{first_shown}");
        assert_entry_uploaded(&rig, &server, &entry);
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        let entries = server.entry_ids(&run_id);
        assert_entry_uploaded(&rig, &server, &entries[1]);
        assert_eq!(reserve_files(&rig), files);
        // The first host's last word did not replace the live segment's state.
        let state = goalcup_cli::run::read_state(&goalcup_cli::run::run_dir(&rig.cfg, &run_id))
            .expect("the run's state");
        assert_eq!(state.entry_id, entries[1]);
    }

    /// Review finding 3. A headless connector, and a terminal that is not there, are refused
    /// before the shared home is locked and before anything is sealed or sent: the segment the
    /// daemon still holds open stays open, and the run can still be continued afterwards.
    #[test]
    fn continue_refusals_that_need_no_server_come_before_anything_is_closed() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let (run_id, entry) = crashed_host(&rig, &server);
        let kept = uploads_dir_names(&rig);
        let calls = server.calls();
        let untouched = |rig: &Rig| {
            assert!(
                !rig.read_trace(&entry).0.sealed,
                "the segment was sealed by a refused continue"
            );
            assert!(server.final_of(&entry).is_none(), "an end was committed");
            assert_eq!(uploads_dir_names(rig), kept);
            assert_eq!(server.calls(), calls, "the server was asked something");
            assert_eq!(server.entry_ids(&run_id).len(), 1);
        };

        // No terminal: the command's input is not one.
        let output = rig
            .continue_command(&[DISK, BEATING], None)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stderr).into_owned();
        assert_eq!(output.status.code(), Some(1), "{said}");
        assert!(said.contains("foreground of a terminal"), "{said}");
        untouched(&rig);

        // A headless connector has no session to pick up.
        let mut record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        record.connector_id = crate::common::m2_cli_support::FAKE_ID.to_string();
        goalcup_cli::run::write(&rig.cfg, &record).unwrap();
        let output = rig
            .continue_command(&[DISK, BEATING], None)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stderr).into_owned();
        assert_eq!(output.status.code(), Some(1), "{said}");
        assert!(said.contains("runs headless"), "{said}");
        untouched(&rig);
        record.connector_id = crate::common::m2_cli_support::CHAT_ID.to_string();
        goalcup_cli::run::write(&rig.cfg, &record).unwrap();

        // The run was not damaged: it continues, and the held segment is sealed then.
        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_entry_sent(&rig, &server, &entry);
        assert_entry_uploaded(&rig, &server, &entries[1]);
    }

    /// WSL1 cannot run Goal Cup, and WSL2 cannot keep a run on a Windows-host filesystem: a
    /// `continue` on such a machine is refused before the home is locked and before anything is
    /// closed, whichever folder it is that the filesystem holds.
    #[test]
    fn continue_refuses_wsl1_and_a_windows_host_filesystem_before_anything_is_closed() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let (run_id, entry) = crashed_host(&rig, &server);
        let kept = uploads_dir_names(&rig);
        let calls = server.calls();
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        let work = record.workspace.parent().unwrap().display().to_string();
        let artifacts = format!("{}-artifacts", rig.cfg.display());

        for (wsl, expected) in [
            ("wsl=1".to_string(), "WSL1 cannot run Goal Cup".to_string()),
            (format!("wsl=2,fs={work}=9p"), "work folder".to_string()),
            (
                format!("wsl=2,fs={}=drvfs", rig.cfg.display()),
                "config folder".to_string(),
            ),
            (
                format!("wsl=2,fs={artifacts}=virtiofs"),
                "artifact root".to_string(),
            ),
            (
                format!("wsl=2,fs={}=9p", rig.root.join("runs").display()),
                "run folder".to_string(),
            ),
        ] {
            let output = rig
                .continue_command(&[DISK, BEATING], None)
                .env(goalcup_cli::wsl_gate::Host::VAR, &wsl)
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&output.stderr).into_owned();
            assert_eq!(output.status.code(), Some(1), "{wsl}: {said}");
            assert!(said.contains(&expected), "{wsl}: {said}");
            assert!(said.contains("Nothing was changed."), "{wsl}: {said}");
            assert!(
                !rig.read_trace(&entry).0.sealed,
                "{wsl}: a segment was sealed"
            );
            assert_eq!(uploads_dir_names(&rig), kept, "{wsl}");
            assert_eq!(
                server.calls(),
                calls,
                "{wsl}: the server was asked something"
            );
            assert_eq!(server.entry_ids(&run_id).len(), 1, "{wsl}");
        }
    }

    /// Review finding 7. The server is away when `continue` asks for the next segment: the
    /// earlier segment is closed correctly on this machine, nothing else is created, and the
    /// command is refused. When the server is back, `continue` goes on from there.
    #[test]
    fn a_continue_with_the_server_away_creates_nothing_and_a_later_one_goes_on() {
        let rig = Rig::chat();
        let server_dir = rig.root.join("server");
        let first_server = UploadServer::start(&server_dir, ServerOptions::default());
        rig.prepare_for(&first_server.url());
        let port = first_server.port();
        let (run_id, entry) = crashed_host(&rig, &first_server);
        first_server.stop();
        let (folders, clones) = (rig.run_dirs(), rig.clones());

        let mut away = rig.continue_on_pty(&[DISK, BEATING], None);
        let status = away.wait();
        let said = away.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        assert!(said.contains("Nothing was started"), "{said}");
        assert!(said.contains("did not continue run"), "{said}");
        // Nothing was made for the segment that was not started.
        assert_eq!(rig.run_dirs(), folders);
        assert_eq!(rig.clones(), clones);
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        assert_eq!(record.entries.len(), 1, "no segment was recorded");
        // The earlier segment was closed on this machine: sealed.
        assert!(rig.read_trace(&entry).0.sealed, "{said}");

        // The server is back, at the same address, with what it held.
        let server = UploadServer::start(
            &server_dir,
            ServerOptions {
                port: Some(port),
                ..ServerOptions::default()
            },
        );
        let mut again = rig.continue_on_pty(&[DISK, BEATING], None);
        again.wait_shown("tui: ready");
        again.type_line("/exit");
        let status = again.wait();
        let shown = again.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_entry_sent(&rig, &server, &entries[0]);
        assert_entry_uploaded(&rig, &server, &entries[1]);
    }

    /// Review finding 7. A segment an earlier `continue` opened on the server and did not launch
    /// is followed once: the server answers 409 to the stale `previous_entry_id`, this machine
    /// asks where the run is, and its segment follows the one it does not know.
    #[test]
    fn a_segment_opened_but_never_launched_is_followed_once() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success());
        let run_id = the_run(&server);
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 1);
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();

        // Another continue got as far as the server and no further.
        let credential =
            goalcup_cli::account::credential::load(&goalcup_cli::run::run_dir(&rig.cfg, &run_id))
                .unwrap();
        let client =
            goalcup_cli::upload::http::ServerClient::with_credential(&server.url(), &credential);
        let orphan = goalcup_cli::upload::continue_run(
            &client,
            &run_id,
            &goalcup_cli::upload::api::ContinueRequest {
                previous_entry_id: entries[0].clone(),
                connector_id: crate::common::m2_cli_support::CHAT_ID.into(),
                manifest_sha256: "a".repeat(64),
                connector_dir_sha256: "b".repeat(64),
                workspace: record.workspace.display().to_string(),
                session_id: "00000000-0000-4000-8000-0000000000aa".into(),
                harness_version: "1".into(),
                tracer_version: "0.1.0".into(),
            },
        )
        .unwrap();
        let goalcup_cli::upload::Continued::Segment(orphan) = orphan else {
            panic!("the stand-in did not open a segment");
        };

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        let on_server = server.entry_ids(&run_id);
        assert_eq!(on_server.len(), 3, "{on_server:?}");
        assert_eq!(on_server[1], orphan.entry_id);
        // This machine knows its own two segments, and the segment it launched follows the
        // orphan on the server.
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        let known: Vec<_> = record.entries.iter().map(|e| e.entry_id.clone()).collect();
        assert_eq!(known, [on_server[0].clone(), on_server[2].clone()]);
        assert_entry_uploaded(&rig, &server, &on_server[2]);
        assert_eq!(api_run(&rig, &server, &run_id)["status"], "open");
    }

    /// Review finding 8. A segment whose upload finished (every byte, and its seal) is not
    /// closed and sent again by the next `continue`: no repeated final heartbeat, no repeated
    /// seal, and no line saying it is being sent.
    #[test]
    fn a_segment_already_sent_is_not_sent_again() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success());
        let run_id = the_run(&server);
        let entry = server.entry_ids(&run_id)[0].clone();
        assert_entry_uploaded(&rig, &server, &entry);
        let beats = server.heartbeats_of(&entry).len();
        let seals = |server: &UploadServer| {
            server
                .calls()
                .iter()
                .filter(|call| call.ends_with("/seal"))
                .count()
        };
        let sealed = seals(&server);

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        let shown = second.shown();
        assert!(
            !shown.contains(goalcup_cli::participant::EARLIER_PART_UPLOADING),
            "{shown}"
        );
        assert_eq!(server.heartbeats_of(&entry).len(), beats);
        assert_eq!(
            seals(&server),
            sealed,
            "no second seal of the first segment"
        );
        second.type_line("/exit");
        assert!(second.wait().success());
    }

    /// Hold a continued launch at `point`, interrupt it there (a Ctrl-C), and let it go on: the
    /// launch fails at its next look at the flag. Returns what `goalcup continue` said.
    fn continue_interrupted_at(rig: &Rig, point: &str) -> String {
        let pause = rig.root.join(format!("pause-{point}"));
        std::fs::create_dir_all(&pause).unwrap();
        std::fs::write(pause.join(format!("{point}.arm")), b"").unwrap();
        let mut held = rig.continue_on_pty(
            &[
                DISK,
                BEATING,
                ("GOALCUP_TEST_PAUSE_DIR", pause.to_str().unwrap()),
            ],
            None,
        );
        wait_until("the launch to reach its pause", || {
            pause.join(format!("{point}.reached")).exists()
        });
        held.signal_foreground(libc::SIGINT);
        std::fs::write(pause.join(format!("{point}.go")), b"").unwrap();
        let status = held.wait();
        let said = held.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        said
    }

    /// Every file of a clone, with its bytes, and its checked-out commit.
    fn clone_contents(rig: &Rig) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
            names.sort_by_key(|e| e.file_name());
            for entry in names {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push((path.clone(), std::fs::read(&path).unwrap_or_default()));
                }
            }
        }
        let clones = rig.clones();
        assert_eq!(clones.len(), 1, "{clones:?}");
        let mut files = Vec::new();
        walk(&clones[0], &mut files);
        assert!(!files.is_empty(), "the clone has files");
        files
    }

    /// Review round 2, findings 1 and 3. A continued launch that fails after the server made its
    /// segment (here before the fork, and after it) leaves the run's clone with every file it
    /// had, and the run's one reserve file where it was; the run then continues.
    #[test]
    fn a_continued_launch_that_fails_leaves_the_clone_and_the_reserve() {
        use std::os::unix::fs::MetadataExt;
        for point in ["capture-shared-home-pre-create", "capture-gate-forked"] {
            let rig = Rig::chat();
            let server = server(&rig, ServerOptions::default());
            let mut first = rig.start_on_pty(&[DISK, BEATING]);
            first.wait_shown("tui: ready");
            kill_agent(&rig);
            assert!(first.wait().success());
            let run_id = the_run(&server);
            let (files, reserves) = (clone_contents(&rig), reserve_files(&rig));
            assert_eq!(reserves.len(), 1);
            let identity = std::fs::metadata(&reserves[0]).map(|m| (m.dev(), m.ino()));

            let said = continue_interrupted_at(&rig, point);
            assert!(said.contains("interrupted"), "{point}: {said}");
            assert_eq!(clone_contents(&rig), files, "{point}: the clone");
            assert_eq!(reserve_files(&rig), reserves, "{point}: the reserve");
            assert_eq!(
                std::fs::metadata(&reserves[0])
                    .map(|m| (m.dev(), m.ino()))
                    .ok(),
                identity.ok(),
                "{point}: the same reserve file"
            );
            assert!(
                !held_by_another(&reserves[0]),
                "{point}: and nobody holds it"
            );
            assert_eq!(api_run(&rig, &server, &run_id)["status"], "open");

            // The run was not damaged: it continues.
            let mut again = rig.continue_on_pty(&[DISK, BEATING], None);
            again.wait_shown("tui: ready");
            again.type_line("/exit");
            let status = again.wait();
            let shown = again.shown();
            assert!(status.success(), "{point}: {status:?}\n{shown}");
            assert_eq!(reserve_files(&rig), reserves, "{point}");
        }
    }

    /// Review round 2, finding 2. A segment sealed without its host noting the commit its
    /// workspace had (here `goalcup seal`'s doing) has that commit noted by `continue` before the
    /// next segment starts: its final heartbeat carries it, and the next segment's agent
    /// committing afterwards does not change it.
    #[test]
    fn the_old_segments_final_heartbeat_carries_the_commit_it_ended_with() {
        use goalcup_protocol::control::{CaptureSealRequest, DaemonClient};
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let (run_id, entry) = crashed_host(&rig, &server);
        let workspace = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap().workspace;
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        // The first segment's agent had committed its work.
        git(&["commit", "--allow-empty", "-m", "the first segment's work"]);
        let sealed = rig
            .client()
            .seal_capture(
                &entry,
                &CaptureSealRequest {
                    proxy: None,
                    remaining_finish_ms: Some(10_000),
                    native_scope: None,
                },
                std::time::Duration::from_secs(20),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(sealed.entry_id, entry);
        let ended_with =
            goalcup_cli::upload::uploader::repo_head(&workspace).expect("the clone has a commit");

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        // The new segment's agent commits.
        git(&["commit", "--allow-empty", "-m", "the next segment's work"]);
        let later = goalcup_cli::upload::uploader::repo_head(&workspace).unwrap();
        assert_ne!(later, ended_with);
        wait_until("the earlier segment's end", || {
            server.final_of(&entry).is_some()
        });
        assert_eq!(
            server.final_of(&entry).unwrap().repo_head.as_deref(),
            Some(ended_with.as_str()),
            "the commit the workspace had when that segment ended"
        );
        second.type_line("/exit");
        assert!(second.wait().success());
        assert_eq!(
            server.final_of(&entry).unwrap().repo_head.as_deref(),
            Some(ended_with.as_str())
        );
    }

    /// Review round 2, finding 4. A `continue` the server refuses because the run is over still
    /// waits for the earlier segment's upload as a stop does (with its progress said), not for the
    /// twenty seconds of other refusals, and says so.
    #[test]
    fn a_continue_refused_because_the_run_is_over_waits_for_the_earlier_upload() {
        let rig = Rig::chat();
        let server = server(
            &rig,
            ServerOptions {
                run_seconds: Some(12),
                ..ServerOptions::default()
            },
        );
        let (run_id, entry) = crashed_host(&rig, &server);
        let deadline = goalcup_protocol::control::parse_z(
            goalcup_cli::run::read(&rig.cfg, &run_id)
                .unwrap()
                .deadline_at
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        wait_until("the run's deadline", || {
            goalcup_cli::run::now_unix() > deadline
        });
        server.fail_next_chunks(2);
        let mut over = rig.continue_on_pty(&[DISK, BEATING], None);
        let status = over.wait();
        let said = over.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        assert!(said.contains("is over"), "{said}");
        assert!(
            said.contains("the run is over: waiting for the earlier segment's upload"),
            "{said}"
        );
        assert!(
            said.contains("an earlier segment of this run, acknowledged"),
            "{said}"
        );
        assert_eq!(server.entry_ids(&run_id).len(), 1, "no segment was made");
        assert_entry_sent(&rig, &server, &entry);
    }

    // ------------------------------------------------------------ the deadline in chat mode

    /// The run's `state.json` as its last host left it.
    fn last_state(rig: &Rig, run_id: &str) -> goalcup_cli::run::RunState {
        goalcup_cli::run::read_state(&goalcup_cli::run::run_dir(&rig.cfg, run_id))
            .expect("the run's state")
    }

    fn alive(pid: &str) -> bool {
        let pid: i32 = pid.parse().expect("a pid");
        // SAFETY: two integers; signal 0 only asks whether the process is there.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Whether `why` is the stop a server's heartbeat answer gives at the run's deadline.
    fn the_servers_stop(why: Option<&str>) -> bool {
        why.is_some_and(|why| why.starts_with("the server says the run is over"))
    }

    /// T2-F2a. A run in chat mode reaches the server's deadline while its agent holds the
    /// terminal: the heartbeat answer says stop, the host ends the agent, `goalcup start`
    /// returns and the shell has the terminal again, and the segment is sealed and uploaded
    /// whole. The run is over: no `continue` is offered, and its reserve goes back.
    #[test]
    fn the_servers_deadline_ends_a_chat_mode_run_and_gives_the_terminal_back() {
        let rig = Rig::chat();
        // Long enough for the agent to start on a loaded test machine; short for a test.
        let server = server(
            &rig,
            ServerOptions {
                run_seconds: Some(8),
                ..ServerOptions::default()
            },
        );
        let mut run = rig.start_under_a_shell_on_pty(&[DISK, BEATING]);
        run.wait_shown("tui: ready");
        run.type_line("hello");
        run.wait_shown("tui: noted hello");
        let agent = harness(&rig, "tui.pid");
        // `goalcup start` has returned: the shell that ran it says its status.
        run.wait_shown("shell: 0");
        assert!(run.wait().success(), "{}", run.shown());
        let shown = run.shown();
        assert!(!alive(&agent), "the agent was ended: {shown}");
        assert_eq!(
            harness(&rig, "tui.end"),
            "",
            "it did not end on its own: {shown}"
        );
        let stop = shown
            .find("deadline: the server says the run is over")
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(
            stop > shown.rfind("tui: ").unwrap(),
            "the host speaks once the agent is gone: {shown}"
        );
        assert!(!shown.contains("goalcup continue"), "{shown}");
        // Below the details, the participant reads that time is up and that nothing is left to
        // do; the `deadline:` line the scripts read is still there above it.
        let up = shown
            .rfind(goalcup_cli::participant::TIME_IS_UP)
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(up > stop, "{shown}");
        assert!(
            shown[up..].contains(goalcup_cli::participant::NOTHING_MORE),
            "{shown}"
        );

        let run_id = the_run(&server);
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_entry_uploaded(&rig, &server, &entries[0]);
        let state = last_state(&rig, &run_id);
        assert!(the_servers_stop(state.stopped.as_deref()), "{state:?}");
        assert!(
            !reserve_path(&artifact_root(&rig.cfg), &run_id).exists(),
            "the run is over: its reserve goes back"
        );
    }

    /// T2-F2b. The first segment's agent crashes and `goalcup continue` starts the next. That
    /// segment's agent is ended by the server's stop at the run's deadline (the run's, not a
    /// fresh one for the segment), and both segments end up on the server.
    #[test]
    fn a_continued_segment_is_stopped_by_the_servers_stop_at_the_runs_deadline() {
        let rig = Rig::chat();
        let server = server(
            &rig,
            ServerOptions {
                run_seconds: Some(20),
                ..ServerOptions::default()
            },
        );
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success(), "{}", first.shown());
        let run_id = the_run(&server);
        let deadline = goalcup_protocol::control::parse_z(
            goalcup_cli::run::read(&rig.cfg, &run_id)
                .unwrap()
                .deadline_at
                .as_deref()
                .unwrap(),
        )
        .unwrap();

        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        second.type_line("again");
        second.wait_shown("tui: noted again");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert!(
            shown.contains("deadline: the server says the run is over"),
            "{shown}"
        );
        let now = goalcup_cli::run::now_unix();
        assert!(
            now + 1 >= deadline && now < deadline + 60,
            "stopped at the run's deadline {deadline}, at {now}"
        );
        assert_eq!(harness(&rig, "tui.end"), "", "it did not end on its own");
        assert!(!shown.contains("goalcup continue"), "{shown}");

        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        for entry in &entries {
            assert_entry_uploaded(&rig, &server, entry);
        }
        let state = last_state(&rig, &run_id);
        assert_eq!(state.entry_id, entries[1]);
        assert!(the_servers_stop(state.stopped.as_deref()), "{state:?}");
        assert!(!reserve_path(&artifact_root(&rig.cfg), &run_id).exists());
    }

    /// T2-F2b and WP11. With the server away, a continued segment stops on its own count of
    /// what the server said was left (`seconds_left`, counted on the continuous clock), and not
    /// on this machine's date. Here the continue's answer says six seconds are left of a run
    /// whose deadline is a week away (which a count from the date would run to), no heartbeat
    /// of the segment is answered, so nothing else sets the count, and then the server is gone.
    /// The rest of the segment is sent by `goalcup upload` once the server is back.
    #[test]
    fn a_continued_segment_with_the_server_away_stops_on_the_servers_count() {
        let rig = Rig::chat();
        let store = rig.root.join("server");
        let first_server = UploadServer::start(&store, ServerOptions::default());
        rig.prepare_for(&first_server.url());
        let port = first_server.port();
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success(), "{}", first.shown());
        let run_id = the_run(&first_server);
        first_server.cap_seconds_left(Some(6));
        first_server.fail_heartbeats(true);

        let asked = std::time::Instant::now();
        let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
        second.wait_shown("tui: ready");
        let segment = first_server.entry_ids(&run_id)[1].clone();
        // The host keeps the server's count where `goalcup status` reads it.
        wait_until("the server's count in the run's state", || {
            last_state(&rig, &run_id)
                .time_left
                .is_some_and(|left| left.seconds <= 6)
        });
        first_server.stop();
        second.wait_shown("by the server's count");
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(40),
            "the segment ran on past the server's count: {}",
            second.shown()
        );
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");
        assert_eq!(harness(&rig, "tui.end"), "", "it did not end on its own");
        let state = last_state(&rig, &run_id);
        assert!(
            state
                .stopped
                .as_deref()
                .is_some_and(|why| why.contains("by the server's count")),
            "{state:?}"
        );

        let server = UploadServer::start(
            &store,
            ServerOptions {
                port: Some(port),
                ..ServerOptions::default()
            },
        );
        goalcup_cli::commands::upload::upload_with(
            &rig.cfg,
            &segment,
            super::quick_policy(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_entry_uploaded(&rig, &server, &segment);
    }

    /// Review fix round 1. A `continue` the server refuses because the run is over (a newer run
    /// replaced it, a week before its deadline) marks the run over in its state: the advice of
    /// `goalcup status` and the reserve's release follow the server at once, although the count
    /// and the date both say days are left.
    #[test]
    fn a_continue_the_server_refuses_as_over_marks_the_run_over() {
        let rig = Rig::chat();
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[DISK, BEATING]);
        first.wait_shown("tui: ready");
        kill_agent(&rig);
        assert!(first.wait().success(), "{}", first.shown());
        let run_id = the_run(&server);
        let reserve = reserve_path(&artifact_root(&rig.cfg), &run_id);
        assert!(reserve.exists(), "the open run keeps its reserve");
        replace_run(&server, &run_id);

        let mut refused = rig.continue_on_pty(&[DISK, BEATING], None);
        let status = refused.wait();
        let said = refused.shown();
        assert_eq!(status.code(), Some(1), "{said}");
        assert!(said.contains("is over"), "{said}");
        let state = last_state(&rig, &run_id);
        assert_eq!(
            state.stopped.as_deref(),
            Some(goalcup_cli::run::SERVER_SAYS_OVER),
            "{state:?}"
        );
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        let now = goalcup_cli::run::now_unix();
        assert!(
            !goalcup_cli::run::time_left::time_up(
                state.time_left.as_ref(),
                record.deadline_at.as_deref(),
                now
            ),
            "the count says the run goes on"
        );
        assert_eq!(
            goalcup_cli::run::advice(&rig.cfg, &record, &state, now),
            goalcup_cli::run::Advice::Over
        );
        goalcup_cli::run::reserve::release_stale(&rig.cfg, &artifact_root(&rig.cfg), now);
        assert!(!reserve.exists(), "the reserve went back");
    }

    // ------------------------------------------------------------ parts, pruned, then continued

    /// A test build's disk sizes for a run whose chain parts are soon past its reservation
    /// (8 KiB), so that every part the server acknowledged is deleted here at once.
    const SMALL_DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "8192,0,50");

    /// Type lines into the agent until `done`. Each is long, so a few fill a 4 KiB part.
    fn type_until(
        on: &crate::common::m2_cli_support::OnPty,
        tag: &str,
        mut done: impl FnMut() -> bool,
    ) {
        let pad = "x".repeat(200);
        for i in 0..400 {
            if done() {
                return;
            }
            on.type_line(&format!("{tag}-{i:03}-{pad}"));
            on.wait_shown(&format!("tui: noted {tag}-{i:03}-"));
        }
        panic!("{tag}: 400 lines, and it never happened:\n{}", on.shown());
    }

    /// Whether a chain part of `entry_id` was deleted on this machine.
    fn pruned(rig: &Rig, entry_id: &str) -> bool {
        goalcup_cli::upload::prune::parts_now(&rig.trace(entry_id))
            .ok()
            .and_then(|parts| parts.first().map(|part| part.start > 0))
            .unwrap_or(false)
    }

    /// Every record of the entry's chain is on the server exactly once and in order, the parts
    /// still on this machine are the server's bytes where they lie and end where the server's
    /// copy ends, and the server holds the entry's seal.
    fn assert_once_on_server(rig: &Rig, server: &UploadServer, entry_id: &str) {
        let copy = copy_of(server, entry_id);
        let seqs: Vec<u64> = copy
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                serde_json::from_slice::<serde_json::Value>(line).unwrap()["seq"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert!(!seqs.is_empty(), "{entry_id}: records");
        assert_eq!(
            seqs,
            (0..seqs.len() as u64).collect::<Vec<_>>(),
            "{entry_id}: each record once, in order"
        );
        let trace = rig.trace(entry_id);
        let parts = goalcup_cli::upload::prune::parts_now(&trace).unwrap();
        let last = parts.last().expect("a part is kept");
        assert_eq!(
            last.start + last.len,
            copy.len() as u64,
            "{entry_id}: the end"
        );
        for part in &parts {
            let at = part.start as usize..(part.start + part.len) as usize;
            assert_eq!(
                std::fs::read(&part.path).unwrap(),
                copy[at],
                "{entry_id}: {}",
                part.path.display()
            );
        }
        let (seal, _) = server
            .seal_of(entry_id)
            .unwrap_or_else(|| panic!("{entry_id} was not sealed on the server"));
        assert_eq!(
            seal.as_bytes(),
            std::fs::read(trace.join("seal.json")).unwrap()
        );
    }

    /// The entry's trace as the server holds it: this machine's trace with its chain parts
    /// replaced by the server's copy of the whole chain, scanned under the server's key. (The
    /// trace here, its acknowledged parts deleted, can be checked only as far as it goes.)
    fn scan_servers_copy(
        rig: &Rig,
        server: &UploadServer,
        entry_id: &str,
    ) -> goalcup_verifier::capture_analysis::reader::CaptureIntegrityReport {
        fn copy_tree(from: &Path, to: &Path, top: bool) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let name = entry.file_name();
                let (source, dest) = (entry.path(), to.join(&name));
                if source.is_dir() {
                    copy_tree(&source, &dest, false);
                } else if !(top
                    && name
                        .to_str()
                        .and_then(goalcup_protocol::export::parse_part_name)
                        .is_some())
                {
                    std::fs::copy(&source, &dest).unwrap();
                }
            }
        }
        let copy = rig.root.join("servers-copy").join(entry_id);
        copy_tree(&rig.trace(entry_id), &copy, true);
        std::fs::write(copy.join("chain.jsonl"), copy_of(server, entry_id)).unwrap();
        let options = goalcup_verifier::VerifyOptions {
            extra_server_keys: vec![server.public_key_hex()],
            ..Default::default()
        };
        scan_capture_trace(&copy, &options, &mut |_| Ok(())).unwrap()
    }

    /// T2-F3. The 168-hour path through the CLI: the daemon rotates the chain into 4 KiB parts,
    /// the uploader sends them, the watch deletes the acknowledged ones under disk pressure, the
    /// host and its agent die, `goalcup continue` seals and sends what was left of the first
    /// segment beside the second, and the second ends. Every record of both reaches the server
    /// exactly once.
    #[test]
    fn rotated_parts_deleted_here_reach_the_server_once_through_a_crash_and_a_continue() {
        let rig = Rig::chat();
        rig.daemon
            .as_ref()
            .expect("the rig's daemon")
            .set_chain_part_limits_for_test(4096, std::time::Duration::from_secs(3600));
        let server = server(&rig, ServerOptions::default());
        let mut first = rig.start_on_pty(&[SMALL_DISK, BEATING]);
        first.wait_shown("tui: ready");
        let run_id = the_run(&server);
        let entry = server.entry_ids(&run_id)[0].clone();
        type_until(&first, "one", || pruned(&rig, &entry));
        // The machine loses power: the host and its agent at once; the daemon goes on.
        // SAFETY: two integers; the pid is this test's own child.
        unsafe { libc::kill(first.child.id() as i32, libc::SIGKILL) };
        kill_agent(&rig);
        first.close();
        first.wait();

        let mut second = rig.continue_on_pty(&[SMALL_DISK, BEATING], None);
        second.wait_shown("tui: ready");
        let entries = server.entry_ids(&run_id);
        assert_eq!(entries.len(), 2, "{entries:?}");
        type_until(&second, "two", || pruned(&rig, &entries[1]));
        second.type_line("/exit");
        let status = second.wait();
        let shown = second.shown();
        assert!(status.success(), "{status:?}\n{shown}");

        for entry in &entries {
            assert!(pruned(&rig, entry), "{entry}: a part was deleted here");
            assert_once_on_server(&rig, &server, entry);
            let report = scan_servers_copy(&rig, &server, entry);
            assert_eq!(report.anchor, AnchorState::Signed, "{entry}: {report:?}");
            assert!(report.sealed, "{entry}: {report:?}");
        }
        // The segment that ended normally is whole; the first, whose host died, may have a gap.
        let report = scan_servers_copy(&rig, &server, &entries[1]);
        assert_eq!(report.integrity, CaptureIntegrity::Valid, "{report:?}");
    }
}
