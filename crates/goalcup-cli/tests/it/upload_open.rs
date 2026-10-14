//! The checks `goalcup start` makes on what it sends to, and takes from, the upload server.

use goalcup_cli::upload::http::ServerError;
use goalcup_cli::upload::is_run_id;

#[test]
fn run_ids_have_one_shape() {
    assert!(is_run_id("r0123456789abcdef"));
    for bad in [
        "r0123",
        "R0123456789abcdef",
        "e0123456789abcdef",
        "r0123456789abcdeg",
    ] {
        assert!(!is_run_id(bad), "{bad}");
    }
}

/// What is asked again and what is not: no answer, a server error, a rate limit, the
/// maintenance switch and a damaged body are; a refusal of the request itself is not.
#[test]
fn only_a_transient_failure_is_retried() {
    let status = |status: u16| ServerError::Status {
        status,
        code: String::new(),
        message: String::new(),
        retry_after: Some(7),
    };
    assert!(ServerError::Transport("refused".into()).is_retryable());
    for retryable in [500, 502, 503, 429, 422] {
        assert!(status(retryable).is_retryable(), "{retryable}");
    }
    for permanent in [400, 401, 403, 404, 409, 410, 413] {
        assert!(!status(permanent).is_retryable(), "{permanent}");
    }
    assert_eq!(status(503).retry_after(), Some(7));
    assert!(status(409).is_conflict());
}

// ------------------------------------------------------------------ a run on the server

/// `goalcup start` for a run on the server, against the stand-in (which approves at once) and a
/// real in-process daemon: the approval comes first, the credential is the run's own and stays
/// out of the harness, the workspace is a clone, and a start that cannot begin leaves nothing.
#[cfg(unix)]
mod server_runs {
    use crate::common::m2_cli_support::{FAKE_GITHUB_TOKEN, Hooked, PREP_REPO, Rig};
    use crate::common::upload_server::{Approval, ServerOptions, UploadServer};
    use goalcup_cli::commands::start::StartOutcome;
    use goalcup_cli::upload::uploader::Policy;
    use std::path::{Path, PathBuf};

    const GOAL: &str = "write hello.txt, then stop";
    const RESERVE: u64 = 4 * 1024 * 1024;

    fn server(rig: &Rig) -> UploadServer {
        UploadServer::start(&rig.root.join("server"), ServerOptions::default())
    }

    /// An "hour" of 200 ms: the finish at the end of a run is quick.
    fn quick_policy() -> Policy {
        Policy::from_test_spec("200,25,100,400,10000,268435456,3000").unwrap()
    }

    /// The credential a run saved, read from its file: the tracer's own type never gives the
    /// value out, and a test needs it to look for it elsewhere.
    fn saved_secret(cfg: &Path, run_id: &str) -> String {
        let path = goalcup_cli::account::credential::credential_path(&goalcup_cli::run::run_dir(
            cfg, run_id,
        ));
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        stored["credential"].as_str().unwrap().to_string()
    }

    struct Ended {
        outcome: StartOutcome,
        out: String,
        workspace: PathBuf,
    }

