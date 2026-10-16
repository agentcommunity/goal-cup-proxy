//! The upload-server stand-in (`tests/common/upload_server/`) against `UPLOAD-API.md`, over real
//! loopback HTTP with `ureq`. These tests are what lets the uploader's own tests trust the
//! stand-in: every rule the contract states is exercised here once.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::control::{format_z, parse_z};
use goalcup_protocol::receipt::StartReceipt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::common::upload_server::{ServerOptions, UploadServer};

const TOKEN: &str = "test-token";
const TOKEN_2: &str = "other-token";

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A stand-in head hash for record `n`; the server never checks that it is a real chain hash.
fn head(n: u64) -> String {
    sha_hex(format!("record-{n}").as_bytes())
}

struct Reply {
    status: u16,
    body: Value,
    retry_after: Option<String>,
}

/// One request on a fresh agent, so no pooled connection can retry behind the test's back.
fn call(
    method: &str,
    url: &str,
    token: Option<&str>,
    extra: &[(&str, String)],
    body: &[u8],
) -> Result<Reply, Box<ureq::Error>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut req = agent.request(method, url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    for (k, v) in extra {
        req = req.set(k, v);
    }
    let result = if body.is_empty() && method == "GET" {
        req.call()
    } else {
        req.send_bytes(body)
    };
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(Box::new(e)),
    };
    let status = resp.status();
    let retry_after = resp.header("Retry-After").map(str::to_string);
    let text = resp.into_string().expect("read the reply body");
    let body = serde_json::from_str(&text).unwrap_or_else(|_| panic!("reply is not JSON: {text}"));
    Ok(Reply {
        status,
        body,
        retry_after,
    })
}

struct Rig {
    _dir: TempDir,
    server: UploadServer,
}

fn two_tokens() -> ServerOptions {
    ServerOptions {
        tokens: vec![
            (TOKEN.to_string(), "member-1".to_string()),
            (TOKEN_2.to_string(), "member-2".to_string()),
        ],
        ..ServerOptions::default()
    }
}

impl Rig {
    fn new() -> Rig {
        Rig::with(two_tokens())
    }

