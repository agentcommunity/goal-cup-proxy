//! PRD-06 "Automated tests", `upload_worker.rs` (full tier: every test is `#[ignore]`; a build box
//! runs them after `npm ci` in `server/worker` with
//! `cargo nextest run -p goalcup --run-ignored only -E 'binary(upload_worker)'`). The real client
//! against the real Worker under `wrangler dev` with its local database, read back through
//! `scripts/fetch-run.sh --local`. Loopback only; nothing from staging or production.

mod common;
mod upload_common;

use goalcup::cli;
use goalcup::upload::client::{ServerClient, ServerError, Token};
use goalcup::upload::manual::{Outcome, run};
use goalcup::upload::state::{self, Final};
use goalcup::upload::{UploadCtx, spawn};
use serde_json::{Value, json};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, io::Write, thread};

/// Switches on the test approval route of this local Worker only. Made up here; not a secret.
const APPROVAL_KEY: &str = "upload-worker-test-approval";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `wrangler dev` in local mode (as `server/worker/test/helpers/worker.ts` starts it), with its own
/// state folder. Its whole process group is stopped on drop, on every exit path.
struct Wrangler {
    child: Child,
    url: String,
    dir: tempfile::TempDir,
}

impl Drop for Wrangler {
    fn drop(&mut self) {
        let group = -(self.child.id() as i32);
        unsafe { libc::kill(group, libc::SIGTERM) };
        let until = Instant::now() + Duration::from_secs(5);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

impl Wrangler {
    /// Ready when the Worker answers its own JSON 404. The ports are let go just before wrangler
    /// binds them, so a start that lost them to someone else is tried again with new ones.
    fn start() -> Wrangler {
        for _ in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let p = |name: &str| dir.path().join(name);
            let ports = [
                common::ports::held().1.port(),
                common::ports::held().1.port(),
            ];
            // The key goes by file, never on wrangler's command line.
            fs::write(p("vars.env"), format!("TEST_APPROVAL_KEY={APPROVAL_KEY}\n")).unwrap();
            let log = fs::File::create(p("wrangler.log")).unwrap();
            let mut cmd = Command::new("node");
            cmd.current_dir(repo().join("server/worker"))
                .args([
                    "node_modules/wrangler/bin/wrangler.js",
                    "dev",
                    "--ip",
                    "127.0.0.1",
                ])
                .args([
                    "--port",
                    &ports[0].to_string(),
                    "--inspector-port",
                    &ports[1].to_string(),
                ])
                .args(["--show-interactive-dev-session=false", "--persist-to"])
                .args([p("state"), "--env-file".into(), p("vars.env")]);
            for var in ["DB_BACKEND:local", "GITHUB_CHECK:off", "EVENT_ID:practice"] {
                cmd.args(["--var", var]);
            }
            for (k, d) in [("XDG_CONFIG_HOME", "config"), ("WRANGLER_LOG_PATH", "logs")] {
                cmd.env(k, p(d));
            }
            cmd.env("WRANGLER_REGISTRY_PATH", p("registry"))
                .envs([
                    ("WRANGLER_SEND_METRICS", "false"),
                    ("CI", "1"),
                    ("NO_COLOR", "1"),
                ])
                .env_remove("CLOUDFLARE_API_TOKEN")
                .env_remove("CLOUDFLARE_ACCOUNT_ID")
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .process_group(0);
            let url = format!("http://127.0.0.1:{}", ports[0]);
            let mut w = Wrangler {
                child: cmd.spawn().expect("node"),
                url,
                dir,
            };
            let probe = ServerClient::new(&w.url, None);
            let until = Instant::now() + Duration::from_secs(90);
            while matches!(w.child.try_wait(), Ok(None)) {
                let ready = probe.json::<Value, ()>("GET", "/v1/ready-check", None);
                if matches!(ready, Err(ServerError::Status { status: 404, ref code, .. }) if code == "not_found")
                {
                    return w;
                }
                assert!(
                    Instant::now() < until,
                    "wrangler dev not ready:\n{}",
                    w.log()
                );
                thread::sleep(Duration::from_millis(250));
            }
            let log = w.log();
            let collision = log.contains("Address already in use") || log.contains("EADDRINUSE");
            assert!(collision, "wrangler dev exited early:\n{log}");
        }
        panic!("wrangler dev lost its ports five times");
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("wrangler.log")).unwrap_or_default()
    }

    /// `fetch-run.sh --local` of `run` into a new folder: the `fetch.json` of `entry`, and its folder.
    fn fetch(&self, run: &str, entry: &str, n: u32) -> (Value, PathBuf) {
        let out = self.dir.path().join(format!("fetch-{n}"));
        let done = Command::new("sh")
            .arg(repo().join("scripts/fetch-run.sh"))
            .arg("--local")
            .args([self.dir.path().join("state"), run.into(), out.clone()])
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "{}",
            String::from_utf8_lossy(&done.stderr)
        );
        let dir = out.join(entry);
        (
            serde_json::from_slice(&fs::read(dir.join("fetch.json")).unwrap()).unwrap(),
            dir,
        )
    }
}

