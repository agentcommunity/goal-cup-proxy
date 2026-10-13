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