    fn with(options: ServerOptions) -> Rig {
        let dir = TempDir::new().expect("tempdir");
        let server = UploadServer::start(dir.path(), options);
        Rig { _dir: dir, server }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.server.url())
    }

    fn json(&self, method: &str, path: &str, token: &str, body: &Value) -> Reply {
        call(
            method,
            &self.url(path),
            Some(token),
            &[("Content-Type", "application/json".to_string())],
            body.to_string().as_bytes(),
        )
        .expect("transport")
    }

    fn open_body(session: &str) -> Value {
        json!({
            "connector_id": "codex-cli",
            "manifest_sha256": sha_hex(b"manifest"),
            "connector_dir_sha256": sha_hex(b"dir"),
            "workspace": "/tmp/run-1/workspace",
            "session_id": session,
            "harness_version": "0.153.0",
            "tracer_version": "0.1.0",
            "repo_url": "https://github.com/someone/their-repo",
        })
    }

    fn open(&self, token: &str, session: &str) -> Reply {
        self.json("POST", "/v1/runs", token, &Rig::open_body(session))
    }

    /// Open a run for `TOKEN`, returning `(run_id, entry_id)`.
    fn open_ok(&self) -> (String, String) {
        let r = self.open(TOKEN, "session-a");
        assert_eq!(r.status, 201, "{}", r.body);
        (
            r.body["run_id"].as_str().unwrap().to_string(),
            r.body["entry_id"].as_str().unwrap().to_string(),
        )
    }

    fn get(&self, run: &str, token: &str) -> Reply {
        call(
            "GET",
            &self.url(&format!("/v1/runs/{run}")),
            Some(token),
            &[],
            b"",
        )
        .expect("transport")
    }

    fn receipt(&self, run: &str, entry: &str, chain_head: &str) -> Reply {
        self.json(
            "POST",
            &format!("/v1/runs/{run}/receipt"),
            TOKEN,
            &json!({"entry_id": entry, "chain_head": chain_head}),
        )
    }

    fn heartbeat(
        &self,
        run: &str,
        entry: &str,
        seq: u64,
        head: &str,
        final_: bool,
        repo_head: Option<&str>,
    ) -> Reply {
        self.json(
            "POST",
            &format!("/v1/runs/{run}/heartbeat"),
            TOKEN,
            &json!({"entry_id": entry, "seq": seq, "head": head, "offset_local": 150,
                    "final": final_, "repo_head": repo_head}),
        )
    }

    /// Block until the run's `stop` is true (polling `GET`), at most 10 s.
    fn wait_for_stop(&self, run: &str) {
        for _ in 0..100 {
            if self.get(run, TOKEN).body["stop"] == true {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the run never reached its deadline");
    }

    fn put(&self, run: &str, entry: &str, c: &Chunk) -> Result<Reply, Box<ureq::Error>> {
        let extra = [
            ("Content-Type", "application/zstd".to_string()),
            ("Goalcup-Entry-Id", entry.to_string()),
            ("Goalcup-Length", c.length.to_string()),
            ("Goalcup-Sha256", c.sha.clone()),
            ("Goalcup-First-Seq", c.first.to_string()),
            ("Goalcup-Last-Seq", c.last.to_string()),
            ("Goalcup-Prev", c.prev.clone()),
            ("Goalcup-Head", c.head.clone()),
        ];
        call(
            "PUT",
            &self.url(&format!("/v1/runs/{run}/chunks/{}", c.offset)),
            Some(TOKEN),
            &extra,
            &c.body,
        )
    }

    fn put_ok(&self, run: &str, entry: &str, c: &Chunk) -> Reply {
        let r = self.put(run, entry, c).expect("transport");
        assert_eq!(r.status, 200, "{}", r.body);
        r
    }
}

/// A chunk whose body is arbitrary bytes: the server hashes it but does not decompress it.
#[derive(Clone)]
struct Chunk {
    offset: u64,
    length: u64,
    first: u64,
    last: u64,
    prev: String,
    head: String,
    body: Vec<u8>,
    sha: String,
}

impl Chunk {
    /// Records `first..=last`, continuing from `prev`, starting at chain offset `offset`.
    fn new(offset: u64, length: u64, first: u64, last: u64, prev: &str) -> Chunk {
        let body = format!("zstd-frame-{offset}-{first}-{last}").into_bytes();
        Chunk {
            offset,
            length,
            first,
            last,
            prev: prev.to_string(),
            head: head(last),
            sha: sha_hex(&body),
            body,
        }
    }

    fn first() -> Chunk {
        Chunk::new(0, 100, 0, 3, GENESIS)
    }

    /// The chunk that follows `first()`.
    fn second() -> Chunk {
        Chunk::new(100, 50, 4, 5, &head(3))
    }
}

fn assert_err(r: &Reply, status: u16, code: &str) {
    assert_eq!(r.status, status, "{}", r.body);
    assert_eq!(r.body["error"], code, "{}", r.body);
    assert!(r.body["message"].as_str().is_some_and(|m| !m.is_empty()));
}

fn is_id(s: &str, prefix: char) -> bool {
    s.len() == 17
        && s.starts_with(prefix)
        && s[1..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[test]
fn open_run_mints_ids_and_times() {
    let rig = Rig::new();
    let r = rig.open(TOKEN, "session-a");
    assert_eq!(r.status, 201, "{}", r.body);
    assert!(is_id(r.body["run_id"].as_str().unwrap(), 'r'));
    assert!(is_id(r.body["entry_id"].as_str().unwrap(), 'e'));
    let start = r.body["official_start"].as_str().unwrap();
    let deadline = r.body["deadline_at"].as_str().unwrap();
    let (s, d) = (parse_z(start).unwrap(), parse_z(deadline).unwrap());
    assert_eq!(format_z(s), start);
    assert_eq!(d - s, 168 * 3600);
    assert_eq!(rig.server.run_ids().len(), 1);
}

#[test]
fn open_run_rules() {
    let rig = Rig::new();
    let first = rig.open(TOKEN, "session-a");
    assert_eq!(first.status, 201);

    // Same session, same member, no receipt yet: the same answer, not a second run.
    let again = rig.open(TOKEN, "session-a");
    assert_eq!(again.status, 200);
    assert_eq!(again.body, first.body);
    assert_eq!(rig.server.run_ids().len(), 1);

    // A different session is a new run: it replaces the first, which ends then.
    let run = first.body["run_id"].as_str().unwrap();
    let entry = first.body["entry_id"].as_str().unwrap();
    let second = rig.open(TOKEN, "session-b");
    assert_eq!(second.status, 201, "{}", second.body);
    assert_ne!(second.body["run_id"], first.body["run_id"]);
    assert!(rig.server.replaced_at(run).is_some());
    assert_eq!(
        rig.heartbeat(run, entry, 0, &head(0), false, None).body["reason"],
        "replaced"
    );
    // Another member is not touched by it.
    assert_eq!(rig.open(TOKEN_2, "session-c").status, 201);

    // Once a receipt exists, the same session is no lost reply: it is a start again.
    let run = second.body["run_id"].as_str().unwrap();
    let entry = second.body["entry_id"].as_str().unwrap();
    assert_eq!(rig.receipt(run, entry, &head(0)).status, 200);
    let third = rig.open(TOKEN, "session-b");
    assert_eq!(third.status, 201, "{}", third.body);
    assert_ne!(third.body["run_id"], second.body["run_id"]);
}

#[test]
fn open_run_validates_input() {
    let rig = Rig::new();
    assert_err(&rig.open("nope", "s"), 401, "unauthorized");
    let no_auth = call("POST", &rig.url("/v1/runs"), None, &[], b"{}").unwrap();
    assert_err(&no_auth, 401, "unauthorized");
    for bad in [
        "http://github.com/o/r",
        "https://github.com/o",
        "https://github.com/o/r/extra",
        "https://github.com/o/r/",
        "https://example.com/o/r",
        "https://github.com//r",
    ] {
        let mut body = Rig::open_body("s");
        body["repo_url"] = json!(bad);
        assert_err(
            &rig.json("POST", "/v1/runs", TOKEN, &body),
            400,
            "malformed",
        );
    }
    let mut body = Rig::open_body("s");
    body["manifest_sha256"] = json!("ABC");
    assert_err(
        &rig.json("POST", "/v1/runs", TOKEN, &body),
        400,
        "malformed",
    );
    let missing = rig.json("POST", "/v1/runs", TOKEN, &json!({"connector_id": "x"}));
    assert_err(&missing, 400, "malformed");
    assert!(rig.server.run_ids().is_empty());
}

#[test]
fn other_members_runs_are_404() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    assert_err(&rig.get(&run, TOKEN_2), 404, "not_found");
    let p = |suffix: &str| format!("/v1/runs/{run}/{suffix}");
    let receipt = json!({"entry_id": entry, "chain_head": head(0)});
    assert_err(
        &rig.json("POST", &p("receipt"), TOKEN_2, &receipt),
        404,
        "not_found",
    );
    assert_err(
        &rig.json("POST", &p("abandon"), TOKEN_2, &json!({"reason": "x"})),
        404,
        "not_found",
    );
    let seal = json!({"entry_id": entry, "seal_json": "{}", "baseline_json": "{}"});
    assert_err(
        &rig.json("POST", &p("seal"), TOKEN_2, &seal),
        404,
        "not_found",
    );
    assert_err(&rig.get("r0000000000000000", TOKEN), 404, "not_found");
    assert_err(&rig.get(&run, "nope"), 401, "unauthorized");
}

#[test]
fn receipt_is_signed_and_idempotent_per_head() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let r = rig.receipt(&run, &entry, &head(0));
    assert_eq!(r.status, 200, "{}", r.body);
    let receipt: StartReceipt = serde_json::from_value(r.body["start_receipt"].clone()).unwrap();
    receipt.verify_with(&rig.server.public_key_hex()).unwrap();
    let c = &receipt.claims;
    assert_eq!(c.entry_id, entry);
    assert_eq!(c.connector_id, "codex-cli");
    assert_eq!(c.workspace, "/tmp/run-1/workspace");
    assert_eq!(c.session_id, "session-a");
    assert_eq!(c.chain_head, head(0));
    assert_eq!(c.manifest_sha256, sha_hex(b"manifest"));
    assert_eq!(
        c.connector_dir_sha256.as_deref(),
        Some(sha_hex(b"dir").as_str())
    );
    assert!(c.run_public_key.is_none() && c.questionnaire_version.is_none());
    let opened = rig.get(&run, TOKEN);
    assert_eq!(c.started_at, opened.body["official_start"]);
    assert_eq!(c.deadline_at, opened.body["deadline_at"]);
    assert_eq!(rig.server.receipt(&run).as_ref(), Some(&receipt));

    // Same head: the stored receipt, byte for byte. Different head: a conflict.
    let again = rig.receipt(&run, &entry, &head(0));
    assert_eq!(again.status, 200);
    assert_eq!(again.body, r.body);
    assert_err(&rig.receipt(&run, &entry, &head(9)), 409, "conflict");

    // Wrong entry, malformed head.
    assert_err(
        &rig.receipt(&run, "e0000000000000000", &head(0)),
        404,
        "not_found",
    );
    assert_err(&rig.receipt(&run, &entry, "nothex"), 400, "malformed");
}

#[test]
fn a_new_run_has_position_zero_at_genesis() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let g = rig.get(&run, TOKEN);
    assert_eq!(g.status, 200);
    assert_eq!(g.body["run_id"], run.as_str());
    assert_eq!(g.body["entry_id"], entry.as_str());
    assert_eq!(g.body["status"], "open");
    assert_eq!(g.body["offset"], 0);
    assert_eq!(g.body["next_seq"], 0);
    assert_eq!(g.body["head"], GENESIS);
    assert!(g.body["last_ack"].is_null());
}

