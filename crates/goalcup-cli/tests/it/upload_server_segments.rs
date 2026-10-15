//! The stand-in's run lifecycle against `UPLOAD-API.md`: one main run per account (a later start
//! replaces the earlier), practice runs, the start window, `continue` and its segments, and the
//! close 24 hours after a run ends. The Worker's own tests (`server/worker/test/lifecycle.test.ts`
//! and `segments.test.ts`) hold the same rules, so the tracer's tests can trust this stand-in.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::control::parse_z;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::common::upload_server::{ServerOptions, UploadServer};

struct Reply {
    status: u16,
    body: Value,
    retry_after: Option<String>,
}

fn call(
    method: &str,
    url: &str,
    token: Option<&str>,
    extra: &[(&str, String)],
    body: Option<&Value>,
    raw: &[u8],
) -> Reply {
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
    let result = match body {
        Some(b) => req
            .set("Content-Type", "application/json")
            .send_string(&b.to_string()),
        None if method == "GET" => req.call(),
        None => req.send_bytes(raw),
    };
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("transport: {e}"),
    };
    let status = resp.status();
    let retry_after = resp.header("Retry-After").map(str::to_string);
    let text = resp.into_string().expect("read the reply body");
    let body = serde_json::from_str(&text).unwrap_or_else(|_| panic!("reply is not JSON: {text}"));
    Reply {
        status,
        body,
        retry_after,
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn head(n: u64) -> String {
    sha_hex(format!("record-{n}").as_bytes())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn is_error(r: &Reply, status: u16, code: &str) {
    assert_eq!(r.status, status, "{}", r.body);
    assert_eq!(r.body["error"], code, "{}", r.body);
    assert!(r.body["message"].is_string(), "{}", r.body);
}

/// An opened run: its ids and the credential that opened it.
#[derive(Clone)]
struct Opened {
    token: String,
    run: String,
    entry: String,
    deadline_at: String,
}

struct Rig {
    dir: TempDir,
    server: UploadServer,
}

/// The keys of an object, in sorted order, however the JSON library orders them.
/// A heartbeat answer without its `seconds_left`, and that number: the clock moves while a test
/// runs, so a test that is not about the time left compares the rest exactly.
fn split_left(answer: &Value) -> (Value, u64) {
    let mut rest = answer.clone();
    let left = rest
        .as_object_mut()
        .unwrap()
        .remove("seconds_left")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| panic!("no seconds_left in {answer}"));
    (rest, left)
}

fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

impl Rig {
    fn new() -> Rig {
        Rig::with(ServerOptions::default())
    }

    fn with(options: ServerOptions) -> Rig {
        let dir = TempDir::new().expect("tempdir");
        let server = UploadServer::start(dir.path(), options);
        Rig { dir, server }
    }

    fn send(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> Reply {
        call(
            method,
            &format!("{}{path}", self.server.url()),
            Some(token),
            &[],
            body,
            b"",
        )
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

    fn open_with(&self, token: &str, body: &Value, extra: &[(&str, String)]) -> Reply {
        call(
            "POST",
            &format!("{}/v1/runs", self.server.url()),
            Some(token),
            extra,
            Some(body),
            b"",
        )
    }

    fn open(&self, token: &str, session: &str) -> Reply {
        self.open_with(token, &Rig::open_body(session), &[])
    }

    fn start(&self, token: &str, session: &str) -> Opened {
        let r = self.open(token, session);
        assert_eq!(r.status, 201, "{}", r.body);
        Opened {
            token: token.to_string(),
            run: r.body["run_id"].as_str().unwrap().to_string(),
            entry: r.body["entry_id"].as_str().unwrap().to_string(),
            deadline_at: r.body["deadline_at"].as_str().unwrap().to_string(),
        }
    }

    fn get(&self, o: &Opened, entry: Option<&str>) -> Reply {
        let query = entry.map_or(String::new(), |e| format!("?entry_id={e}"));
        self.send("GET", &format!("/v1/runs/{}{query}", o.run), &o.token, None)
    }

    fn beat(&self, o: &Opened, entry: &str, seq: u64, head: &str, final_: bool) -> Reply {
        self.send(
            "POST",
            &format!("/v1/runs/{}/heartbeat", o.run),
            &o.token,
            Some(
                &json!({"entry_id": entry, "seq": seq, "head": head, "offset_local": 1,
                         "final": final_, "repo_head": null}),
            ),
        )
    }

    fn seal(&self, o: &Opened, entry: &str) -> Reply {
        self.send(
            "POST",
            &format!("/v1/runs/{}/seal", o.run),
            &o.token,
            Some(
                &json!({"entry_id": entry, "seal_json": "{\"a\":1}", "baseline_json": "{\"b\":2}"}),
            ),
        )
    }

    fn cont(&self, o: &Opened, previous: &str, session: &str) -> Reply {
        self.cont_with(o, previous, session, &[])
    }

    fn cont_with(
        &self,
        o: &Opened,
        previous: &str,
        session: &str,
        extra: &[(&str, String)],
    ) -> Reply {
        call(
            "POST",
            &format!("{}/v1/runs/{}/continue", self.server.url(), o.run),
            Some(&o.token),
            extra,
            Some(&json!({
                "previous_entry_id": previous,
                "connector_id": "other-cli",
                "manifest_sha256": sha_hex(b"manifest-2"),
                "connector_dir_sha256": sha_hex(b"dir-2"),
                "workspace": "/tmp/run-1/two",
                "session_id": session,
                "harness_version": "9",
                "tracer_version": "0.2.0",
            })),
            b"",
        )
    }

    /// A chunk of an entry: records `first..=last`, continuing from `prev`, at byte `offset`.
    fn put(&self, o: &Opened, entry: &str, c: &Chunk) -> Reply {
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
            &format!(
                "{}/v1/runs/{}/chunks/{}",
                self.server.url(),
                o.run,
                c.offset
            ),
            Some(&o.token),
            &extra,
            None,
            &c.body,
        )
    }
}

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

    /// Records 0..=3 from the genesis.
    fn first() -> Chunk {
        Chunk::new(0, 100, 0, 3, GENESIS)
    }

    /// The chunk that follows `first()`.
    fn second() -> Chunk {
        Chunk::new(100, 50, 4, 5, &head(3))
    }
}

#[test]
fn a_second_competition_start_replaces_the_first_and_its_heartbeat_says_stop_replaced() {
    let rig = Rig::new();
    let [c1, c2, c3] = [(); 3].map(|_| rig.server.mint_credential("member-1"));
    let other = rig.server.mint_credential("member-2");
    let bystander = rig.start(&other, "bystander");
    let a = rig.start(&c1, "s1");
    let b = rig.start(&c2, "s2"); // was 409 "already has an open run"
    assert_ne!(a.run, b.run);

    let hb = rig.beat(&a, &a.entry, 0, GENESIS, false);
    assert_eq!(hb.status, 200, "{}", hb.body);
    // The time left counts to `deadline_at`, which a replacement does not move; `stop` ends it.
    let (rest, left) = split_left(&hb.body);
    assert_eq!(
        rest,
        json!({"stop": true, "deadline_at": a.deadline_at, "reason": "replaced"})
    );
    assert!(left > 0, "{}", hb.body);
    let pos = rig.get(&a, None).body;
    assert_eq!(
        (pos["stop"].clone(), pos["status"].clone()),
        (json!(true), json!("open"))
    );
    assert_eq!(pos["deadline_at"], a.deadline_at.as_str());
    assert_eq!(
        pos["ends_at"],
        rig.server.replaced_at(&a.run).unwrap().as_str()
    );
    let (rest, left) = split_left(&rig.beat(&b, &b.entry, 0, GENESIS, false).body);
    assert_eq!(
        rest,
        json!({"stop": false, "deadline_at": b.deadline_at, "reason": null})
    );
    assert!(left > 168 * 3600 - 60 && left <= 168 * 3600, "{left}");
    assert_eq!(rig.get(&b, None).body["ends_at"], b.deadline_at.as_str());
    assert_eq!(
        rig.beat(&bystander, &bystander.entry, 0, GENESIS, false)
            .body["stop"],
        false
    );

    // A lost reply is the same run; a third start replaces the second and leaves the first's end.
    let again = rig.open(&c2, "s2");
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(again.body["run_id"], b.run.as_str());
    let first_end = rig.server.replaced_at(&a.run);
    let c = rig.start(&c3, "s3");
    assert_eq!(rig.server.replaced_at(&a.run), first_end);
    assert!(rig.server.replaced_at(&b.run).is_some());
    assert_eq!(rig.server.replaced_at(&c.run), None);
    assert_eq!(rig.server.replaced_at(&bystander.run), None);
    assert_eq!(rig.server.mode(&a.run).as_deref(), Some("competition"));
}

#[test]
fn the_replaced_run_still_takes_its_wrap_up_until_it_closes() {
    let rig = Rig::new();
    let [c1, c2] = [(); 2].map(|_| rig.server.mint_credential("member-1"));
    let a = rig.start(&c1, "s1");
    let first = rig.put(&a, &a.entry, &Chunk::first());
    assert_eq!(first.status, 200, "{}", first.body);
    rig.start(&c2, "s2");

    // Over, so a chunk waits for the final heartbeat; one the server holds is answered at once.
    let wait = rig.put(&a, &a.entry, &Chunk::second());
    is_error(&wait, 503, "awaiting_final");
    assert_eq!(wait.retry_after.as_deref(), Some("1"));
    let retry = rig.put(&a, &a.entry, &Chunk::first());
    assert_eq!(retry.status, 200, "{}", retry.body);
    assert_eq!(retry.body["ack"], first.body["ack"]);

    assert_eq!(
        rig.beat(&a, &a.entry, 5, &head(5), true).body["reason"],
        "replaced"
    );
    assert_eq!(rig.put(&a, &a.entry, &Chunk::second()).status, 200);
    is_error(
        &rig.put(&a, &a.entry, &Chunk::new(150, 50, 6, 7, &head(5))),
        410,
        "closed",
    );
    let sealed = rig.seal(&a, &a.entry);
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    assert_eq!(
        sealed.body,
        json!({"status": "sealed", "run_status": "sealed"})
    );
    assert_eq!(rig.get(&a, None).body["status"], "sealed");
}

#[test]
fn practice_never_replaces_and_has_no_window_and_its_own_length() {
    let now = now();
    let rig = Rig::with(ServerOptions {
        start_window_opens: Some(now + 3600), // a competition start is not yet possible
        practice_run_seconds: 600,
        ..ServerOptions::default()
    });
    let [p1c, p2c] = [(); 2].map(|_| rig.server.mint_practice_credential("member-1"));
    let p1 = rig.start(&p1c, "p1");
    let p2 = rig.start(&p2c, "p2");
    let (s, d) = (
        parse_z(rig.get(&p1, None).body["official_start"].as_str().unwrap()).unwrap(),
        parse_z(&p1.deadline_at).unwrap(),
    );
    assert_eq!(d - s, 600);
    for p in [&p1, &p2] {
        assert_eq!(rig.beat(p, &p.entry, 0, GENESIS, false).body["stop"], false);
        assert_eq!(rig.server.replaced_at(&p.run), None);
        assert_eq!(rig.server.mode(&p.run).as_deref(), Some("practice"));
    }
}

#[test]
fn a_test_start_replaces_only_an_open_test_run_and_a_competition_start_leaves_it() {
    let rig = Rig::new();
    let [t1c, t2c] = [(); 2].map(|_| rig.server.mint_test_credential("member-1"));
    let [c1, c2] = [(); 2].map(|_| rig.server.mint_credential("member-1"));
    let t1 = rig.start(&t1c, "t1");
    assert_eq!(rig.server.mode(&t1.run).as_deref(), Some("test"));
    // A competition start does not end the test run, and a test start does not end it either
    // until it is itself a test start.
    let c1 = rig.start(&c1, "c1");
    assert_eq!(rig.server.replaced_at(&t1.run), None);
    assert_eq!(
        rig.beat(&t1, &t1.entry, 0, GENESIS, false).body["stop"],
        false
    );
    // A test start replaces the open test run, and only that one.
    let t2 = rig.start(&t2c, "t2");
    assert!(rig.server.replaced_at(&t1.run).is_some());
    assert_eq!(rig.server.replaced_at(&c1.run), None);
    assert_eq!(rig.server.replaced_at(&t2.run), None);
    // And a second competition start replaces the competition run, not the test one.
    let c2 = rig.start(&c2, "c2");
    assert!(rig.server.replaced_at(&c1.run).is_some());
    assert_eq!(rig.server.replaced_at(&t2.run), None);
    assert_eq!(rig.server.replaced_at(&c2.run), None);
}

#[test]
fn a_practice_run_ends_after_its_length() {
    let rig = Rig::with(ServerOptions {
        practice_run_seconds: 1,
        ..ServerOptions::default()
    });
    let credential = rig.server.mint_practice_credential("member-1");
    let p = rig.start(&credential, "p1");
    assert_eq!(
        rig.beat(&p, &p.entry, 0, GENESIS, false).body["stop"],
        false
    );
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(
        rig.beat(&p, &p.entry, 0, GENESIS, false).body,
        json!({"stop": true, "deadline_at": p.deadline_at, "seconds_left": 0, "reason": "deadline"})
    );
}

#[test]
fn a_competition_start_is_403_outside_the_window_and_continue_still_works() {
    let now = now();
    let rig = Rig::with(ServerOptions {
        start_window_opens: Some(now - 3600),
        start_window_closes: Some(now + 2),
        ..ServerOptions::default()
    });
    let alice = rig.server.mint_credential("member-1");
    let a = rig.start(&alice, "s1"); // inside
    std::thread::sleep(Duration::from_millis(3100));

    let bob = rig.server.mint_credential("member-2");
    let late = rig.open(&bob, "late");
    is_error(&late, 403, "forbidden");
    assert_eq!(
        late.body["message"],
        "after the start window you can only `goalcup continue` your run"
    );
    // The credential stays unbound, practice has no window, and the run goes on with continue.
    let practice = rig.server.mint_practice_credential("member-2");
    assert_eq!(rig.open(&practice, "practice-1").status, 201);
    let c = rig.cont(&a, &a.entry, "s-two");
    assert_eq!(c.status, 201, "{}", c.body);
    assert_eq!(c.body["segment"], 1);
}

#[test]
fn a_competition_start_before_the_window_opens_is_403() {
    let rig = Rig::with(ServerOptions {
        start_window_opens: Some(now() + 3600),
        ..ServerOptions::default()
    });
    let token = rig.server.mint_credential("member-1");
    let r = rig.open(&token, "early");
    is_error(&r, 403, "forbidden");
    assert!(r.body["message"].as_str().unwrap().contains("not opened"));
    assert!(rig.server.run_ids().is_empty());
}

#[test]
fn continue_mints_segment_one_with_its_own_entry_and_receipt() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    let receipt_of = |o: &Opened, entry: &str, chain_head: &str| {
        rig.send(
            "POST",
            &format!("/v1/runs/{}/receipt", o.run),
            &o.token,
            Some(&json!({"entry_id": entry, "chain_head": chain_head})),
        )
    };
    let first = receipt_of(&a, &a.entry, &head(0));
    assert_eq!(first.status, 200, "{}", first.body);

    let c = rig.cont(&a, &a.entry, "s-two");
    assert_eq!(c.status, 201, "{}", c.body);
    assert_eq!(
        sorted_keys(&c.body),
        [
            "deadline_at",
            "entry_id",
            "official_start",
            "seconds_left",
            "segment"
        ]
    );
    assert!(c.body["seconds_left"].as_u64().unwrap() <= 168 * 3600);
    assert_ne!(c.body["entry_id"], a.entry.as_str());
    assert_eq!(c.body["segment"], 1);
    let start = rig.get(&a, None).body["official_start"].clone();
    assert_eq!(c.body["official_start"], start);
    assert_eq!(c.body["deadline_at"], a.deadline_at.as_str());
    let e1 = c.body["entry_id"].as_str().unwrap().to_string();

    let second = receipt_of(&a, &e1, &head(9));
    assert_eq!(second.status, 200, "{}", second.body);
    let claims = &second.body["start_receipt"]["claims"];
    assert_eq!(claims["entry_id"], e1.as_str());
    assert_eq!(claims["connector_id"], "other-cli");
    assert_eq!(claims["workspace"], "/tmp/run-1/two");
    assert_eq!(claims["session_id"], "s-two");
    assert_eq!(claims["started_at"], start);
    assert_eq!(claims["deadline_at"], a.deadline_at.as_str());
    assert_eq!(claims["manifest_sha256"], sha_hex(b"manifest-2").as_str());
    assert_eq!(claims["chain_head"], head(9).as_str());
    // Segment 0 keeps its own, and the accessors see each.
    assert_eq!(
        receipt_of(&a, &a.entry, &head(0)).body["start_receipt"],
        first.body["start_receipt"]
    );
    assert_eq!(
        rig.server.receipt_of(&e1).unwrap().claims.session_id,
        "s-two"
    );
    assert_eq!(
        rig.server.entry_ids(&a.run),
        vec![a.entry.clone(), e1.clone()]
    );

    // The position: the current entry, or the one asked for.
    let now = rig.get(&a, None).body;
    assert_eq!(
        (now["entry_id"].clone(), now["segment"].clone()),
        (json!(e1), json!(1))
    );
    let old = rig.get(&a, Some(&a.entry)).body;
    assert_eq!(
        (old["entry_id"].clone(), old["segment"].clone()),
        (json!(a.entry), json!(0))
    );
    assert_eq!(
        sorted_keys(&old),
        [
            "deadline_at",
            "ends_at",
            "entry_id",
            "final",
            "head",
            "last_ack",
            "next_seq",
            "official_start",
            "offset",
            "run_id",
            "segment",
            "status",
            "stop"
        ]
    );
    is_error(&rig.get(&a, Some("e7777777777777777")), 404, "not_found");
    is_error(&rig.get(&a, Some("nope")), 400, "malformed");

    // The same call again is the same answer.
    let repeat = rig.cont(&a, &a.entry, "s-two");
    assert_eq!(repeat.status, 200, "{}", repeat.body);
    assert_eq!(repeat.body, c.body);
    assert_eq!(rig.server.entry_ids(&a.run).len(), 2);
}

