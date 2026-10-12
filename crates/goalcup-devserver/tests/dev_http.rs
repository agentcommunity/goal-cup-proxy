//! The transport, over a real loopback socket.
//!
//! `dev_router` is the only part of this crate a daemon actually talks to, and a router
//! whose status codes were only asserted in prose would let a client be written against
//! the wrong shape: a retrying uploader has to tell "offline, come back" from "refused,
//! this document is bad", and those are the same `Err` in Rust and 503 against 400 on the
//! wire.
//!
//! The client here is a few lines of HTTP/1.1 over `std::net::TcpStream` rather than a
//! client crate. This crate ships nothing, and a dependency added so a test can speak the
//! protocol it already serves is a dependency the workspace then carries — as is the
//! `io-util` feature an async client would need on the tokio the daemon shares. The
//! runtime is built by hand rather than by `#[tokio::test]` for the same reason: the
//! server runs on its worker threads while this thread blocks on a socket.
//!
//! The refusals are here too, because a daemon has to tell them apart on the wire: an entry
//! it never opened (404) from a document that does not link (400) from a server that is
//! merely down (503).

/// The link and chunk builders, included by `#[path]`: this crate's tests have no `support`
/// module.
#[path = "support/chain.rs"]
mod chain;

use goalcup_devserver::{DevServer, ReceiptAuthority, ReceiptRequest, dev_router};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::StartReceipt;
use std::io::{Read, Write};
use std::sync::Arc;

/// Start the router on a loopback port the OS chose, in this process.
async fn start(server: Arc<DevServer>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, dev_router(server)).await;
    });
    format!("{addr}")
}

/// One request, one response: `(status, body)`. `Connection: close` so the body ends at
/// EOF and this needs no chunked decoder.
fn request(addr: &str, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    (status, body.to_string())
}

fn post_json<T: serde::Serialize>(addr: &str, path: &str, value: &T) -> (u16, String) {
    request(
        addr,
        "POST",
        path,
        &serde_json::to_string(value).expect("serialise the request"),
    )
}

#[test]
fn the_router_signs_over_the_wire_and_says_which_refusal_it_is() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let server = Arc::new(DevServer::new());
    server.set_clock(chain::AT);
    let addr = runtime.block_on(start(server.clone()));

    let links = chain::chain("e1");
    let run = chain::run_key();
    let request_body = chain::receipt_request("e1", &links[0].link.hash, &chain::run_key_hex(&run));

    // The banner, so an operator who points a daemon at this is told what it is.
    let (status, body) = request(&addr, "GET", "/v0/dev/banner", "");
    assert_eq!(status, 200);
    assert_eq!(body, server.banner());

    // A chunk before any receipt: 404, because this server has no such entry to file it
    // under. Asserted before the receipt is asked for, so it is the state and not the
    // document that is being refused.
    let chunk = chain::chunk_over("e1", &links[0..=3], &run);
    let (status, _) = post_json(&addr, "/v0/entries/e1/chunks", &chunk);
    assert_eq!(status, 404);

    // A receipt, signed by the server the socket is in front of, and dated by its clock.
    let (status, body) = post_json(&addr, "/v0/entries/e1/receipt", &request_body);
    assert_eq!(status, 200, "{body}");
    let receipt: StartReceipt = serde_json::from_str(&body).unwrap();
    assert_eq!(
        receipt.verify_with(&goalcup_devserver::dev_server_public_key_hex()),
        Ok(())
    );
    assert_eq!(
        (
            receipt.claims.started_at.as_str(),
            receipt.claims.deadline_at.as_str()
        ),
        (chain::AT, chain::DEADLINE)
    );

    // On the wire: 409, not a second receipt with a later `started_at`.
    let (status, _) = post_json(&addr, "/v0/entries/e1/receipt", &request_body);
    assert_eq!(status, 409);

    // The URL names one entry and the document another: 404, rather than a document filed
    // under whichever of the two the server happened to read.
    let (status, _) = post_json(&addr, "/v0/entries/e2/receipt", &request_body);
    assert_eq!(status, 404);

    // A run key nothing can decode: 400 at the receipt, the one moment it can be refused.
    let (status, _) = post_json(
        &addr,
        "/v0/entries/e9/receipt",
        &ReceiptRequest {
            claims: chain::receipt_request("e9", &links[0].link.hash, "").claims,
            run_public_key: "not a key".to_string(),
        },
    );
    assert_eq!(status, 400);

    // Legacy key acceptance is intentionally unchanged by the new binding grammar. The
    // identity encoding is a curve point the historical server accepted; new-event mode
    // rejects it as weak, asserted below.
    let weak = format!("01{}", "00".repeat(31));
    let mut legacy_weak = chain::receipt_request("legacy-weak", &links[0].link.hash, &weak);
    legacy_weak.claims.entry_id = "legacy-weak".to_string();
    let (status, body) = post_json(&addr, "/v0/entries/legacy-weak/receipt", &legacy_weak);
    assert_eq!(status, 200, "legacy acceptance drifted: {body}");

    // A chunk that carries nothing: 400. The claims alone are the client's word.
    let hollow = chain::chunk_without_envelopes("e1", 0, 3, &links[3].link.hash, &run);
    let (status, _) = post_json(&addr, "/v0/entries/e1/chunks", &hollow);
    assert_eq!(status, 400);

    // A chunk, acknowledged over the same socket, and an acknowledgement whose signature
    // covers `acked_at`.
    let (status, body) = post_json(&addr, "/v0/entries/e1/chunks", &chunk);
    assert_eq!(status, 200, "{body}");
    let ack: AckCheckpoint = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (ack.seq, ack.hash.as_str()),
        (3, links[3].link.hash.as_str())
    );
    assert!(
        ack.verify(&goalcup_devserver::dev_server_public_key_hex())
            .is_ok()
    );
    assert!(ack.acked_at_secs().is_some());

    // A chunk that does not continue the acknowledged chain: 400, not a second opinion
    // about seq 3.
    let forked = chain::links("e1", 8, 1);
    let (status, _) = post_json(
        &addr,
        "/v0/entries/e1/chunks",
        &chain::chunk_over("e1", &forked[4..=7], &run),
    );
    assert_eq!(status, 400);

    // A document that did not verify is a 400: the client's to fix, not to retry.
    let mut forged = chain::chunk_over("e1", &links[4..=7], &run);
    forged.signature = "00".repeat(64);
    let (status, _) = post_json(&addr, "/v0/entries/e1/chunks", &forged);
    assert_eq!(status, 400);

    // Offline is a 503: the same request, worth retrying, and the anchor has not moved.
    server.set_offline(true);
    let (status, _) = post_json(&addr, "/v0/entries/e1/chunks", &chunk);
    assert_eq!(status, 503);
    assert_eq!(server.acked_seq("e1"), Some(3));
}