#[test]
fn chunks_are_stored_acknowledged_and_continue() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let c1 = Chunk::first();
    let r = rig.put_ok(&run, &entry, &c1);
    assert_eq!(r.body["offset"], 100);
    assert_eq!(r.body["next_seq"], 4);
    assert_eq!(r.body["head"], c1.head.as_str());
    let ack: AckCheckpoint = serde_json::from_value(r.body["ack"].clone()).unwrap();
    ack.verify(&rig.server.public_key_hex()).unwrap();
    assert_eq!(
        (ack.entry_id.as_str(), ack.seq, ack.hash.as_str()),
        (entry.as_str(), 3, c1.head.as_str())
    );
    assert!(parse_z(&ack.acked_at).is_some());

    let c2 = Chunk::second();
    rig.put_ok(&run, &entry, &c2);
    let g = rig.get(&run, TOKEN);
    assert_eq!(g.body["offset"], 150);
    assert_eq!(g.body["next_seq"], 6);
    assert_eq!(g.body["head"], c2.head.as_str());
    assert_eq!(g.body["last_ack"]["seq"], 5);

    let stored = rig.server.chunks(&run);
    assert_eq!(stored.len(), 2);
    assert_eq!(
        (
            stored[1].offset,
            stored[1].length,
            stored[1].first_seq,
            stored[1].last_seq
        ),
        (100, 50, 4, 5)
    );
    assert_eq!(
        (stored[1].prev.as_str(), stored[1].head.as_str()),
        (head(3).as_str(), c2.head.as_str())
    );
    assert_eq!(std::fs::read(&stored[0].body_path).unwrap(), c1.body);
    let name = stored[0].body_path.file_name().unwrap().to_str().unwrap();
    assert_eq!(name, format!("{:016}-{}.zst", 0, c1.sha));
    assert_eq!(rig.server.position(&run).unwrap().offset, 150);
    // Nothing but the two bodies is left in the entry directory (no temp files).
    let entries = std::fs::read_dir(stored[0].body_path.parent().unwrap())
        .unwrap()
        .count();
    assert_eq!(entries, 2);
}

#[test]
fn chunks_that_do_not_continue_are_conflicts() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    rig.put_ok(&run, &entry, &Chunk::first());
    let before = rig.server.position(&run).unwrap();

    let mut bad_offset = Chunk::second();
    bad_offset.offset = 120;
    let mut bad_prev = Chunk::second();
    bad_prev.prev = head(1);
    let mut bad_seq = Chunk::second();
    bad_seq.first = 5;
    let mut gap_offset = Chunk::second();
    gap_offset.offset = 400;
    for bad in [bad_offset, bad_prev, bad_seq, gap_offset] {
        let r = rig.put(&run, &entry, &bad).unwrap();
        assert_err(&r, 409, "conflict");
    }
    assert_eq!(rig.server.position(&run).unwrap(), before);
    assert_eq!(rig.server.chunks(&run).len(), 1);
}