#[test]
fn a_lost_reply_repeat_of_the_open_is_the_same_run_also_after_continue() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    assert_eq!(rig.cont(&a, &a.entry, "s-two").status, 201);
    let again = rig.open(&token, "s1");
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(again.body["run_id"], a.run.as_str());
    assert_eq!(again.body["entry_id"], a.entry.as_str()); // segment 0's entry
    assert_eq!(again.body["deadline_at"], a.deadline_at.as_str());
    // The session a continue brought is not the open call's.
    is_error(&rig.open(&token, "s-two"), 409, "conflict");
}

#[test]
fn a_stale_previous_entry_id_is_409_and_a_run_that_ended_is_410() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    let c1 = rig.cont(&a, &a.entry, "s-two");
    assert_eq!(c1.status, 201, "{}", c1.body);
    is_error(&rig.cont(&a, &a.entry, "s-three"), 409, "conflict");
    is_error(
        &rig.cont(&a, "e5555555555555555", "s-three"),
        409,
        "conflict",
    );
    is_error(&rig.cont(&a, "not-an-entry", "s-three"), 400, "malformed");
    is_error(
        &rig.cont(&a, c1.body["entry_id"].as_str().unwrap(), ""),
        400,
        "malformed",
    );
    let c2 = rig.cont(&a, c1.body["entry_id"].as_str().unwrap(), "s-three");
    assert_eq!(c2.status, 201, "{}", c2.body);
    assert_eq!(c2.body["segment"], 2);
    let stranger = Opened {
        token: rig.server.mint_credential("member-2"),
        ..a.clone()
    };
    is_error(
        &rig.cont(&stranger, c2.body["entry_id"].as_str().unwrap(), "x"),
        404,
        "not_found",
    );

    // A replaced run is over.
    let other = rig.server.mint_credential("member-1");
    rig.start(&other, "s9");
    is_error(
        &rig.cont(&a, c2.body["entry_id"].as_str().unwrap(), "s-four"),
        410,
        "closed",
    );
    // A run that reached its deadline too.
    let short = Rig::with(ServerOptions {
        run_seconds: Some(1),
        ..ServerOptions::default()
    });
    let t = short.server.mint_credential("member-1");
    let s = short.start(&t, "s1");
    std::thread::sleep(Duration::from_millis(1100));
    is_error(&short.cont(&s, &s.entry, "s-two"), 410, "closed");
}

