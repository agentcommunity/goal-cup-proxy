//! The stand-in's device flow and run credential against `UPLOAD-API.md` ("Sign in and the run
//! credential"), over real loopback HTTP. The Worker's own tests hold the same rules, so the
//! tracer's tests can trust this stand-in for them.

use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::common::upload_server::{Approval, ServerOptions, UploadServer};

const LEGACY_TOKEN: &str = "test-token";
/// The member the stand-in approves device requests as, unless told otherwise.
const TEST_MEMBER: &str = "member-1";

struct Reply {
    status: u16,
    body: Value,
}

fn call(method: &str, url: &str, token: Option<&str>, body: Option<&Value>) -> Reply {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut req = agent.request(method, url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let result = match body {
        Some(b) => req
            .set("Content-Type", "application/json")
            .send_string(&b.to_string()),
        None if method == "GET" => req.call(),
        None => req.send_bytes(b""),
    };
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("transport: {e}"),
    };
    let status = resp.status();
    let text = resp.into_string().expect("read the reply body");
    let body = serde_json::from_str(&text).unwrap_or_else(|_| panic!("reply is not JSON: {text}"));
    Reply { status, body }
}

struct Rig {
    dir: TempDir,
    server: UploadServer,
}

impl Rig {
    fn new() -> Rig {
        let dir = TempDir::new().expect("tempdir");
        let server = UploadServer::start(dir.path(), ServerOptions::default());
        Rig { dir, server }
    }

    fn send(&self, method: &str, path: &str, token: Option<&str>, body: Option<&Value>) -> Reply {
        call(method, &format!("{}{path}", self.server.url()), token, body)
    }

    fn device(&self, purpose: &str) -> Value {
        let r = self.send(
            "POST",
            "/v1/device",
            None,
            Some(&json!({"purpose": purpose, "mode": "practice",
                         "connector_id": "codex-cli", "tracer_version": "0.1.0"})),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        r.body
    }

    fn poll(&self, device_code: &str) -> Reply {
        self.send(
            "POST",
            "/v1/device/token",
            None,
            Some(&json!({"device_code": device_code})),
        )
    }

    /// The credential of a fresh device request.
    fn credential(&self) -> String {
        let device = self.device("run");
        let r = self.poll(device["device_code"].as_str().unwrap());
        assert_eq!(r.status, 200, "{}", r.body);
        r.body["credential"].as_str().unwrap().to_string()
    }

    fn open(&self, token: &str, session: &str) -> Reply {
        self.send(
            "POST",
            "/v1/runs",
            Some(token),
            Some(&json!({
                "connector_id": "codex-cli",
                "manifest_sha256": hex::encode(Sha256::digest(b"manifest")),
                "connector_dir_sha256": hex::encode(Sha256::digest(b"dir")),
                "workspace": "/tmp/run-1/workspace",
                "session_id": session,
                "harness_version": "0.153.0",
                "tracer_version": "0.1.0",
                "repo_url": "https://github.com/someone/their-repo",
            })),
        )
    }

    fn get(&self, run: &str, token: &str) -> Reply {
        self.send("GET", &format!("/v1/runs/{run}"), Some(token), None)
    }
}

fn is_error(r: &Reply, status: u16, code: &str) {
    assert_eq!(r.status, status, "{}", r.body);
    assert_eq!(r.body["error"], code, "{}", r.body);
    assert!(r.body["message"].is_string(), "{}", r.body);
}

fn is_credential(text: &str) -> bool {
    text.len() == 68
        && text.starts_with("gcr_")
        && text[4..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[test]
fn a_device_request_is_approved_at_once_and_picked_up_once() {
    let rig = Rig::new();
    let device = rig.device("run");
    let code = device["device_code"].as_str().unwrap();
    let user = device["user_code"].as_str().unwrap();
    assert_eq!(code.len(), 64);
    assert!(code.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(user.len(), 9);
    assert_eq!(&user[4..5], "-");
    assert!(
        user.replace('-', "")
            .bytes()
            .all(|b| b"BCDFGHJKLMNPQRSTVWXZ".contains(&b))
    );
    let uri = device["verification_uri"].as_str().unwrap();
    assert_eq!(
        device["verification_uri_complete"],
        format!("{uri}?code={user}")
    );
    assert_eq!(device["interval"], 5);
    assert_eq!(device["expires_in"], 900);

    let first = rig.poll(code);
    assert_eq!(first.status, 200, "{}", first.body);
    assert_eq!(first.body["status"], "approved");
    assert_eq!(first.body["purpose"], "run");
    assert!(is_credential(first.body["credential"].as_str().unwrap()));
    assert!(first.body["expires_at"].is_string());
    is_error(&rig.poll(code), 410, "closed");
    is_error(&rig.poll(code), 410, "closed");

    // What the request said is kept for a test to look at, and the device code never travels in a URL.
    let requests = rig.server.device_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].purpose.as_str(), requests[0].mode.as_str()),
        ("run", "practice")
    );
    assert_eq!(requests[0].connector_id, "codex-cli");
    assert_eq!(requests[0].tracer_version, "0.1.0");
    assert!(rig.server.calls().iter().all(|c| !c.contains(code)));
    assert!(
        rig.server
            .calls()
            .contains(&"POST /v1/device/token".to_string())
    );
}

#[test]
fn a_login_request_returns_the_handle_once_and_no_credential() {
    let rig = Rig::new();
    let device = rig.device("login");
    let code = device["device_code"].as_str().unwrap();
    let r = rig.poll(code);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.body,
        json!({"status": "approved", "purpose": "login", "account": {"handle": TEST_MEMBER}})
    );
    is_error(&rig.poll(code), 410, "closed");
}