#[test]
fn a_retried_chunk_is_answered_from_the_stored_ack_without_its_body() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let c1 = Chunk::first();
    let first = rig.put_ok(&run, &entry, &c1);
    rig.put_ok(&run, &entry, &Chunk::second());

    // The retry of the first chunk, with a body that would fail the digest if it were read.
    let mut retry = c1.clone();
    retry.body = b"not the bytes".to_vec();
    let again = rig.put_ok(&run, &entry, &retry);
    assert_eq!(again.body, first.body);
    assert_eq!(rig.server.chunks(&run).len(), 2);

    // The same offset with a different length or head is a conflict.
    let mut other_len = c1.clone();
    other_len.length = 101;
    assert_err(&rig.put(&run, &entry, &other_len).unwrap(), 409, "conflict");
    let mut other_head = c1.clone();
    other_head.head = head(77);
    assert_err(
        &rig.put(&run, &entry, &other_head).unwrap(),
        409,
        "conflict",
    );
}

#[test]
fn a_tampered_body_is_422_and_nothing_is_kept() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let mut c = Chunk::first();
    c.body.push(b'!');
    let r = rig.put(&run, &entry, &c).unwrap();
    assert_err(&r, 422, "digest_mismatch");
    assert_eq!(rig.server.position(&run).unwrap().offset, 0);
    assert!(rig.server.chunks(&run).is_empty());
    // Not even a temp file survives.
    let dir = rig._dir.path().join(&run).join(&entry);
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    // The honest chunk still goes through afterwards.
    rig.put_ok(&run, &entry, &Chunk::first());
}

#[test]
fn chunk_header_and_size_checks_come_before_the_body() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let url = rig.url(&format!("/v1/runs/{run}/chunks/0"));
    let auth = format!("Bearer {TOKEN}");

    // A missing Goalcup header is malformed.
    let r = call(
        "PUT",
        &url,
        Some(TOKEN),
        &[("Goalcup-Entry-Id", entry.clone())],
        b"x",
    )
    .unwrap();
    assert_err(&r, 400, "malformed");

    // A Content-Length over 32 MiB is refused from the headers alone: send no body at all.
    let mut tcp = TcpStream::connect(("127.0.0.1", rig.server.port())).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let c = Chunk::first();
    let request = format!(
        "PUT /v1/runs/{run}/chunks/0 HTTP/1.1\r\nHost: x\r\nAuthorization: {auth}\r\n\
         Content-Length: 33554433\r\nGoalcup-Entry-Id: {entry}\r\nGoalcup-Length: 1\r\n\
         Goalcup-Sha256: {}\r\nGoalcup-First-Seq: 0\r\nGoalcup-Last-Seq: 0\r\n\
         Goalcup-Prev: {GENESIS}\r\nGoalcup-Head: {}\r\n\r\n",
        c.sha, c.head
    );
    tcp.write_all(request.as_bytes()).unwrap();
    let mut got = vec![0u8; 12];
    tcp.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"HTTP/1.1 413");

    // Chunks for another entry are not this run's.
    let r = rig.put(&run, "e0000000000000000", &c).unwrap();
    assert_err(&r, 404, "not_found");
}

#[test]
fn abandon_only_before_any_chunk() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let path = format!("/v1/runs/{run}/abandon");
    let too_long = rig.json("POST", &path, TOKEN, &json!({"reason": "x".repeat(501)}));
    assert_err(&too_long, 400, "malformed");
    let r = rig.json("POST", &path, TOKEN, &json!({"reason": "daemon refused"}));
    assert_eq!((r.status, &r.body), (200, &json!({"status": "abandoned"})));
    assert_eq!(rig.get(&run, TOKEN).body["status"], "abandoned");
    // The account is free again, and the abandoned run takes no chunks.
    assert_err(
        &rig.put(&run, &entry, &Chunk::first()).unwrap(),
        410,
        "closed",
    );
    let next = rig.open(TOKEN, "session-b");
    assert_eq!(next.status, 201);

    // A run with a stored chunk cannot be abandoned.
    let run2 = next.body["run_id"].as_str().unwrap().to_string();
    let entry2 = next.body["entry_id"].as_str().unwrap().to_string();
    rig.put_ok(&run2, &entry2, &Chunk::first());
    let r = rig.json(
        "POST",
        &format!("/v1/runs/{run2}/abandon"),
        TOKEN,
        &json!({"reason": "x"}),
    );
    assert_err(&r, 409, "conflict");
    assert_eq!(rig.server.status(&run2).as_deref(), Some("open"));
}

#[test]
fn seal_stores_both_documents_and_seals_the_entry() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    rig.put_ok(&run, &entry, &Chunk::first());
    let path = format!("/v1/runs/{run}/seal");
    let docs = json!({"entry_id": entry, "seal_json": "{\"a\":1}\n", "baseline_json": "{\"b\":2}"});
    // A seal needs the entry's committed end first.
    let early = rig.json("POST", &path, TOKEN, &docs);
    assert_err(&early, 409, "conflict");
    assert!(
        early.body["message"]
            .as_str()
            .unwrap()
            .contains("final heartbeat")
    );
    assert_eq!(
        rig.heartbeat(&run, &entry, 3, &head(3), true, None).status,
        200
    );
    let r = rig.json("POST", &path, TOKEN, &docs);
    // Sealed before the run's end: the run stays open, so it can be continued.
    assert_eq!(
        (r.status, &r.body),
        (200, &json!({"status": "sealed", "run_status": "open"}))
    );
    assert_eq!(
        rig.server.seal(&run),
        Some(("{\"a\":1}\n".to_string(), "{\"b\":2}".to_string()))
    );
    assert_eq!(rig.get(&run, TOKEN).body["status"], "open");

    // Same documents again: 200. Different documents: 409.
    assert_eq!(rig.json("POST", &path, TOKEN, &docs).status, 200);
    let other = json!({"entry_id": entry, "seal_json": "{}", "baseline_json": "{\"b\":2}"});
    assert_err(&rig.json("POST", &path, TOKEN, &other), 409, "conflict");

    // After the seal the entry takes no more chunks, and the run (it holds chunks) cannot be abandoned.
    assert_err(
        &rig.put(&run, &entry, &Chunk::second()).unwrap(),
        410,
        "closed",
    );
    let gone = rig.json(
        "POST",
        &format!("/v1/runs/{run}/abandon"),
        TOKEN,
        &json!({"reason": "x"}),
    );
    assert_err(&gone, 409, "conflict");
}