#[test]
fn chunks_and_seal_address_an_older_segment_by_entry_id() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    assert_eq!(rig.put(&a, &a.entry, &Chunk::first()).status, 200);
    let e1 = rig.cont(&a, &a.entry, "s-two").body["entry_id"]
        .as_str()
        .unwrap()
        .to_string();
    // The new segment's chain starts from zero; the old one carries on where it was.
    assert_eq!(rig.put(&a, &e1, &Chunk::first()).status, 200);
    let more = rig.put(&a, &a.entry, &Chunk::second());
    assert_eq!(more.status, 200, "{}", more.body);
    assert_eq!(more.body["ack"]["entry_id"], a.entry.as_str());
    assert_eq!(rig.get(&a, Some(&a.entry)).body["offset"], 150);
    assert_eq!(rig.get(&a, None).body["offset"], 100);
    assert_eq!(rig.server.position_of(&a.entry).unwrap().offset, 150);
    assert_eq!(rig.server.chunks_of(&e1).len(), 1);
    assert_eq!(rig.server.chunks_of(&a.entry).len(), 2);

    // Another run's entry is nobody's here.
    let theirs = rig.start(&rig.server.mint_credential("member-2"), "theirs");
    is_error(
        &rig.put(&a, &theirs.entry, &Chunk::first()),
        404,
        "not_found",
    );
    is_error(&rig.seal(&a, &theirs.entry), 404, "not_found");

    // Each entry commits its own end and seals alone.
    assert_eq!(rig.beat(&a, &a.entry, 5, &head(5), true).status, 200);
    assert_eq!(rig.server.final_of(&a.entry).map(|f| f.seq), Some(5));
    assert_eq!(rig.server.final_of(&e1), None);
    let sealed = rig.seal(&a, &a.entry);
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    // Before the run's end: the run stays open and the other segment goes on.
    assert_eq!(
        sealed.body,
        json!({"status": "sealed", "run_status": "open"})
    );
    assert_eq!(rig.get(&a, None).body["status"], "open");
    assert!(rig.server.seal_of(&a.entry).is_some() && rig.server.seal_of(&e1).is_none());
    let noend = rig.seal(&a, &e1);
    is_error(&noend, 409, "conflict");
    assert!(
        noend.body["message"]
            .as_str()
            .unwrap()
            .contains("final heartbeat")
    );
    is_error(
        &rig.put(&a, &a.entry, &Chunk::new(150, 50, 6, 7, &head(5))),
        410,
        "closed",
    );
    assert_eq!(rig.put(&a, &e1, &Chunk::second()).status, 200);
    assert_eq!(rig.cont(&a, &e1, "s-three").status, 201);
}

