//! What the binary refuses before it binds anything.
//!
//! This server has no authentication at all — it signs with a key published in this
//! repository, so a token would protect nothing and would only make it possible to forget
//! what the key is worth. What keeps `receipt` and `chunks` safe is therefore the socket,
//! and the socket alone: a flag documented as "loopback address" that enforced nothing
//! would let `--addr 0.0.0.0:8080` serve unauthenticated receipt issuance to the network,
//! where anybody could open an entry under a run key of their choosing and be handed
//! signatures for it. `DaemonConfig::validate` refuses a non-loopback `control_addr` on the
//! other side of the same wire; this is the same refusal, on this side.

#[path = "support/chain.rs"]
mod chain;

use goalcup_devserver::ReceiptRequest;
use goalcup_protocol::receipt::StartReceipt;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};

/// The binary, with `args`, run to completion. It never binds on any path these tests take.
fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-devserver"))
        .args(args)
        .output()
        .expect("the binary runs")
}

#[test]
fn a_non_loopback_listen_address_is_refused_before_anything_binds() {
    for addr in ["0.0.0.0:0", "0.0.0.0:8080", "[::]:8080"] {
        let out = run(&["--addr", addr, "--print-key"]);
        assert!(
            !out.status.success(),
            "--addr {addr} was accepted: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("not loopback"),
            "the refusal must say why: {stderr}"
        );
        // Nothing was printed to stdout, which is what a script pins a key from: a refusal
        // that still answered `--print-key` would be a script configuring a daemon against a
        // server that never came up.
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "a refused start prints nothing on stdout"
        );
    }
}

/// A host NAME is refused by spelling rather than resolved. A name resolves at connect time
/// to whatever the resolver says that day, which is not a fact this check could hold, so
/// `localhost` is written out as `127.0.0.1` or `[::1]`.
#[test]
fn an_address_that_is_not_an_ip_and_port_is_refused_and_says_so() {
    let out = run(&["--addr", "localhost:8080", "--print-key"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is not an <ip>:<port> address"),
        "the refusal must name the shape it wanted: {stderr}"
    );
}

/// The loopback spellings still work, and `--print-key` is what a script pins a daemon's
/// `server_public_key` against — so this asserts the whole of that path, not just the parse.
#[test]
fn every_loopback_spelling_is_accepted_and_print_key_answers_the_pinned_placeholder() {
    for addr in ["127.0.0.1:0", "127.0.0.5:0", "[::1]:0"] {
        let out = run(&["--addr", addr, "--print-key"]);
        assert!(
            out.status.success(),
            "--addr {addr} was refused: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            goalcup_devserver::dev_server_public_key_hex(),
            "--print-key answers the key this build signs with"
        );
    }
}

#[test]
fn receipt_mode_flags_are_closed_and_validate_before_printing_a_key() {
    for args in [
        vec!["--receipt-mode", "new-event", "--print-key"],
        vec![
            "--receipt-mode",
            "legacy",
            "--questionnaire-version",
            "2026.09-v1",
            "--print-key",
        ],
        vec!["--receipt-mode", "unknown", "--print-key"],
        vec![
            "--receipt-mode",
            "new-event",
            "--questionnaire-version",
            "contains spaces",
            "--print-key",
        ],
    ] {
        let out = run(&args);
        assert!(
            !out.status.success(),
            "invalid flags were accepted: {args:?}"
        );
        assert!(
            out.stdout.is_empty(),
            "authority validation must precede --print-key: {args:?}"
        );
    }

    let out = run(&[
        "--receipt-mode",
        "new-event",
        "--questionnaire-version",
        "2026.09-v1",
        "--print-key",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        goalcup_devserver::dev_server_public_key_hex()
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Receipt mode: new-event (2026.09-v1)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn qualification_window_cli_requires_a_positive_integer_before_printing_a_key() {
    let out = run(&["--qualification-window-seconds", "0", "--print-key"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("qualification-window-seconds"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct RunningServer {
    child: Child,
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_new_event_binary() -> (RunningServer, String) {
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-devserver"))
        .args([
            "--addr",
            "127.0.0.1:0",
            "--receipt-mode",
            "new-event",
            "--questionnaire-version",
            "2026.09-v1",
            "--clock",
            chain::AT,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the configured binary starts");
    // Install the kill-and-wait guard before any fallible wait so a failed startup leaves
    // no child behind.
    let mut server = RunningServer { child };
    let stderr = server.child.stderr.take().expect("stderr is piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut looking = true;
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if looking && let Some(addr) = line.strip_prefix("listening on http://") {
                looking = tx.send(addr.to_string()).is_ok();
            }
        }
    });
    let addr = rx
        // A listener line from the real binary can take more than ten seconds; the receipt
        // assertions begin only after the child reports the bound socket.
        .recv_timeout(std::time::Duration::from_secs(60))
        .unwrap_or_else(|e| {
            let status = server.child.try_wait().ok().flatten();
            panic!("the binary did not report its loopback address ({e}); status {status:?}")
        });
    (server, addr)
}

fn post(addr: &str, path: &str, request: &ReceiptRequest) -> (u16, String) {
    let body = serde_json::to_string(request).expect("the request serialises");
    let mut stream = std::net::TcpStream::connect(addr).expect("the binary is listening");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .expect("the request writes");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("the response reads");
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .expect("an HTTP status");
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
        .to_string();
    (status, body)
}

#[test]
fn the_new_event_binary_serves_its_configured_receipt_contract() {
    let (_server, addr) = start_new_event_binary();
    let run = chain::run_key();
    let key = chain::run_key_hex(&run);
    let links = chain::chain("binary-entry");
    let mut request = chain::receipt_request("binary-entry", &links[0].link.hash, &key);
    request.claims.run_public_key = Some(key.clone());
    request.claims.questionnaire_version = Some("2026.09-v1".to_string());

    let (status, body) = post(&addr, "/v0/entries/binary-entry/receipt", &request);
    assert_eq!(status, 200, "{body}");
    let receipt: StartReceipt = serde_json::from_str(&body).expect("a signed receipt");
    assert_eq!(receipt.claims.run_public_key.as_deref(), Some(key.as_str()));
    assert_eq!(
        receipt.claims.questionnaire_version.as_deref(),
        Some("2026.09-v1")
    );
    receipt
        .verify_with(&goalcup_devserver::dev_server_public_key_hex())
        .expect("the binary used its configured signing authority");
}

#[test]
fn the_binary_signs_the_selected_short_qualification_window() {
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-devserver"))
        .args([
            "--addr",
            "127.0.0.1:0",
            "--qualification-window-seconds",
            "2",
            "--clock",
            chain::AT,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the configured binary starts");
    let mut server = RunningServer { child };
    let stderr = server.child.stderr.take().expect("stderr is piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut looking = true;
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if looking && let Some(addr) = line.strip_prefix("listening on http://") {
                looking = tx.send(addr.to_string()).is_ok();
            }
        }
    });
    let addr = rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the binary reports its loopback address");

    let key = chain::run_key();
    let key_hex = chain::run_key_hex(&key);
    let links = chain::chain("short-window-binary");
    let request = chain::receipt_request("short-window-binary", &links[0].link.hash, &key_hex);
    let (status, body) = post(&addr, "/v0/entries/short-window-binary/receipt", &request);
    assert_eq!(status, 200, "{body}");
    let receipt: StartReceipt = serde_json::from_str(&body).expect("a signed receipt");
    assert_eq!(receipt.claims.started_at, chain::AT);
    assert_eq!(receipt.claims.deadline_at, "2026-09-01T00:00:02Z");
}