#[test]
fn a_stopped_server_refuses_and_a_restart_resumes_the_same_state() {
    let dir = TempDir::new().unwrap();
    let server = UploadServer::start(dir.path(), ServerOptions::default());
    let port = server.port();
    let url = server.url();
    let key = server.public_key_hex();
    let opened = call(
        "POST",
        &format!("{url}/v1/runs"),
        Some(TOKEN),
        &[],
        Rig::open_body("session-a").to_string().as_bytes(),
    )
    .unwrap();
    assert_eq!(opened.status, 201);
    let run = opened.body["run_id"].as_str().unwrap().to_string();
    let entry = opened.body["entry_id"].as_str().unwrap().to_string();
    let c1 = Chunk::first();
    let put = |url: &str, c: &Chunk| {
        let rig_headers = [
            ("Goalcup-Entry-Id", entry.clone()),
            ("Goalcup-Length", c.length.to_string()),
            ("Goalcup-Sha256", c.sha.clone()),
            ("Goalcup-First-Seq", c.first.to_string()),
            ("Goalcup-Last-Seq", c.last.to_string()),
            ("Goalcup-Prev", c.prev.clone()),
            ("Goalcup-Head", c.head.clone()),
        ];
        call(
            "PUT",
            &format!("{url}/v1/runs/{run}/chunks/{}", c.offset),
            Some(TOKEN),
            &rig_headers,
            &c.body,
        )
    };
    assert_eq!(put(&url, &c1).unwrap().status, 200);

    server.stop();
    assert!(
        call(
            "GET",
            &format!("{url}/v1/runs/{run}"),
            Some(TOKEN),
            &[],
            b""
        )
        .is_err(),
        "a stopped server must refuse connections"
    );

    let server = UploadServer::start(
        dir.path(),
        ServerOptions {
            port: Some(port),
            ..ServerOptions::default()
        },
    );
    assert_eq!(server.url(), url);
    assert_eq!(server.public_key_hex(), key);
    let g = call(
        "GET",
        &format!("{url}/v1/runs/{run}"),
        Some(TOKEN),
        &[],
        b"",
    )
    .unwrap();
    assert_eq!(g.status, 200);
    assert_eq!(
        (g.body["offset"].clone(), g.body["next_seq"].clone()),
        (json!(100), json!(4))
    );
    assert_eq!(g.body["head"], c1.head.as_str());
    // The run is still the account's open one, and the chain continues where it was.
    let again = call(
        "POST",
        &format!("{url}/v1/runs"),
        Some(TOKEN),
        &[],
        Rig::open_body("session-a").to_string().as_bytes(),
    )
    .unwrap();
    assert_eq!(
        (again.status, again.body["run_id"].as_str()),
        (200, Some(run.as_str()))
    );
    assert_eq!(put(&url, &Chunk::second()).unwrap().status, 200);
    assert_eq!(server.chunks(&run).len(), 2);
}

#[test]
fn maintenance_answers_503_with_retry_after() {
    let rig = Rig::new();
    let (run, _) = rig.open_ok();
    rig.server.set_maintenance(true);
    rig.server.set_maintenance_retry_after(42);
    for r in [
        rig.get(&run, TOKEN),
        rig.open(TOKEN_2, "s"),
        rig.get(&run, "bad-token"),
    ] {
        assert_err(&r, 503, "maintenance");
        assert_eq!(r.retry_after.as_deref(), Some("42"));
    }
    rig.server.set_maintenance(false);
    assert_eq!(rig.get(&run, TOKEN).status, 200);
}

#[test]
fn failed_chunks_store_nothing_and_then_succeed() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    rig.server.fail_next_chunks(2);
    let c = Chunk::first();
    for _ in 0..2 {
        let r = rig.put(&run, &entry, &c).unwrap();
        assert_eq!(r.status, 500);
        assert!(rig.server.chunks(&run).is_empty());
    }
    rig.put_ok(&run, &entry, &c);
    assert_eq!(rig.server.chunks(&run).len(), 1);
}

#[test]
fn a_dropped_reply_leaves_the_chunk_stored_and_the_retry_idempotent() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let c = Chunk::first();
    rig.server.drop_next_chunk_reply();
    let lost = rig.put(&run, &entry, &c);
    assert!(
        matches!(
            lost.as_ref().map_err(|e| &**e),
            Err(ureq::Error::Transport(_))
        ),
        "the reply must be lost"
    );
    // The server holds the chunk and its ack even though the caller never heard.
    assert_eq!(rig.server.chunks(&run).len(), 1);
    let stored = rig.server.last_ack(&run).expect("an ack was stored");
    let retry = rig.put_ok(&run, &entry, &c);
    let ack: AckCheckpoint = serde_json::from_value(retry.body["ack"].clone()).unwrap();
    assert_eq!(ack, stored);
    ack.verify(&rig.server.public_key_hex()).unwrap();
    assert_eq!(rig.server.chunks(&run).len(), 1);
    // The hook is one-shot.
    rig.put_ok(&run, &entry, &Chunk::second());
}