    /// One whole run: the harness is told to stop before it starts, so the run ends at once.
    fn run_to_end(rig: &Rig, practice: bool) -> Ended {
        rig.tell("stop", "");
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, rig);
        let mut opts = rig.options(&goal, RESERVE);
        opts.upload_policy = quick_policy();
        opts.practice = practice;
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
        let workspace = PathBuf::from(&client.book().creates[0].workspace);
        Ended {
            outcome,
            out,
            workspace,
        }
    }

    /// Every file below `dir`, with its bytes.
    fn files_below(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                found.extend(files_below(&path));
            } else if kind.is_file() {
                found.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
        found
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    /// A run of 64 lowercase hex digits anywhere in `text`: the shape of a credential's body
    /// and of a device code.
    fn has_64_hex(text: &str) -> bool {
        let mut run = 0;
        for b in text.bytes() {
            if b.is_ascii_digit() || (b'a'..=b'f').contains(&b) {
                run += 1;
                if run >= 64 {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }

    /// The approval comes first and the run is opened with the credential it returns; the
    /// credential is kept in the run's own directory, mode 0600, and `run.json` says what the
    /// server stamped.
    #[test]
    fn start_gets_a_run_bound_credential_before_creating_anything() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let ended = run_to_end(&rig, false);

        let runs = server.run_ids();
        assert_eq!(runs.len(), 1, "{}", ended.out);
        let run_id = &runs[0];

        // One approval, for a competition run of this connector, asked before the run opened.
        let requests = server.device_requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert_eq!(
            (requests[0].purpose.as_str(), requests[0].mode.as_str()),
            ("run", "competition")
        );
        assert_eq!(
            requests[0].connector_id,
            crate::common::m2_cli_support::FAKE_ID
        );
        let calls = server.calls();
        let at = |what: &str| calls.iter().position(|c| c == what).unwrap();
        assert!(
            at("POST /v1/device") < at("POST /v1/device/token")
                && at("POST /v1/device/token") < at("POST /v1/runs"),
            "{calls:?}"
        );

        // The credential: in the run's directory, owner-only, and bound to this run (it lasts to
        // the deadline and a day more, not the hour an unbound one does).
        let dir = goalcup_cli::run::run_dir(&rig.cfg, run_id);
        let file = goalcup_cli::account::credential::credential_path(&dir);
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&file), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }
        goalcup_cli::account::credential::load(&dir).expect("the credential loads");
        let secret = saved_secret(&rig.cfg, run_id);
        assert!(secret.starts_with("gcr_"));
        let ends = server.credential_expires_at(&secret).unwrap();
        assert!(
            ends > goalcup_cli::run::now_unix() + 23 * 3600,
            "a bound credential outlives the run by a day"
        );

        // `run.json` matches what the server opened.
        let record = goalcup_cli::run::read(&rig.cfg, run_id).unwrap();
        let kept = goalcup_cli::upload::read_record(&rig.cfg, &ended.outcome.entry_id)
            .unwrap()
            .unwrap();
        assert_eq!(record.run_id, *run_id);
        assert_eq!(record.server_url.as_deref(), Some(server.url().as_str()));
        assert!(!record.practice);
        assert_eq!(
            record.deadline_at.as_deref(),
            Some(kept.deadline_at.as_str())
        );
        assert_eq!(record.entries[0].entry_id, ended.outcome.entry_id);
        assert_eq!(record.workspace, ended.workspace);
        assert_eq!(server.mode(run_id).as_deref(), Some("competition"));

        // The machine's facts were sent with the open; a name or a serial number was not.
        let machine = server.machine(run_id).expect("machine facts were sent");
        assert_eq!(machine["os"], std::env::consts::OS, "{machine}");
        for key in machine.as_object().unwrap().keys() {
            assert!(
                [
                    "os",
                    "os_version",
                    "arch",
                    "cpu_count",
                    "memory_bytes",
                    "disk_total_bytes",
                    "disk_free_bytes",
                    "wsl",
                    "container",
                    "ac_power"
                ]
                .contains(&key.as_str()),
                "unexpected machine fact {key}: {machine}"
            );
        }
    }

    /// `--practice` asks for a practice approval, and the run is a practice run on both sides.
    #[test]
    fn a_practice_start_is_approved_and_recorded_as_practice() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let ended = run_to_end(&rig, true);
        let run_id = server.run_ids()[0].clone();
        assert_eq!(
            server.device_requests()[0].mode,
            "practice",
            "{}",
            ended.out
        );
        assert_eq!(server.mode(&run_id).as_deref(), Some("practice"));
        assert!(goalcup_cli::run::read(&rig.cfg, &run_id).unwrap().practice);
    }

    /// Nothing of the credential, the GitHub token or the device code reaches the harness: not
    /// its environment, not its arguments; not the trace; not what `start` printed.
    #[test]
    fn the_credential_never_reaches_the_harness() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let ended = run_to_end(&rig, false);
        let run_id = server.run_ids()[0].clone();
        let secret = saved_secret(&rig.cfg, &run_id);

        let env = rig
            .harness_file("env")
            .expect("the harness dumped its environment");
        let argv = rig.harness_file("argv").expect("and its arguments");
        assert!(
            env.contains("HTTPS_PROXY"),
            "the dump is the harness's own: {env}"
        );
        for (what, text) in [("environment", &env), ("arguments", &argv)] {
            assert!(
                !text.contains("gcr_"),
                "the {what} holds a credential: {text}"
            );
            assert!(!text.contains(&secret), "the {what} holds the credential");
            assert!(
                !text.contains(FAKE_GITHUB_TOKEN),
                "the {what} holds the token"
            );
            assert!(
                !has_64_hex(text),
                "the {what} holds a 64-digit hex value: {text}"
            );
        }

        // The trace, and what the participant was shown.
        let trace = rig.trace(&ended.outcome.entry_id);
        let traced = files_below(&trace);
        assert!(!traced.is_empty());
        for (path, bytes) in traced {
            assert!(
                !contains(&bytes, &secret) && !contains(&bytes, FAKE_GITHUB_TOKEN),
                "{} holds a secret",
                path.display()
            );
        }
        assert!(
            !ended.out.contains("gcr_") && !ended.out.contains(FAKE_GITHUB_TOKEN),
            "{}",
            ended.out
        );
        assert!(!ended.out.contains(&secret), "{}", ended.out);

        // Nor is it in the clone the harness works in.
        for (path, bytes) in files_below(&ended.workspace.join(".git")) {
            assert!(
                !contains(&bytes, &secret) && !contains(&bytes, FAKE_GITHUB_TOKEN),
                "{} holds a secret",
                path.display()
            );
        }
    }

    /// The workspace is a clone of the participant's repository in their work folder, and its
    /// remote and configuration hold no token: git asks `goalcup git-credential` for one.
    #[test]
    fn the_workspace_is_a_clone_whose_remote_holds_no_token() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let ended = run_to_end(&rig, false);

        assert_eq!(ended.workspace.parent(), Some(rig.work_root().as_path()));
        assert!(
            ended
                .workspace
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("goalcup-"),
            "{}",
            ended.workspace.display()
        );
        assert!(ended.workspace.join(".git").is_dir(), "a clone has a .git");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&ended.workspace)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(
            git(&["config", "--get", "remote.origin.url"]).trim(),
            PREP_REPO
        );
        let config = std::fs::read_to_string(ended.workspace.join(".git/config")).unwrap();
        assert!(!config.contains(FAKE_GITHUB_TOKEN), "{config}");
        assert!(!config.contains("gcr_"), "{config}");
        assert!(config.contains("git-credential"), "{config}");
        assert!(config.contains("useHttpPath = true"), "{config}");
        // The run record points at the clone, and the server was told the same workspace.
        let record = goalcup_cli::run::read(&rig.cfg, &server.run_ids()[0]).unwrap();
        assert_eq!(record.workspace, ended.workspace);
        // Nothing else is in the folder beside the goal file but the run's own directory.
        assert_eq!(rig.clones(), std::slice::from_ref(&ended.workspace));
    }

    /// `goalcup upload <entry>` finds the run's credential from the entry id alone: no login, no
    /// server file. A run whose credential is not here was not started here.
    #[test]
    fn upload_finds_the_credential_from_the_entry() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let ended = run_to_end(&rig, false);
        let entry = &ended.outcome.entry_id;
        let run_id = server.run_ids()[0].clone();
        assert_ne!(
            *entry, run_id,
            "the entry id and the run id are different names"
        );

        std::fs::remove_file(goalcup_cli::server::server_path(&rig.cfg)).unwrap();
        let mut said = Vec::new();
        goalcup_cli::commands::upload::upload_with(&rig.cfg, entry, quick_policy(), &mut said)
            .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&said)));
        let said = String::from_utf8(said).unwrap();
        assert!(said.contains("uploaded:"), "{said}");

        // A credential that others can read is refused, naming the fix.
        let file = goalcup_cli::account::credential::credential_path(&goalcup_cli::run::run_dir(
            &rig.cfg, &run_id,
        ));
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            let refused = goalcup_cli::commands::upload::upload_with(
                &rig.cfg,
                entry,
                quick_policy(),
                &mut Vec::new(),
            )
            .unwrap_err();
            assert!(refused.to_string().contains("chmod 600"), "{refused}");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        // No credential: not this machine's run.
        std::fs::remove_file(&file).unwrap();
        let refused = goalcup_cli::commands::upload::upload_with(
            &rig.cfg,
            entry,
            quick_policy(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(refused.exit_code(), 1, "{refused}");
        assert!(
            refused
                .to_string()
                .contains("this run was not started on this machine"),
            "{refused}"
        );
        // And an entry nobody started here says the same.
        let refused = goalcup_cli::commands::upload::upload_with(
            &rig.cfg,
            "e0000000000000000",
            quick_policy(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(refused.exit_code(), 1, "{refused}");
    }

    /// A server run needs `goalcup prep` first: its repository and work folder in `prep.json`
    /// and the GitHub token in `github.json`. Whatever is missing, the refusal is the one
    /// sentence, and it comes before the participant is asked to approve anything and before
    /// anything is created.
    #[test]
    fn start_without_prep_refuses_before_creating_anything() {
        type Break = fn(&Rig);
        let cases: [(&str, Break); 5] = [
            ("no prep.json", |rig| {
                std::fs::remove_file(goalcup_cli::prep_file::prep_path(&rig.cfg)).unwrap();
            }),
            ("no github.json", |rig| {
                std::fs::remove_file(goalcup_cli::account::github::github_path(&rig.cfg)).unwrap();
            }),
            ("no work_root", |rig| {
                std::fs::write(
                    goalcup_cli::prep_file::prep_path(&rig.cfg),
                    format!(r#"{{"harness":"x","repo_url":"{PREP_REPO}"}}"#),
                )
                .unwrap();
            }),
            ("no repo_url", |rig| {
                std::fs::write(
                    goalcup_cli::prep_file::prep_path(&rig.cfg),
                    format!(
                        r#"{{"harness":"x","work_root":"{}"}}"#,
                        rig.work_root().display()
                    ),
                )
                .unwrap();
            }),
            ("a prep.json that is not json", |rig| {
                std::fs::write(goalcup_cli::prep_file::prep_path(&rig.cfg), "{ nope").unwrap();
            }),
        ];
        for (what, breakage) in cases {
            let rig = Rig::new();
            let server = server(&rig);
            rig.prepare_for(&server.url());
            breakage(&rig);
            let goal = rig.goal(GOAL);
            let http = rig.client();
            let client = Hooked::new(&http, &rig);
            let opts = rig.options(&goal, RESERVE);
            let (result, out) =
                std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
            let refused = result.expect_err(what);
            assert_eq!(refused.exit_code(), 1, "{what}: {refused}");
            assert_eq!(refused.to_string(), "run `goalcup prep` first", "{what}");
            assert!(
                server.device_requests().is_empty(),
                "{what}: approval was asked"
            );
            assert!(server.calls().is_empty(), "{what}: {:?}", server.calls());
            assert!(client.book().creates.is_empty(), "{what}: {out}");
            assert!(rig.run_dirs().is_empty(), "{what}");
            assert!(rig.clones().is_empty(), "{what}");
            assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists(), "{what}");
            assert!(!rig.harness_started(), "{what}");
        }
    }

    /// A clone that fails ends the start with a sentence and leaves nothing: no run directory,
    /// no half clone, and no run opened on the server (the clone comes before the open).
    #[test]
    fn a_clone_that_fails_leaves_nothing_and_opens_no_run() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        rig.with_env(
            goalcup_cli::commands::start::TEST_CLONE_SOURCE,
            &rig.root
                .join("no-such-repository.git")
                .display()
                .to_string(),
        );
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("the clone did not fail");
        assert_eq!(refused.exit_code(), 1, "{refused}");
        let said = refused.to_string();
        assert!(said.contains("could not clone"), "{said}");
        assert!(said.contains("removed"), "{said}");
        assert!(!said.contains(FAKE_GITHUB_TOKEN), "{said}");
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty());
        assert!(rig.clones().is_empty(), "{:?}", rig.clones());
        assert!(server.run_ids().is_empty(), "no run was opened");
        assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
    }

    // -------------------------------------------------------------- the approval, abandoned

    /// A server that hands out a device code and never approves it: what the participant's
    /// browser not answering looks like. It counts what it was asked.
    struct Waiting {
        port: u16,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Waiting {
        fn start() -> Waiting {
            use std::io::{BufRead, BufReader, Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (seen2, stop2) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop2.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() {
                        continue;
                    }
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; length];
                    let _ = reader.read_exact(&mut body);
                    let label = request_line
                        .split_whitespace()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" ");
                    seen2.lock().unwrap().push(label.clone());
                    let (status, reply) = match label.as_str() {
                        "POST /v1/device" => (
                            "200 OK",
                            format!(
                                r#"{{"device_code":"{}","user_code":"BCDF-GHJK","verification_uri":"http://127.0.0.1:{port}/device","verification_uri_complete":"http://127.0.0.1:{port}/device?code=BCDF-GHJK","interval":1,"expires_in":900}}"#,
                                "ab".repeat(32)
                            ),
                        ),
                        "POST /v1/device/token" => {
                            ("202 Accepted", r#"{"status":"pending"}"#.to_string())
                        }
                        _ => ("404 Not Found", "{}".to_string()),
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                }
            });
            Waiting { port, seen, stop }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn asked(&self, label: &str) -> usize {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|l| *l == label)
                .count()
        }
    }

    impl Drop for Waiting {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    /// A Ctrl-C while the participant has not yet approved ends the start with one sentence:
    /// no run directory, no clone, no daemon entry, and no run opened on the server. A real
    /// SIGINT, which `goalcup start` catches into a flag; each test is its own process.
    #[test]
    fn ctrl_c_while_waiting_for_approval_creates_nothing() {
        let rig = Rig::new();
        let waiting = Waiting::start();
        rig.prepare_for(&waiting.url());
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) = std::thread::scope(|s| {
            let running = rig.start(s, &client, opts, None);
            // The handler is installed before the first poll, so a poll means it is safe.
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while waiting.asked("POST /v1/device/token") == 0 {
                assert!(std::time::Instant::now() < until, "the start never polled");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // SAFETY: signals this process, whose SIGINT handler `goalcup start` installed.
            unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
            running.join().unwrap()
        });
        let refused = result.expect_err("the start went on after a Ctrl-C");
        assert_eq!(refused.exit_code(), 1, "{refused}");
        let said = refused.to_string();
        assert!(said.contains("interrupted"), "{said}");
        assert!(said.contains("Nothing was created"), "{said}");
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty(), "{:?}", rig.run_dirs());
        assert!(rig.clones().is_empty(), "{:?}", rig.clones());
        assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
        assert!(!rig.harness_started());
        assert_eq!(waiting.asked("POST /v1/runs"), 0, "a run was opened");
        // The code was shown, and the user was told what to do with it.
        assert!(out.contains("BCDF-GHJK"), "{out}");
    }

    /// A denial and an expiry end the same way (see `upload_e2e` for the full-tier twin): one
    /// sentence, and no run on the server.
    #[test]
    fn a_denied_approval_opens_no_run() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        server.set_approval(Approval::Deny);
        let goal = rig.goal(GOAL);
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("denied");
        assert!(refused.to_string().contains("denied"), "{refused}");
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty());
        assert!(rig.clones().is_empty());
        assert!(server.run_ids().is_empty());
    }
}