#[test]
fn the_approval_knob_denies_or_expires_a_request() {
    let rig = Rig::new();
    rig.server.set_approval(Approval::Deny);
    let denied = rig.device("run");
    let code = denied["device_code"].as_str().unwrap();
    is_error(&rig.poll(code), 403, "forbidden");
    is_error(&rig.poll(code), 403, "forbidden");

    rig.server.set_approval(Approval::Expire);
    let expired = rig.device("run");
    is_error(
        &rig.poll(expired["device_code"].as_str().unwrap()),
        410,
        "closed",
    );

    rig.server.set_approval(Approval::Auto);
    assert_eq!(
        rig.poll(rig.device("run")["device_code"].as_str().unwrap())
            .status,
        200
    );
    is_error(&rig.poll(&"a".repeat(64)), 410, "closed"); // never issued
}

#[test]
fn bad_device_requests_are_400() {
    let rig = Rig::new();
    let good =
        json!({"purpose": "run", "mode": "practice", "connector_id": "c", "tracer_version": "t"});
    for (field, value) in [
        ("purpose", json!("root")),
        ("mode", json!("dry_run")),
        ("connector_id", json!("")),
        ("tracer_version", json!(5)),
    ] {
        let mut body = good.clone();
        body[field] = value;
        is_error(
            &rig.send("POST", "/v1/device", None, Some(&body)),
            400,
            "malformed",
        );
    }
    is_error(
        &rig.send(
            "POST",
            "/v1/device/token",
            None,
            Some(&json!({"device_code": "nope"})),
        ),
        400,
        "malformed",
    );
}

#[test]
fn a_credential_opens_one_run_and_sees_only_it() {
    let rig = Rig::new();
    let a = rig.mint_credential_for("member-a");
    let b = rig.mint_credential_for("member-b");
    let unbound = rig.mint_credential_for("member-c");
    let run_a = rig.open(&a, "sa");
    let run_b = rig.open(&b, "sb");
    assert_eq!(run_a.status, 201, "{}", run_a.body);
    let (id_a, id_b) = (
        run_a.body["run_id"].as_str().unwrap(),
        run_b.body["run_id"].as_str().unwrap(),
    );

    assert_eq!(rig.get(id_a, &a).status, 200);
    is_error(&rig.get(id_a, &b), 404, "not_found");
    is_error(&rig.get(id_b, &a), 404, "not_found");
    is_error(&rig.get(id_a, &unbound), 404, "not_found"); // unbound: no run at all
    is_error(&rig.get("r0123456789abcdef", &unbound), 404, "not_found");
    is_error(
        &rig.get(id_a, &format!("gcr_{}", "0".repeat(64))),
        401,
        "unauthorized",
    );
    is_error(&rig.get(id_a, "nope"), 401, "unauthorized");
    is_error(
        &rig.send("GET", &format!("/v1/runs/{id_a}"), None, None),
        401,
        "unauthorized",
    );
}