#[test]
fn a_seal_needs_the_committed_end_and_the_chain_up_to_it() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    is_error(&rig.seal(&a, &a.entry), 409, "conflict");
    assert_eq!(rig.beat(&a, &a.entry, 5, &head(5), true).status, 200);
    is_error(&rig.seal(&a, &a.entry), 409, "conflict"); // nothing held
    rig.put(&a, &a.entry, &Chunk::first());
    is_error(&rig.seal(&a, &a.entry), 409, "conflict"); // held up to 3 of 5
    rig.put(&a, &a.entry, &Chunk::second());
    assert_eq!(rig.seal(&a, &a.entry).status, 200);
}

#[test]
fn a_final_that_contradicts_the_held_chain_is_409() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let a = rig.start(&token, "s1");
    rig.put(&a, &a.entry, &Chunk::first());
    is_error(&rig.beat(&a, &a.entry, 2, &head(2), true), 409, "conflict"); // behind what is held
    is_error(&rig.beat(&a, &a.entry, 3, &head(99), true), 409, "conflict"); // other head
    assert_eq!(rig.beat(&a, &a.entry, 3, &head(3), true).status, 200);
}

#[test]
fn at_plus_24_hours_a_run_whose_current_entry_is_sealed_is_sealed_else_incomplete() {
    let rig = Rig::with(ServerOptions {
        close_after_seconds: 2,
        ..ServerOptions::default()
    });
    // Two accounts, so that starting again replaces each account's own run.
    let x = rig.start(&rig.server.mint_credential("member-1"), "x1");
    let y = rig.start(&rig.server.mint_credential("member-2"), "y1");
    for o in [&x, &y] {
        assert_eq!(rig.put(o, &o.entry, &Chunk::first()).status, 200);
        assert_eq!(rig.beat(o, &o.entry, 3, &head(3), true).status, 200);
    }
    assert_eq!(rig.seal(&x, &x.entry).body["run_status"], "open");
    // Both accounts start again, which ends these two runs now.
    rig.start(&rig.server.mint_credential("member-1"), "x2");
    rig.start(&rig.server.mint_credential("member-2"), "y2");
    assert_eq!(rig.get(&x, None).body["status"], "sealed");
    assert_eq!(rig.get(&y, None).body["status"], "open");

    std::thread::sleep(Duration::from_millis(3200));
    assert_eq!(rig.get(&y, None).body["status"], "incomplete");
    assert_eq!(rig.server.status(&y.run).as_deref(), Some("incomplete"));
    assert_eq!(rig.get(&x, None).body["status"], "sealed");
    is_error(&rig.beat(&y, &y.entry, 3, &head(3), false), 410, "closed");
    is_error(&rig.seal(&y, &y.entry), 410, "closed");
    is_error(&rig.beat(&x, &x.entry, 3, &head(3), false), 410, "closed");
    // A seal that was stored is still answered.
    assert_eq!(
        rig.seal(&x, &x.entry).body,
        json!({"status": "sealed", "run_status": "sealed"})
    );
}