#[test]
fn the_call_log_lists_method_and_path() {
    let rig = Rig::new();
    let (run, _) = rig.open_ok();
    rig.get(&run, TOKEN);
    assert_eq!(
        rig.server.calls(),
        vec!["POST /v1/runs".to_string(), format!("GET /v1/runs/{run}")]
    );
}

// ---- the deadline ------------------------------------------------------------------------

/// A run that ends two seconds after it opens.
fn short_run() -> Rig {
    Rig::with(ServerOptions {
        run_seconds: Some(2),
        ..two_tokens()
    })
}

fn seal_body(entry: &str) -> Value {
    json!({"entry_id": entry, "seal_json": "{\"a\":1}", "baseline_json": "{\"b\":2}"})
}

#[test]
fn stop_is_false_before_the_deadline_and_true_after() {
    let rig = short_run();
    let (run, entry) = rig.open_ok();
    let opened = rig.get(&run, TOKEN);
    assert_eq!(opened.body["stop"], false, "{}", opened.body);
    assert_eq!(opened.body["final"], Value::Null);
    let (s, d) = (
        parse_z(opened.body["official_start"].as_str().unwrap()).unwrap(),
        parse_z(opened.body["deadline_at"].as_str().unwrap()).unwrap(),
    );
    assert_eq!(d - s, 2, "run_seconds overrides run_hours");

    let hb = rig.heartbeat(&run, &entry, 3, &head(3), false, None);
    assert_eq!(hb.status, 200, "{}", hb.body);
    assert_eq!(hb.body["stop"], false);
    assert_eq!(hb.body["deadline_at"], opened.body["deadline_at"]);
    // The time left on the server's clock: at most the run's 2 seconds.
    assert!(
        hb.body["seconds_left"]
            .as_u64()
            .is_some_and(|left| left <= 2),
        "{}",
        hb.body
    );

    rig.wait_for_stop(&run);
    let hb = rig.heartbeat(&run, &entry, 3, &head(3), false, None);
    assert_eq!((hb.status, &hb.body["stop"]), (200, &json!(true)));
    assert_eq!(hb.body["deadline_at"], opened.body["deadline_at"]);
    assert_eq!(hb.body["seconds_left"], 0, "nothing is left once it stops");
    // The run is still open: past the deadline is not abandoned.
    assert_eq!(rig.get(&run, TOKEN).body["status"], "open");
}

#[test]
fn a_chunk_after_the_deadline_without_a_final_is_503_with_retry_after() {
    let rig = short_run();
    let (run, entry) = rig.open_ok();
    rig.wait_for_stop(&run);
    let r = rig.put(&run, &entry, &Chunk::first()).unwrap();
    assert_err(&r, 503, "awaiting_final");
    assert_eq!(r.retry_after.as_deref(), Some("1"));
    assert_eq!(rig.server.chunks(&run).len(), 0);

    // The final heartbeat is what lets the upload through.
    let hb = rig.heartbeat(&run, &entry, 3, &head(3), true, None);
    assert_eq!(hb.status, 200, "{}", hb.body);
    rig.put_ok(&run, &entry, &Chunk::first());
}

#[test]
fn a_final_commits_the_end_and_chunks_past_it_are_closed() {
    let rig = short_run();
    let (run, entry) = rig.open_ok();
    let hb = rig.heartbeat(&run, &entry, 5, &head(5), true, None);
    assert_eq!(hb.status, 200, "{}", hb.body);
    assert_eq!(hb.body["stop"], false, "early final: the deadline is ahead");
    let g = rig.get(&run, TOKEN).body;
    assert_eq!(g["final"], json!({"seq": 5, "head": head(5)}));

    // Before the deadline, up to the committed end is taken ...
    rig.put_ok(&run, &entry, &Chunk::first());
    // ... and so is a chunk ending exactly at it with the committed head, after the deadline too.
    rig.wait_for_stop(&run);
    rig.put_ok(&run, &entry, &Chunk::second());
    // Past it: 410, and nothing is stored.
    let past = Chunk::new(150, 50, 6, 7, &head(5));
    assert_err(&rig.put(&run, &entry, &past).unwrap(), 410, "closed");
    assert_eq!(rig.server.chunks(&run).len(), 2);
}

#[test]
fn a_chunk_past_the_committed_end_is_closed_before_the_deadline_too() {
    let rig = Rig::new(); // 168 h: the deadline is far away
    let (run, entry) = rig.open_ok();
    rig.heartbeat(&run, &entry, 3, &head(3), true, None);
    rig.put_ok(&run, &entry, &Chunk::first());
    assert_err(
        &rig.put(&run, &entry, &Chunk::second()).unwrap(),
        410,
        "closed",
    );
    assert_eq!(rig.get(&run, TOKEN).body["stop"], false);
}

#[test]
fn a_chunk_ending_at_the_committed_seq_with_another_head_is_closed() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    rig.heartbeat(&run, &entry, 3, &head(3), true, None);
    let mut wrong = Chunk::first();
    wrong.head = sha_hex(b"not the committed head");
    assert_err(&rig.put(&run, &entry, &wrong).unwrap(), 410, "closed");
    assert_eq!(rig.server.chunks(&run).len(), 0);
    rig.put_ok(&run, &entry, &Chunk::first());
}