/// A practice run opened through the real device flow and the test approval route, written under
/// `home` as `goalcup` writes it: the credential (0600, by file only) and `server.json`.
fn open_run(url: &str, home: &Path) -> (PathBuf, String, String) {
    let version = env!("CARGO_PKG_VERSION");
    let anon = ServerClient::new(url, None);
    let ask = json!({"purpose": "run", "mode": "practice", "connector_id": "c", "proxy_version": version});
    let made: Value = anon.json("POST", "/v1/device", Some(&ask)).unwrap();
    let yes =
        json!({"user_code": made["user_code"], "member_id": "upload-worker", "approve": true});
    let approver = ServerClient::new(url, Some(Token::new(APPROVAL_KEY)));
    approver
        .json::<Value, _>("POST", "/v1/device/approve", Some(&yes))
        .unwrap();
    let poll = json!({"device_code": made["device_code"]});
    let got: Value = anon.json("POST", "/v1/device/token", Some(&poll)).unwrap();
    let secret = got["credential"]
        .as_str()
        .expect("a run credential")
        .to_owned();
    let body = json!({"connector_id": "c", "connector_sha256": "a".repeat(64), "workspace": "/w",
        "session_id": "6f1c2d3e-0000-4000-8000-0000000000aa", "harness_version": "1",
        "proxy_version": version, "repo_url": "https://github.com/someone/entry"});
    let client = ServerClient::new(url, Some(Token::new(secret.clone())));
    let open: Value = client.json("POST", "/v1/runs", Some(&body)).unwrap();
    let (run_id, entry) = (
        open["run_id"].as_str().unwrap(),
        open["entry_id"].as_str().unwrap(),
    );
    let dir = cli::run_dir(home, run_id);
    fs::create_dir_all(&dir).unwrap();
    let mut cred = fs::OpenOptions::new();
    cred.write(true).create_new(true).mode(0o600);
    cred.open(cli::credential(&dir))
        .unwrap()
        .write_all(secret.as_bytes())
        .unwrap();
    let server = json!({"v": 1, "server": url, "run": run_id, "mode": "practice",
        "deadline": open["deadline_at"], "repo": null, "hosts_file": null,
        "entries": [{"id": entry, "harness_version": "1", "resumed": false}],
        "connector": {"id": "c", "name": "c", "source": "builtin"}});
    fs::write(cli::server_json(&dir), server.to_string()).unwrap();
    (dir, run_id.into(), entry.into())
}

#[test]
#[ignore = "full tier: needs `npm ci` in server/worker; a build box runs it"]
fn a_sealed_entry_of_two_parts_is_held_whole_and_sealed() {
    let w = Wrangler::start();
    let home = tempfile::tempdir().unwrap();
    let (run_dir, run_id, entry) = open_run(&w.url, home.path());
    let entry_dir = run_dir.join(&entry);
    let all = upload_common::entry(&entry_dir, &entry, true);
    let ids = (run_id.clone(), entry.clone());
    let (e, r) = (entry_dir.clone(), run_dir.clone());
    let sent = common::within(Duration::from_secs(120), move || {
        run(&e, &r, (&ids.0, &ids.1), false)
    });
    assert_eq!(sent, Outcome::Done);
    assert_eq!(state::read(&entry_dir).unwrap().final_, Final::Sealed);
    let cred = fs::read_to_string(cli::credential(&run_dir)).unwrap();
    let client = ServerClient::new(&w.url, Some(Token::new(cred.trim())));
    assert!(
        client.position(&run_id, &entry).unwrap().sealed,
        "the position says sealed"
    );
    let (f, got) = w.fetch(&run_id, &entry, 0);
    assert_eq!(
        fs::read(got.join("rec.gcr.zst")).unwrap(),
        all,
        "cat rec-*.gcr.zst, byte for byte"
    );
    assert_eq!(f["complete"], true);
    assert!(f["sealed_at"].is_string(), "fetch.json says sealed");
}

/// Ledger R7, the uploader's half: what the null recorder hands the uploader (`recorder: None`)
/// is stored as `failed` with `counts.mode` null. The other half, `gaps` after the next good boot,
/// needs PRD-02's `watchdog_null.rs` setup with the `test-hooks` binary and is added with it.
#[test]
#[ignore = "full tier: needs `npm ci` in server/worker; a build box runs it"]
fn null_mode_is_stored_as_failed_with_mode_null() {
    let w = Wrangler::start();
    let home = tempfile::tempdir().unwrap();
    let (run_dir, run_id, entry) = open_run(&w.url, home.path());
    let entry_dir = run_dir.join(&entry);
    upload_common::entry(&entry_dir, &entry, false);
    let _uploader = spawn(UploadCtx {
        home: home.path().into(),
        run_dir,
        entry_dir,
        run: run_id.clone(),
        entry: entry.clone(),
        recorder: None,
        keep: true,
    });
    let until = Instant::now() + Duration::from_secs(90);
    let beat = (0..)
        .map(|n| {
            thread::sleep(Duration::from_secs(2));
            w.fetch(&run_id, &entry, n).0["last_heartbeat"].clone()
        })
        .find(|b| b["recording"] == "failed" || Instant::now() > until)
        .unwrap();
    assert_eq!(
        beat["recording"], "failed",
        "a stored heartbeat says failed: {beat}"
    );
    assert_eq!(
        beat["counts"].get("mode"),
        Some(&Value::Null),
        "counts.mode is JSON null"
    );
}
