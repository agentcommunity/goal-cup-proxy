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
/// out of the harness, the workspace is the project folder, and a start that cannot begin leaves
/// nothing.
#[cfg(unix)]
mod server_runs {
    use crate::common::m2_cli_support::{FAKE_GITHUB_TOKEN, Hooked, PREP_REPO, Rig};
    use crate::common::upload_server::{ServerOptions, UploadServer};
    use goalcup_cli::account::device::RunMode;
    use goalcup_cli::commands::start::StartOutcome;
    use goalcup_cli::upload::uploader::Policy;
    use std::path::{Path, PathBuf};

    const RESERVE: u64 = 4 * 1024 * 1024;

    fn server(rig: &Rig) -> UploadServer {
        UploadServer::start(&rig.root.join("server"), ServerOptions::default())
    }

    /// An "hour" of 200 ms: the finish at the end of a run is quick.
    fn quick_policy() -> Policy {
        Policy::from_test_spec("200,25,100,400,10000,268435456,3000").unwrap()
    }

    /// The credential a run saved, read from its file: goalcup's own type never gives the
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
        let goal = rig.run_folder();
        run_to_end_with(rig, practice, &goal)
    }

    fn run_to_end_with(rig: &Rig, practice: bool, goal: &Path) -> Ended {
        let mode = if practice {
            RunMode::Practice
        } else {
            RunMode::Competition
        };
        run_to_end_in(rig, mode, goal)
    }

    fn run_to_end_in(rig: &Rig, mode: RunMode, goal: &Path) -> Ended {
        rig.tell("stop", "");
        let http = rig.client();
        let client = Hooked::new(&http, rig);
        let mut opts = rig.options(goal, RESERVE);
        opts.upload_policy = quick_policy();
        opts.mode = mode;
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
        // A practice run has no GitHub repository (D69): the server is told the organisers'
        // practice placeholder.
        assert_eq!(
            server.repo_url(&run_id).as_deref(),
            Some(goalcup_cli::commands::start::PRACTICE_REPO_URL)
        );
    }

    /// `--test` asks for a test approval; the run is a competition run in every other way: it
    /// names its repository, its deadline is the server's, and `run.json` says it is a test run.
    #[test]
    fn a_test_start_asks_for_test_mode() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let goal = rig.run_folder();
        let ended = run_to_end_in(&rig, RunMode::Test, &goal);
        let run_id = server.run_ids()[0].clone();
        let requests = server.device_requests();
        assert_eq!(requests.len(), 1, "{}", ended.out);
        assert_eq!(
            (requests[0].purpose.as_str(), requests[0].mode.as_str()),
            ("run", "test"),
            "{}",
            ended.out
        );
        assert_eq!(server.mode(&run_id).as_deref(), Some("test"));
        let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
        assert!(record.test && !record.practice, "{record:?}");
        assert_eq!(record.mode(), RunMode::Test);
        let kept = goalcup_cli::upload::read_record(&rig.cfg, &ended.outcome.entry_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            record.deadline_at.as_deref(),
            Some(kept.deadline_at.as_str())
        );
        assert_eq!(
            server.repo_url(&run_id).as_deref(),
            Some(PREP_REPO),
            "{}",
            ended.out
        );
    }

    /// Nothing of the credential, the GitHub token in the project folder's `.env.local` or the
    /// device code reaches the harness: not its environment, not its arguments; not the trace
    /// that is uploaded; not what `start` printed.
    #[test]
    fn the_credential_never_reaches_the_harness() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        std::fs::write(
            rig.run_folder().join(".env.local"),
            format!("GH_TOKEN={FAKE_GITHUB_TOKEN}\n"),
        )
        .unwrap();
        std::fs::write(rig.run_folder().join(".gitignore"), ".env*\n").unwrap();
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
    }

    /// A run on the server, real or practice, works in the folder `goalcup start` was run from,
    /// with the participant's starting kit in it (`.gitignore`, `.env*` files): the harness
    /// opens there, and the run's own folder, with the harness's `HOME`, is made in
    /// `<cfg>-runs`, outside the project. Goal Cup runs no git there and leaves the kit as it
    /// was.
    #[test]
    fn a_run_works_in_the_project_folder_with_its_kit() {
        for practice in [false, true] {
            let rig = Rig::new();
            let server = server(&rig);
            rig.prepare_for(&server.url());
            let project = rig.run_folder();
            let kit = [
                (".gitignore", ".env*\n"),
                (".env", "A=1\n"),
                (".env.local", "GH_TOKEN=x\n"),
            ];
            for (name, text) in kit {
                std::fs::write(project.join(name), text).unwrap();
            }
            rig.tell("steps.sh", "pwd -P > \"$out/cwd\"\n");
            let ended = run_to_end(&rig, practice);
            assert_eq!(server.run_ids().len(), 1, "{practice}: {}", ended.out);
            assert_eq!(ended.workspace, project, "{practice}");
            assert_eq!(
                rig.harness_file("cwd").unwrap().trim(),
                project.display().to_string(),
                "{practice}: the harness opens in the project folder"
            );
            let env = rig.harness_file("env").unwrap();
            let home = env
                .lines()
                .find_map(|l| l.strip_prefix("HOME="))
                .expect("the harness has a HOME");
            assert!(
                Path::new(home).starts_with(goalcup_cli::commands::start::run_folders(&rig.cfg)),
                "{practice}: {home}"
            );
            assert!(!project.join(".git").exists(), "{practice}");
            for (name, text) in kit {
                assert_eq!(
                    std::fs::read_to_string(project.join(name)).unwrap(),
                    text,
                    "{practice}"
                );
            }
        }
    }

    /// A new run on the server starts in a folder that holds nothing but the starting kit: one
    /// with anything else in it (a README, a `.git`, a folder named like a key file) is refused
    /// before the approval is asked for, and nothing is created.
    #[test]
    fn a_start_in_a_folder_that_is_not_empty_is_refused_before_the_approval() {
        let leftovers: [(&str, bool); 3] = [("README.md", false), (".git", true), (".env.d", true)];
        for (name, is_dir) in leftovers {
            let rig = Rig::new();
            let server = server(&rig);
            rig.prepare_for(&server.url());
            let project = rig.run_folder();
            std::fs::write(project.join(".env.local"), "GH_TOKEN=x\n").unwrap();
            if is_dir {
                std::fs::create_dir(project.join(name)).unwrap();
            } else {
                std::fs::write(project.join(name), "hi\n").unwrap();
            }
            let http = rig.client();
            let client = Hooked::new(&http, &rig);
            let opts = rig.options(&project, RESERVE);
            let (result, out) =
                std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
            let refused = result.expect_err(name);
            assert_eq!(refused.exit_code(), 1, "{name}: {refused}");
            assert!(refused.to_string().contains(name), "{name}: {refused}");
            assert!(server.device_requests().is_empty(), "{name}: {out}");
            assert!(server.calls().is_empty(), "{name}: {:?}", server.calls());
            assert!(client.book().creates.is_empty(), "{name}");
            assert!(rig.run_dirs().is_empty(), "{name}");
            assert!(!rig.harness_started(), "{name}");
        }
    }

    /// Setup no longer writes a work folder, and one an older setup wrote in `prep.json` is
    /// ignored: the run starts in the project folder and makes nothing in the old work folder.
    #[test]
    fn a_prep_file_with_an_old_work_folder_still_starts() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let old = rig.root.join("goalcup-work");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            goalcup_cli::prep_file::prep_path(&rig.cfg),
            format!(
                r#"{{"harness":"{}","work_root":"{}"}}"#,
                crate::common::m2_cli_support::FAKE_ID,
                old.display()
            ),
        )
        .unwrap();
        let ended = run_to_end(&rig, false);
        assert_eq!(server.run_ids().len(), 1, "{}", ended.out);
        assert_eq!(ended.workspace, rig.run_folder());
        assert_eq!(std::fs::read_dir(&old).unwrap().count(), 0);
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
        // The technical line stays, and the last line is the plain word on how it went.
        let last = said.lines().last().unwrap();
        assert!(
            last == goalcup_cli::participant::upload_finished()
                || last == goalcup_cli::participant::upload_not_sealed(),
            "{said}"
        );

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

    /// A server run needs `goalcup setup` first: a `prep.json`. Missing or unreadable, the
    /// refusal is the one sentence, and it comes before the participant is asked to approve
    /// anything and before anything is created.
    #[test]
    fn start_without_setup_refuses_before_creating_anything() {
        type Break = fn(&Rig);
        let cases: [(&str, Break); 2] = [
            ("no prep.json", |rig| {
                std::fs::remove_file(goalcup_cli::prep_file::prep_path(&rig.cfg)).unwrap();
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
            let goal = rig.run_folder();
            let http = rig.client();
            let client = Hooked::new(&http, &rig);
            let opts = rig.options(&goal, RESERVE);
            let (result, out) =
                std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
            let refused = result.expect_err(what);
            assert_eq!(refused.exit_code(), 1, "{what}: {refused}");
            assert!(
                refused.to_string().contains("goalcup setup"),
                "{what}: the refusal names the command: {refused}"
            );
            assert!(
                server.device_requests().is_empty(),
                "{what}: approval was asked"
            );
            assert!(server.calls().is_empty(), "{what}: {:?}", server.calls());
            assert!(client.book().creates.is_empty(), "{what}: {out}");
            assert!(rig.run_dirs().is_empty(), "{what}");
            assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists(), "{what}");
            assert!(!rig.harness_started(), "{what}");
        }
    }

    /// `--practice` is a run on the server: without a login (no `server.json`) it is refused
    /// before anything is created.
    #[test]
    fn practice_without_a_server_is_refused_before_anything_is_created() {
        let rig = Rig::new();
        let goal = rig.run_folder();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let mut opts = rig.options(&goal, RESERVE);
        opts.mode = RunMode::Practice;
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("refused");
        assert_eq!(refused.exit_code(), 1, "{refused}");
        assert!(
            refused
                .to_string()
                .contains("`--practice` needs `goalcup login` and `goalcup setup` first"),
            "{refused}"
        );
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty());
        assert!(!rig.harness_started());
    }

    /// `--test` is a run on the server too: without a login it is refused before anything is
    /// created, rather than becoming a local run that status would still call a test run.
    #[test]
    fn test_without_a_server_is_refused_before_anything_is_created() {
        let rig = Rig::new();
        let goal = rig.run_folder();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let mut opts = rig.options(&goal, RESERVE);
        opts.mode = RunMode::Test;
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("refused");
        assert_eq!(refused.exit_code(), 1, "{refused}");
        assert!(
            refused
                .to_string()
                .contains("`--test` needs `goalcup login` and `goalcup setup` first"),
            "{refused}"
        );
        assert!(client.book().creates.is_empty(), "{out}");
        assert!(rig.run_dirs().is_empty());
        assert!(!rig.harness_started());
    }

    /// A fake GitHub API that answers every request with `body`, and counts the requests.
    fn fake_github(body: &'static str) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = asked.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                    head.push(byte[0]);
                }
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, asked)
    }

    /// A competition or a test run needs a repository no other run has pushed to: with commits
    /// in it, the start is refused before the approval and before anything is created. A
    /// practice run never pushes, and is not asked.
    #[test]
    fn a_competition_or_test_start_refuses_a_repository_with_commits_and_practice_does_not() {
        let (used, asked) =
            fake_github(r#"{"private":false,"size":12,"permissions":{"push":true}}"#);
        for mode in [RunMode::Competition, RunMode::Test] {
            let rig = Rig::new();
            let upload = server(&rig);
            rig.prepare_for(&upload.url());
            rig.with_env(goalcup_cli::commands::start::TEST_GITHUB_API, &used);
            let goal = rig.run_folder();
            let http = rig.client();
            let client = Hooked::new(&http, &rig);
            let mut opts = rig.options(&goal, RESERVE);
            opts.mode = mode;
            let (result, out) =
                std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
            let refused = result.expect_err("a repository with commits was used");
            assert_eq!(refused.exit_code(), 1, "{mode:?}: {refused}");
            let said = refused.to_string();
            assert!(
                said.contains("is not empty (12 KB)") && said.contains("--repo"),
                "{mode:?}: {said}"
            );
            assert!(upload.device_requests().is_empty(), "{mode:?}: approval");
            assert!(client.book().creates.is_empty(), "{mode:?}: {out}");
            assert!(rig.run_dirs().is_empty(), "{mode:?}");
            assert!(!rig.harness_started(), "{mode:?}");
        }
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Practice keeps today's behaviour: GitHub is not asked, and the run goes ahead.
        let rig = Rig::new();
        let upload = server(&rig);
        rig.prepare_for(&upload.url());
        rig.with_env(goalcup_cli::commands::start::TEST_GITHUB_API, &used);
        run_to_end(&rig, true);
        assert_eq!(upload.run_ids().len(), 1);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);

        // An empty repository lets a competition run start.
        let (empty, _) = fake_github(r#"{"private":false,"size":0,"permissions":{"push":true}}"#);
        let rig = Rig::new();
        let upload = server(&rig);
        rig.prepare_for(&upload.url());
        rig.with_env(goalcup_cli::commands::start::TEST_GITHUB_API, &empty);
        run_to_end(&rig, false);
        assert_eq!(upload.run_ids().len(), 1);
    }

    /// The last real run's repository is kept only once a run has opened with it: a start that
    /// is denied at the approval leaves the name the next start suggests as it was, and a run
    /// that opens keeps its own.
    #[test]
    fn the_repository_is_kept_only_once_the_run_opens() {
        let rig = Rig::new();
        let server = server(&rig);
        rig.prepare_for(&server.url());
        let earlier = "https://github.com/goal-cup/earlier";
        let mut file = goalcup_cli::prep_file::read_prep_lenient(&rig.cfg);
        file.repo_url = earlier.to_string();
        goalcup_cli::prep_file::write_prep(&rig.cfg, &file).unwrap();
        server.set_approval(crate::common::upload_server::Approval::Deny);
        let goal = rig.run_folder();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        result.expect_err("a denied approval started a run");
        assert_eq!(server.device_requests().len(), 1, "{out}");
        assert!(server.run_ids().is_empty());
        assert_eq!(
            goalcup_cli::prep_file::read_prep_lenient(&rig.cfg).repo_url,
            earlier,
            "{out}"
        );
        server.set_approval(crate::common::upload_server::Approval::Auto);
        let ended = run_to_end_in(&rig, RunMode::Test, &goal);
        assert_eq!(server.run_ids().len(), 1, "{}", ended.out);
        assert_eq!(
            goalcup_cli::prep_file::read_prep_lenient(&rig.cfg).repo_url,
            PREP_REPO
        );
    }

    /// A start that fails after the run opened, with the server away so the run cannot be given
    /// back then, keeps that run's credential: the next start gives the run back with it.
    #[test]
    fn a_run_that_could_not_be_given_back_is_given_back_by_the_next_start() {
        let rig = Rig::new();
        let server = std::sync::Arc::new(server(&rig));
        rig.prepare_for(&server.url());
        let goal = rig.run_folder();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        {
            let mut book = client.book();
            // The recording proxy cannot write its CA file, and the server goes away as the daemon
            // is asked.
            book.occupy_ca_file = true;
            let away = server.clone();
            book.on_create = Some(Box::new(move || away.set_maintenance(true)));
        }
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        result.expect_err("the start did not fail");
        let runs = server.run_ids();
        assert_eq!(runs.len(), 1, "{out}");
        let first = runs[0].clone();
        assert_eq!(server.status(&first).as_deref(), Some("open"), "{out}");
        assert!(
            goalcup_cli::upload::pending_path(&rig.cfg).exists(),
            "the open is noted"
        );
        goalcup_cli::account::credential::load(&goalcup_cli::run::run_dir(&rig.cfg, &first))
            .expect("the credential is kept while the run may still be open");

        server.set_maintenance(false);
        client.book().occupy_ca_file = false;
        let ended = run_to_end(&rig, false);
        assert_eq!(
            server.status(&first).as_deref(),
            Some("abandoned"),
            "{}",
            ended.out
        );
        assert_eq!(server.run_ids().len(), 2);
        assert!(!goalcup_cli::upload::pending_path(&rig.cfg).exists());
        assert!(
            !goalcup_cli::run::run_dir(&rig.cfg, &first).exists(),
            "and the given-back run's directory is gone"
        );
    }

    /// A failure after the approval (here the server refusing the open, its start window being
    /// shut) ends with the sentence about what was removed, which never includes the project
    /// folder.
    #[test]
    fn a_refused_open_ends_with_the_cleanup_sentence() {
        let rig = Rig::new();
        let server = UploadServer::start(
            &rig.root.join("server"),
            ServerOptions {
                start_window_closes: Some(1),
                ..ServerOptions::default()
            },
        );
        rig.prepare_for(&server.url());
        let goal = rig.run_folder();
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let opts = rig.options(&goal, RESERVE);
        let (result, out) =
            std::thread::scope(|s| rig.start(s, &client, opts, None).join().unwrap());
        let refused = result.expect_err("the open was not refused");
        assert_eq!(refused.exit_code(), 1, "{refused}");
        assert!(refused.to_string().contains("403"), "{refused}");
        assert!(
            out.contains("Nothing was started, and what this start made was removed."),
            "{out}"
        );
        assert!(client.book().creates.is_empty());
        assert!(rig.run_dirs().is_empty());
        assert!(goal.is_dir(), "the project folder stays");
        assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
    }

    /// A kept note that this start cannot act on is dropped with its credential: it names another
    /// server, or the credential has ended (the server would refuse it for ever).
    #[test]
    fn a_note_for_another_server_or_an_ended_credential_is_dropped_with_its_credential() {
        for (note_server, expires_at) in [
            ("http://127.0.0.1:1", "2099-01-01T00:00:00Z"),
            ("http://127.0.0.1:2", "2020-01-01T00:00:00Z"),
        ] {
            let cfg = tempfile::tempdir().unwrap();
            let run_id = "r0123456789abcdef";
            let credential = goalcup_cli::account::credential::RunCredential::new(
                format!("gcr_{}", "ab".repeat(32)),
                expires_at.to_string(),
            )
            .unwrap();
            let dir = goalcup_cli::run::run_dir(cfg.path(), run_id);
            goalcup_cli::account::credential::store(&dir, &credential).unwrap();
            goalcup_cli::config::write_secret_file(
                &goalcup_cli::upload::pending_path(cfg.path()),
                format!(r#"{{"run_id":"{run_id}","server_url":"{note_server}"}}"#).as_bytes(),
            )
            .unwrap();
            // The current server is `note_server` itself for the ended credential, and a
            // different one for the first case; neither is ever asked.
            let current = if note_server.ends_with(":2") {
                note_server
            } else {
                "http://127.0.0.1:3"
            };
            goalcup_cli::upload::give_back_pending(current, cfg.path());
            assert!(
                !goalcup_cli::upload::pending_path(cfg.path()).exists(),
                "{note_server}: the note stays"
            );
            assert!(!dir.exists(), "{note_server}: the credential stays");
        }
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

    /// The open of a run failed to keep its record (after the run was opened on the server): the
    /// note of the open and the credential are deleted only once the server has taken the run
    /// back. With the server away they stay, so the next start can give the run back.
    #[test]
    fn a_failed_open_keeps_its_note_and_credential_until_the_server_takes_the_run_back() {
        let run_id = "r0123456789abcdef";
        let credential = || {
            goalcup_cli::account::credential::RunCredential::new(
                format!("gcr_{}", "ab".repeat(32)),
                "2099-01-01T00:00:00Z".to_string(),
            )
            .unwrap()
        };
        let kept = |cfg: &Path| {
            let dir = goalcup_cli::run::run_dir(cfg, run_id);
            goalcup_cli::account::credential::store(&dir, &credential()).unwrap();
            goalcup_cli::config::write_secret_file(
                &goalcup_cli::upload::pending_path(cfg),
                format!(r#"{{"run_id":"{run_id}","server_url":"http://127.0.0.1:1"}}"#).as_bytes(),
            )
            .unwrap();
            dir
        };

        // The server is away (nothing listens on port 1): nothing is deleted.
        let cfg = tempfile::tempdir().unwrap();
        let dir = kept(cfg.path());
        let away = goalcup_cli::upload::http::ServerClient::with_credential(
            "http://127.0.0.1:1",
            &credential(),
        );
        assert!(!goalcup_cli::upload::undo_open(&away, cfg.path(), run_id));
        assert!(goalcup_cli::upload::pending_path(cfg.path()).exists());
        assert!(goalcup_cli::account::credential::credential_path(&dir).exists());
        // And the next start, with the server back, gives the run back with that credential.
        let waiting = Waiting::start();
        let back = waiting.url();
        goalcup_cli::config::write_secret_file(
            &goalcup_cli::upload::pending_path(cfg.path()),
            format!(r#"{{"run_id":"{run_id}","server_url":"{back}"}}"#).as_bytes(),
        )
        .unwrap();
        goalcup_cli::upload::give_back_pending(&back, cfg.path());
        assert_eq!(waiting.asked(&format!("POST /v1/runs/{run_id}/abandon")), 1);
        assert!(!goalcup_cli::upload::pending_path(cfg.path()).exists());
        assert!(
            !goalcup_cli::account::credential::credential_path(&dir).exists(),
            "the kept credential goes once the run is given back"
        );
        assert!(!dir.exists(), "and so does the run's directory");

        // The server answers (it holds no such run: gone is settled): note and credential go.
        let cfg = tempfile::tempdir().unwrap();
        let dir = kept(cfg.path());
        let near =
            goalcup_cli::upload::http::ServerClient::with_credential(&waiting.url(), &credential());
        assert!(goalcup_cli::upload::undo_open(&near, cfg.path(), run_id));
        assert!(!goalcup_cli::upload::pending_path(cfg.path()).exists());
        assert!(!dir.exists(), "the run's directory and credential are gone");
    }

    /// A Ctrl-C while the participant has not yet approved ends the start with one sentence:
    /// no run directory, no daemon entry, and no run opened on the server. A real
    /// SIGINT, which `goalcup start` catches into a process-global flag, so the case runs in a
    /// process of its own (this test binary, re-run on [`CTRL_C_CHILD`] alone). Under plain
    /// `cargo test` every other start in this binary shares the flag: one closing or clearing
    /// it swallowed the Ctrl-C, and this start then waited out the whole device code, while a
    /// start the SIGINT landed in refused itself.
    #[test]
    fn ctrl_c_while_waiting_for_approval_creates_nothing() {
        let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
            .args([
                "--exact",
                CTRL_C_CHILD,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CTRL_C_HOST, "1")
            .output()
            .expect("the child runs");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "the child failed:\n{printed}");
        assert!(
            printed.contains("1 passed"),
            "the child did not run its one test:\n{printed}"
        );
    }

    // The libtest name inside the folded `it` binary, which prefixes the module path.
    const CTRL_C_CHILD: &str =
        "upload_open::server_runs::ctrl_c_while_waiting_for_approval_child_half";
    /// Set only in the process [`ctrl_c_while_waiting_for_approval_creates_nothing`] starts.
    const CTRL_C_HOST: &str = "GOALCUP_TEST_CTRL_C_HOST";

    /// Run only by [`ctrl_c_while_waiting_for_approval_creates_nothing`], in a process of its own.
    #[test]
    #[ignore = "run by its parent, in a process of its own"]
    fn ctrl_c_while_waiting_for_approval_child_half() {
        if std::env::var_os(CTRL_C_HOST).is_none() {
            println!("${CTRL_C_HOST} is unset: this case is run by its parent, not alone.");
            return;
        }
        let rig = Rig::new();
        let waiting = Waiting::start();
        rig.prepare_for(&waiting.url());
        let goal = rig.run_folder();
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
        assert!(!goalcup_cli::commands::start::run_folders(&rig.cfg).exists());
        assert!(!goalcup_cli::run::runs_dir(&rig.cfg).exists());
        assert!(!rig.harness_started());
        assert_eq!(waiting.asked("POST /v1/runs"), 0, "a run was opened");
        // The code was shown, and the user was told what to do with it.
        assert!(out.contains("BCDF-GHJK"), "{out}");
    }
}