#[test]
fn the_same_session_id_returns_the_same_run_and_another_is_409() {
    let rig = Rig::new();
    let credential = rig.credential();
    let first = rig.open(&credential, "same");
    assert_eq!(first.status, 201, "{}", first.body);
    let again = rig.open(&credential, "same");
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(again.body, first.body);
    let other = rig.open(&credential, "other");
    is_error(&other, 409, "conflict");
    assert!(
        other.body["message"]
            .as_str()
            .unwrap()
            .contains("goalcup continue"),
        "{}",
        other.body
    );

    // A second credential of the same member meets the account's one open run.
    let second = rig.credential();
    is_error(&rig.open(&second, "another"), 409, "conflict");
}

#[test]
fn an_expired_credential_is_401() {
    let rig = Rig::new();
    let credential = rig.credential();
    let run = rig.open(&credential, "s");
    let id = run.body["run_id"].as_str().unwrap();
    assert_eq!(rig.get(id, &credential).status, 200);
    rig.server.expire_credential(&credential);
    is_error(&rig.get(id, &credential), 401, "unauthorized");
}

#[test]
fn the_legacy_member_token_still_sees_the_members_runs() {
    let rig = Rig::new();
    let run = rig.open(LEGACY_TOKEN, "legacy");
    assert_eq!(run.status, 201, "{}", run.body);
    let id = run.body["run_id"].as_str().unwrap();
    assert_eq!(rig.get(id, LEGACY_TOKEN).status, 200);
    // The member's credential sees the run only if it opened it.
    let credential = rig.server.mint_credential(TEST_MEMBER);
    is_error(&rig.get(id, &credential), 404, "not_found");
}

#[test]
fn credentials_and_their_runs_survive_a_restart() {
    let rig = Rig::new();
    let credential = rig.credential();
    let id = rig.open(&credential, "s").body["run_id"]
        .as_str()
        .unwrap()
        .to_string();
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
    let back = call(
        "GET",
        &format!("{}/v1/runs/{id}", server.url()),
        Some(&credential),
        None,
    );
    assert_eq!(back.status, 200, "{}", back.body);
}

#[test]
fn the_heartbeat_takes_recording_and_model_requests_and_answers_reason_null() {
    let rig = Rig::new();
    let credential = rig.credential();
    let run = rig.open(&credential, "s").body;
    let (id, entry) = (
        run["run_id"].as_str().unwrap(),
        run["entry_id"].as_str().unwrap(),
    );
    let beat = |extra: Value| {
        let mut body = json!({"entry_id": entry, "seq": 0, "head": "0".repeat(64),
                              "offset_local": 0, "final": false, "repo_head": null});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        rig.send(
            "POST",
            &format!("/v1/runs/{id}/heartbeat"),
            Some(&credential),
            Some(&body),
        )
    };
    let plain = beat(json!({}));
    assert_eq!(plain.status, 200, "{}", plain.body);
    assert_eq!(plain.body["reason"], Value::Null);
    assert_eq!(plain.body["stop"], false);
    for recording in ["ok", "gaps", "failed", "unknown"] {
        let r = beat(json!({"recording": recording, "model_requests": 12}));
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["reason"], Value::Null);
    }
    is_error(&beat(json!({"recording": "fine"})), 400, "malformed");
    is_error(&beat(json!({"model_requests": -1})), 400, "malformed");

    let seen = rig.server.heartbeats(id);
    assert_eq!(seen.len(), 5);
    assert_eq!(
        (seen[0].recording.as_deref(), seen[0].model_requests),
        (None, None)
    );
    assert_eq!(
        (seen[1].recording.as_deref(), seen[1].model_requests),
        (Some("ok"), Some(12))
    );
}

impl Rig {
    /// A credential minted without HTTP, for a named member.
    fn mint_credential_for(&self, member: &str) -> String {
        self.server.mint_credential(member)
    }
}