#[test]
fn a_seal_after_the_deadline_needs_the_chain_up_to_the_committed_end() {
    let rig = short_run();
    let (run, entry) = rig.open_ok();
    rig.heartbeat(&run, &entry, 5, &head(5), true, None);
    rig.put_ok(&run, &entry, &Chunk::first());
    rig.wait_for_stop(&run);
    let path = format!("/v1/runs/{run}/seal");
    // Held up to seq 3 only: short of the committed 5.
    assert_err(
        &rig.json("POST", &path, TOKEN, &seal_body(&entry)),
        409,
        "conflict",
    );
    assert_eq!(rig.get(&run, TOKEN).body["status"], "open");
    rig.put_ok(&run, &entry, &Chunk::second());
    // The run is over and this is its current entry: the run is sealed with it.
    let r = rig.json("POST", &path, TOKEN, &seal_body(&entry));
    assert_eq!(
        (r.status, &r.body),
        (200, &json!({"status": "sealed", "run_status": "sealed"}))
    );
    assert_eq!(rig.get(&run, TOKEN).body["status"], "sealed");
    // A repeat is still 200.
    assert_eq!(
        rig.json("POST", &path, TOKEN, &seal_body(&entry)).status,
        200
    );
}

#[test]
fn a_seal_after_the_deadline_with_no_final_waits_for_it() {
    let rig = short_run();
    let (run, entry) = rig.open_ok();
    rig.wait_for_stop(&run);
    let r = rig.json(
        "POST",
        &format!("/v1/runs/{run}/seal"),
        TOKEN,
        &seal_body(&entry),
    );
    assert_err(&r, 503, "awaiting_final");
    assert_eq!(r.retry_after.as_deref(), Some("1"));
}

#[test]
fn a_final_heartbeat_is_idempotent_and_never_changes() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    let repo = "0123456789abcdef0123456789abcdef01234567";
    let first = rig.heartbeat(&run, &entry, 5, &head(5), true, Some(repo));
    assert_eq!(first.status, 200, "{}", first.body);
    // The same (seq, head) again: 200.
    assert_eq!(
        rig.heartbeat(&run, &entry, 5, &head(5), true, Some(repo))
            .status,
        200
    );
    // A different seq, or a different head at the same seq: 409, nothing changes.
    assert_err(
        &rig.heartbeat(&run, &entry, 6, &head(6), true, None),
        409,
        "conflict",
    );
    assert_err(
        &rig.heartbeat(&run, &entry, 5, &head(99), true, None),
        409,
        "conflict",
    );
    assert_eq!(
        rig.server
            .final_commit(&run)
            .map(|f| (f.seq, f.head, f.repo_head)),
        Some((5, head(5), Some(repo.to_string())))
    );
    // A plain heartbeat after the final is not a second commit.
    assert_eq!(
        rig.heartbeat(&run, &entry, 7, &head(7), false, None).status,
        200
    );
    assert_eq!(rig.get(&run, TOKEN).body["final"]["seq"], 5);
}

#[test]
fn a_sealed_entry_takes_the_same_final_again_and_no_plain_heartbeat() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    rig.put_ok(&run, &entry, &Chunk::first());
    assert_eq!(
        rig.heartbeat(&run, &entry, 3, &head(3), true, None).status,
        200
    );
    let sealed = rig.json(
        "POST",
        &format!("/v1/runs/{run}/seal"),
        TOKEN,
        &seal_body(&entry),
    );
    assert_eq!(sealed.status, 200);
    // A repeat of the final is the same answer; a plain one has nothing left to say (and is not kept).
    assert_eq!(
        rig.heartbeat(&run, &entry, 3, &head(3), true, None).status,
        200
    );
    let before = rig.server.heartbeats(&run).len();
    assert_eq!(
        rig.heartbeat(&run, &entry, 3, &head(3), false, None).status,
        200
    );
    assert_eq!(rig.server.heartbeats(&run).len(), before);
    assert_err(
        &rig.heartbeat(&run, &entry, 4, &head(4), true, None),
        409,
        "conflict",
    );
}

#[test]
fn heartbeats_are_validated() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    assert_err(
        &rig.heartbeat(&run, &entry, 1, "abc", false, None),
        400,
        "malformed",
    );
    assert_err(
        &rig.heartbeat(&run, &entry, 1, &head(1), true, Some("not-a-commit")),
        400,
        "malformed",
    );
    assert_err(
        &rig.heartbeat(&run, "e0000000000000000", 1, &head(1), false, None),
        404,
        "not_found",
    );
    let path = format!("/v1/runs/{run}/heartbeat");
    assert_err(
        &rig.json("POST", &path, TOKEN, &json!({"entry_id": entry})),
        400,
        "malformed",
    );
    assert_err(
        &rig.json(
            "POST",
            &path,
            TOKEN_2,
            &json!({"entry_id": entry, "seq": 1, "head": head(1), "offset_local": 1,
                    "final": false, "repo_head": null}),
        ),
        404,
        "not_found",
    );
    assert!(rig.server.heartbeats(&run).is_empty());
}

#[test]
fn the_heartbeat_helper_sees_the_heartbeats() {
    let rig = Rig::new();
    let (run, entry) = rig.open_ok();
    assert!(rig.server.heartbeats(&run).is_empty());
    rig.heartbeat(&run, &entry, 3, &head(3), false, None);
    rig.heartbeat(&run, &entry, 5, &head(5), false, None);
    let repo = "fedcba9876543210fedcba9876543210fedcba98";
    rig.heartbeat(&run, &entry, 5, &head(5), true, Some(repo));
    let seen: Vec<_> = rig
        .server
        .heartbeats(&run)
        .into_iter()
        .map(|h| (h.seq, h.head, h.offset_local, h.final_, h.repo_head))
        .collect();
    assert_eq!(
        seen,
        vec![
            (3, head(3), 150, false, None),
            (5, head(5), 150, false, None),
            (5, head(5), 150, true, Some(repo.to_string())),
        ]
    );
    assert!(parse_z(&rig.server.heartbeats(&run)[0].received_at).is_some());
    assert!(rig.server.heartbeats("r0000000000000000").is_empty());
    assert!(
        rig.server
            .calls()
            .iter()
            .any(|c| c == &format!("POST /v1/runs/{run}/heartbeat"))
    );
}