#[test]
fn new_event_receipts_require_one_canonical_key_and_the_configured_version() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let authority = ReceiptAuthority::new_event("2026.09-v1").expect("a valid authority");
    let server = Arc::new(DevServer::with_authority(authority).expect("the server builds"));
    server.set_clock(chain::AT);
    let addr = runtime.block_on(start(server.clone()));
    let links = chain::chain("e1");
    let run = chain::run_key();
    let canonical = chain::run_key_hex(&run);

    let (status, body) = request(&addr, "GET", "/v0/dev/banner", "");
    assert_eq!(status, 200);
    assert!(body.contains("new-event (2026.09-v1)"), "{body}");

    let mut missing = chain::receipt_request("missing", &links[0].link.hash, &canonical);
    missing.claims.entry_id = "missing".to_string();
    let (status, _) = post_json(&addr, "/v0/entries/missing/receipt", &missing);
    assert_eq!(status, 400, "new-event requires both binding claims");

    let mut half = chain::receipt_request("half", &links[0].link.hash, &canonical);
    half.claims.entry_id = "half".to_string();
    half.claims.run_public_key = Some(canonical.clone());
    let (status, _) = post_json(&addr, "/v0/entries/half/receipt", &half);
    assert_eq!(status, 400, "half a binding is refused");

    let mut malformed = chain::receipt_request("malformed", &links[0].link.hash, "not-a-key");
    malformed.claims.entry_id = "malformed".to_string();
    malformed.claims.run_public_key = Some("not-a-key".to_string());
    malformed.claims.questionnaire_version = Some("contains spaces".to_string());
    let (status, _) = post_json(&addr, "/v0/entries/malformed/receipt", &malformed);
    assert_eq!(status, 400, "malformed binding claims are refused");

    let mut bad_version = chain::receipt_request("bad-version", &links[0].link.hash, &canonical);
    bad_version.claims.entry_id = "bad-version".to_string();
    bad_version.claims.run_public_key = Some(canonical.clone());
    bad_version.claims.questionnaire_version = Some("contains spaces".to_string());
    let (status, _) = post_json(&addr, "/v0/entries/bad-version/receipt", &bad_version);
    assert_eq!(status, 400, "malformed questionnaire versions are refused");

    let mut weak = chain::receipt_request("weak", &links[0].link.hash, &canonical);
    weak.claims.entry_id = "weak".to_string();
    weak.claims.run_public_key = Some(format!("01{}", "00".repeat(31)));
    weak.claims.questionnaire_version = Some("2026.09-v1".to_string());
    weak.run_public_key = weak.claims.run_public_key.clone().unwrap();
    let (status, _) = post_json(&addr, "/v0/entries/weak/receipt", &weak);
    assert_eq!(status, 400, "a weak point is not a usable run key");

    let other = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    let mut wrong = chain::receipt_request("correctable", &links[0].link.hash, &canonical);
    wrong.claims.entry_id = "correctable".to_string();
    wrong.claims.run_public_key = Some(other);
    wrong.claims.questionnaire_version = Some("2026.09-v2".to_string());
    let (status, _) = post_json(&addr, "/v0/entries/correctable/receipt", &wrong);
    assert_eq!(status, 400, "mismatched binding is refused before issuance");

    // The first refusal inserted no state: a corrected request may use the same id. Both
    // ingress spellings are uppercase, while the signed receipt is canonical lowercase.
    let mut corrected = chain::receipt_request(
        "correctable",
        &links[0].link.hash,
        &canonical.to_uppercase(),
    );
    corrected.claims.entry_id = "correctable".to_string();
    corrected.claims.run_public_key = Some(canonical.to_uppercase());
    corrected.claims.questionnaire_version = Some("2026.09-v1".to_string());
    let (status, body) = post_json(&addr, "/v0/entries/correctable/receipt", &corrected);
    assert_eq!(status, 200, "{body}");
    let receipt: StartReceipt = serde_json::from_str(&body).unwrap();
    assert_eq!(
        receipt.claims.run_public_key.as_deref(),
        Some(canonical.as_str())
    );
    assert_eq!(
        receipt.claims.questionnaire_version.as_deref(),
        Some("2026.09-v1")
    );
    receipt
        .verify_with(&goalcup_devserver::dev_server_public_key_hex())
        .expect("the configured service signed the canonical claims");

    let unchecked = ReceiptAuthority::NewEvent {
        questionnaire_version: "contains spaces".to_string(),
    };
    assert!(
        DevServer::with_authority(unchecked).is_err(),
        "a caller cannot bypass validation by constructing the public enum"
    );
}