#[test]
fn machine_and_connection_facts_are_kept_and_never_shown() {
    let rig = Rig::new();
    let token = rig.server.mint_credential("member-1");
    let machine = json!({"os": "macOS", "cpus": 8, "ac_power": true, "sandbox": null});
    let mut body = Rig::open_body("s1");
    body["machine"] = machine.clone();
    let opened = rig.open_with(
        &token,
        &body,
        &[
            ("CF-Connecting-IP", "203.0.113.9".to_string()),
            ("CF-IPCountry", "DE".to_string()),
        ],
    );
    assert_eq!(opened.status, 201, "{}", opened.body);
    let run = opened.body["run_id"].as_str().unwrap().to_string();
    assert_eq!(rig.server.machine(&run), Some(machine.clone()));
    assert_eq!(
        rig.server.client(&run),
        Some((Some("203.0.113.9".to_string()), Some("DE".to_string())))
    );
    let a = Opened {
        token: token.clone(),
        run: run.clone(),
        entry: opened.body["entry_id"].as_str().unwrap().to_string(),
        deadline_at: opened.body["deadline_at"].as_str().unwrap().to_string(),
    };
    let c = rig.cont_with(
        &a,
        &a.entry,
        "s-two",
        &[("CF-Connecting-IP", "198.51.100.7".to_string())],
    );
    assert_eq!(c.status, 201, "{}", c.body);
    assert_eq!(
        rig.server.client(&run).unwrap().0.as_deref(),
        Some("198.51.100.7")
    );
    assert_eq!(rig.server.machine(&run), Some(machine)); // the open call's facts stay
    for text in [
        opened.body.to_string(),
        c.body.to_string(),
        rig.get(&a, None).body.to_string(),
        rig.beat(&a, c.body["entry_id"].as_str().unwrap(), 0, GENESIS, false)
            .body
            .to_string(),
    ] {
        for secret in ["203.0.113.9", "198.51.100.7", "macOS", "machine", "client_"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }

    // None sent: nothing kept.
    let bare = rig.start(&rig.server.mint_credential("member-2"), "s2");
    assert_eq!(rig.server.machine(&bare.run), None);
    assert_eq!(rig.server.client(&bare.run), Some((None, None)));
}

#[test]
fn machine_must_be_a_flat_object_of_at_most_4_kib() {
    let rig = Rig::new();
    let open = |machine: Value| {
        let mut body = Rig::open_body("s1");
        body["machine"] = machine;
        rig.open_with(&rig.server.mint_credential("member-1"), &body, &[])
    };
    let exactly = |n: usize| json!({"k": "x".repeat(n - r#"{"k":""}"#.len())});
    assert_eq!(exactly(4096).to_string().len(), 4096);
    assert_eq!(open(exactly(4096)).status, 201);
    assert_eq!(open(Value::Null).status, 201); // null is absent
    for bad in [
        exactly(4097),
        json!({"nested": {"a": 1}}),
        json!({"list": [1]}),
        json!([1, 2]),
        json!("linux"),
        json!(5),
    ] {
        is_error(&open(bad), 400, "malformed");
    }
}

#[test]
fn string_caps_count_utf16_code_units_as_the_worker_does() {
    let rig = Rig::new();
    let open = |field: &str, value: String| {
        let mut body = Rig::open_body("s1");
        body[field] = json!(value);
        rig.open_with(&rig.server.mint_credential("member-1"), &body, &[])
    };
    // 32 of these are 64 UTF-16 code units (and 32 characters); 33 are 66 (and 33 characters).
    let emoji = "\u{1F600}";
    assert_eq!(open("harness_version", emoji.repeat(32)).status, 201);
    is_error(&open("harness_version", emoji.repeat(33)), 400, "malformed");
    assert_eq!(open("connector_id", "c".repeat(128)).status, 201);
    is_error(&open("connector_id", "c".repeat(129)), 400, "malformed");
    is_error(
        &open("session_id", "has space".to_string()),
        400,
        "malformed",
    );
    is_error(&open("workspace", "w".repeat(4097)), 400, "malformed");
    let device = |value: String| {
        call(
            "POST",
            &format!("{}/v1/device", rig.server.url()),
            None,
            &[],
            Some(
                &json!({"purpose": "run", "mode": "competition", "connector_id": "x",
                         "tracer_version": value}),
            ),
            b"",
        )
    };
    assert_eq!(device(emoji.repeat(32)).status, 200);
    is_error(&device(emoji.repeat(33)), 400, "malformed");
}

#[test]
fn entries_and_the_replacement_survive_a_restart() {
    let rig = Rig::new();
    let [c1, c2] = [(); 2].map(|_| rig.server.mint_credential("member-1"));
    let a = rig.start(&c1, "s1");
    let e1 = rig.cont(&a, &a.entry, "s-two").body["entry_id"]
        .as_str()
        .unwrap()
        .to_string();
    rig.put(&a, &e1, &Chunk::first());
    let b = rig.start(&c2, "s2");
    let replaced = rig.server.replaced_at(&a.run);
    assert!(replaced.is_some());
    let port = rig.server.port();
    let Rig { dir, server } = rig;
    server.stop();

    let server = UploadServer::start(
        dir.path(),
        ServerOptions {
            port: Some(port),
            ..ServerOptions::default()
        },
    );
    assert_eq!(server.entry_ids(&a.run), vec![a.entry.clone(), e1.clone()]);
    assert_eq!(server.replaced_at(&a.run), replaced);
    assert_eq!(server.replaced_at(&b.run), None);
    assert_eq!(server.chunks_of(&e1).len(), 1);
    assert_eq!(server.current_entry(&a.run).as_deref(), Some(e1.as_str()));
}