#[test]
fn an_unsealed_run_is_incomplete_after_the_close_time_and_takes_nothing() {
    let rig = Rig::with(ServerOptions {
        run_seconds: Some(1),
        close_after_seconds: 1,
        ..two_tokens()
    });
    let (run, entry) = rig.open_ok();
    rig.put_ok(&run, &entry, &Chunk::first());
    // Over at 1 s, closed after 1 s more.
    let mut status = String::new();
    for _ in 0..100 {
        status = rig.get(&run, TOKEN).body["status"]
            .as_str()
            .unwrap()
            .to_string();
        if status == "incomplete" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "incomplete");
    assert_eq!(rig.server.status(&run).as_deref(), Some("incomplete"));

    // Every write is 410; the read still answers.
    assert_err(
        &rig.put(&run, &entry, &Chunk::second()).unwrap(),
        410,
        "closed",
    );
    assert_err(
        &rig.heartbeat(&run, &entry, 3, &head(3), true, None),
        410,
        "closed",
    );
    assert_err(
        &rig.json(
            "POST",
            &format!("/v1/runs/{run}/seal"),
            TOKEN,
            &seal_body(&entry),
        ),
        410,
        "closed",
    );
    assert_err(
        &rig.json(
            "POST",
            &format!("/v1/runs/{run}/receipt"),
            TOKEN,
            &json!({"entry_id": entry, "chain_head": head(0)}),
        ),
        410,
        "closed",
    );
    assert_err(
        &rig.json(
            "POST",
            &format!("/v1/runs/{run}/abandon"),
            TOKEN,
            &json!({"reason": "x"}),
        ),
        410,
        "closed",
    );
    assert_eq!(rig.get(&run, TOKEN).status, 200);
    // A chunk the server already held is still answered with its stored acknowledgement.
    assert_eq!(rig.put(&run, &entry, &Chunk::first()).unwrap().status, 200);
}

#[test]
fn a_sealed_run_stays_sealed_when_it_closes() {
    // The stand-in's clock counts whole seconds, so a one-second run is over somewhere between
    // 0 and 1 s after the open, whatever the load. The setup below never depends on which: the
    // final heartbeat comes first, and once the end is committed the chunk and the seal are
    // taken whether the run is over or not. Only the close ends it, `close_after_seconds` later,
    // and 2 s of that leaves the setup at least 2 s however slow the machine is.
    let rig = Rig::with(ServerOptions {
        run_seconds: Some(1),
        close_after_seconds: 2,
        ..two_tokens()
    });
    let (run, entry) = rig.open_ok();
    assert_eq!(
        rig.heartbeat(&run, &entry, 3, &head(3), true, None).status,
        200
    );
    rig.put_ok(&run, &entry, &Chunk::first());
    let sealed = rig.json(
        "POST",
        &format!("/v1/runs/{run}/seal"),
        TOKEN,
        &seal_body(&entry),
    );
    assert_eq!(sealed.status, 200, "{}", sealed.body);

    // Wait for the close itself, not a time that should be after it: a repeat of the committed
    // final heartbeat is answered 200 until the run closes, and 410 from then on.
    let mut closed = false;
    for _ in 0..150 {
        let repeat = rig.heartbeat(&run, &entry, 3, &head(3), true, None);
        if repeat.status == 410 {
            assert_eq!(repeat.body["error"], "closed", "{}", repeat.body);
            closed = true;
            break;
        }
        assert_eq!(repeat.status, 200, "{}", repeat.body);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(closed, "the run never closed");
    assert_eq!(rig.get(&run, TOKEN).body["status"], "sealed");
}

#[test]
fn the_deadline_state_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    let options = || ServerOptions {
        run_seconds: Some(3600),
        ..ServerOptions::default()
    };
    let server = UploadServer::start(dir.path(), options());
    let r = call(
        "POST",
        &format!("{}/v1/runs", server.url()),
        Some(TOKEN),
        &[],
        Rig::open_body("session-a").to_string().as_bytes(),
    )
    .unwrap();
    let run = r.body["run_id"].as_str().unwrap().to_string();
    let entry = r.body["entry_id"].as_str().unwrap().to_string();
    let hb = |server: &UploadServer, seq: u64, head: &str, fin: bool| {
        call(
            "POST",
            &format!("{}/v1/runs/{run}/heartbeat", server.url()),
            Some(TOKEN),
            &[],
            json!({"entry_id": entry, "seq": seq, "head": head, "offset_local": 9,
                   "final": fin, "repo_head": null})
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    };
    assert_eq!(hb(&server, 2, &head(2), false).status, 200);
    assert_eq!(hb(&server, 5, &head(5), true).status, 200);
    server.stop();

    let server = UploadServer::start(dir.path(), options());
    assert_eq!(server.heartbeats(&run).len(), 2);
    assert_eq!(server.final_commit(&run).map(|f| f.seq), Some(5));
    assert_eq!(hb(&server, 5, &head(5), true).status, 200);
    assert_eq!(hb(&server, 6, &head(6), true).status, 409);
}
