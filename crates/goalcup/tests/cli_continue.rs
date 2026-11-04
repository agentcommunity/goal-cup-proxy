//! `goalcup continue` (PRD-10 section 3.1): the resume rule from the parts' `status.json` files,
//! a live harness or recorder from the last part, a changed connector or host file, and the call
//! to the fake Worker. Facts only; no wording.

mod cli_common;
mod common;

use std::path::PathBuf;

use cli_common::{Stand, put, with};
use common::home::TempHome;
use common::worker::{CREDENTIAL, FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli::EntryFacts;
use goalcup::cli::approve::Stop;
use goalcup::cli::continue_run::{Args, prepare, resume_from_files};
use goalcup::cli::start::Ready;
use goalcup::cli::text;
use goalcup::pipeline::status::Mode;
use serde_json::{Value, json};

const DEAD_SERVER: &str = "http://127.0.0.1:1";

fn e(n: u32) -> String {
    format!("e{n:016x}")
}

/// One run in a temporary home, parts added by hand.
struct World {
    t: TempHome,
    project: PathBuf,
    connector: PathBuf,
    parts: Vec<Value>,
    server: Option<String>,
}

impl World {
    fn new(server: Option<String>) -> World {
        let t = TempHome::new();
        let project = t.home().join("p");
        std::fs::create_dir_all(&project).unwrap();
        let connector = t.home().join("tester.toml");
        let body = "id = \"tester\"\nname = \"Tester\"\nprogram = \"true\"\nargs = [\"go\"]\n\
            resume = [\"--resume\"]\nskills_dir = \"{home}/skills\"\nfirst_screen = \"Go.\"\n";
        std::fs::write(&connector, body).unwrap();
        let w = World {
            t,
            project,
            connector,
            parts: Vec::new(),
            server,
        };
        std::fs::create_dir_all(w.run_dir()).unwrap();
        std::fs::write(w.run_dir().join("credential"), CREDENTIAL).unwrap();
        w
    }

    fn gc(&self) -> PathBuf {
        self.t.home().join("b/gc")
    }

    fn run_dir(&self) -> PathBuf {
        self.gc().join("runs").join(RUN_ID)
    }

    /// Adds part `n` with `status` as its `status.json` (none writes no file) and saves
    /// `server.json`.
    fn part(&mut self, n: u32, resumed: bool, status: Option<Value>) {
        let dir = self.run_dir().join(e(n));
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(s) = status {
            let st = with(Mode::Recording, s);
            put(&dir, "status.json", &st);
        }
        self.parts
            .push(json!({"id": e(n), "harness_version": null, "resumed": resumed}));
        self.save();
    }

    fn sealed(written: Value) -> Value {
        json!({"mode": "sealed", "written": written})
    }

    fn save(&self) {
        let sj = json!({"v": 1, "server": self.server, "run": RUN_ID, "mode": "practice",
            "deadline": null, "repo": null, "hosts_file": null, "entries": self.parts,
            "connector": {"id": "tester", "name": "Tester",
                "source": self.connector.display().to_string()}});
        put(&self.run_dir(), "server.json", &sj);
    }

    fn server_json(&self) -> Vec<u8> {
        std::fs::read(self.run_dir().join("server.json")).unwrap()
    }

    fn folders(&self) -> usize {
        std::fs::read_dir(self.run_dir())
            .unwrap()
            .filter(|d| d.as_ref().unwrap().path().is_dir())
            .count()
    }

    fn go(&self, connector: Option<&PathBuf>, hosts: Option<&PathBuf>) -> Result<Ready, Stop> {
        let a = Args {
            run: None,
            connector: connector.map(PathBuf::as_path),
            hosts: hosts.map(PathBuf::as_path),
        };
        let vars = [("PATH".to_owned(), std::env::var("PATH").unwrap_or_default())].into();
        prepare(&a, &self.gc(), &self.project, &vars, &mut Vec::new())
    }

    fn prepare(&self) -> Result<Ready, Stop> {
        self.go(Some(&self.connector), None)
    }

    fn resume(&self) -> bool {
        let parts = serde_json::from_slice::<Value>(&self.server_json()).unwrap();
        let entries: Vec<EntryFacts> = serde_json::from_value(parts["entries"].clone()).unwrap();
        resume_from_files(&self.run_dir(), &entries)
    }
}

/// A resumed part whose harness ran `secs` and exited with `exit`.
fn ran(secs: u64, exit: Value) -> Value {
    let (start, end) = (cli_common::T0 - secs, cli_common::T0);
    let stamp = |t: u64| {
        let s = t % 86_400;
        format!(
            "2026-11-07T{:02}:{:02}:{:02}Z",
            s / 3600,
            s / 60 % 60,
            s % 60
        )
    };
    json!({"mode": "sealed", "updated": stamp(end), "written": {},
        "harness": {"pid": 1, "started": stamp(start), "start_id": 1, "exit": exit}})
}

#[test]
fn the_resume_rule_follows_written_counts_alone() {
    let counted = |written: Value| {
        let mut w = World::new(None);
        w.part(1, false, Some(World::sealed(written)));
        // Session files on disk with nothing counted do not make a conversation (HF6).
        std::fs::write(w.run_dir().join(e(1)).join("session.jsonl"), "{}").unwrap();
        w.resume()
    };
    assert!(counted(json!({"model_requests": 3})));
    assert!(counted(json!({"native_files": 2})));
    assert!(!counted(json!({})));
    assert!(!counted(json!({"model_requests": 0, "native_files": 0})));
}

#[test]
fn a_resume_that_failed_at_once_opens_a_new_session() {
    let after = |resumed: bool, secs: u64, exit: Value| {
        let mut w = World::new(None);
        w.part(1, false, Some(World::sealed(json!({"model_requests": 5}))));
        w.part(2, resumed, Some(ran(secs, exit)));
        w.resume()
    };
    assert!(!after(true, 5, json!({"code": 1})));
    // Not a resume, a clean exit, a kill, or a harness that ran: the rule does not apply.
    assert!(after(false, 5, json!({"code": 1})));
    assert!(after(true, 5, json!({"code": 0})));
    assert!(after(true, 5, json!({"signal": 9})));
    assert!(after(true, 600, json!({"code": 1})));
}

#[test]
fn a_harness_still_running_from_the_last_part_starts_nothing() {
    let harness = Stand::new();
    let mut w = World::new(Some(DEAD_SERVER.into()));
    let running = json!({"harness": {"pid": harness.0.id(), "started": "2026-11-07T10:00:00Z",
        "start_id": harness.alive_as()["start_id"], "exit": null}});
    w.part(1, false, Some(running));
    let before = w.server_json();
    let stop = w.prepare().err().expect("refused");
    assert_eq!(stop.code, 1);
    assert_eq!(w.server_json(), before);
    assert_eq!(w.folders(), 1);
}

#[test]
fn a_part_whose_recorder_lives_is_left_recording() {
    let recorder = Stand::new();
    let mut w = World::new(Some(DEAD_SERVER.into()));
    w.part(1, false, Some(recorder.alive_as()));
    let before = w.server_json();
    let stop = w.prepare().err().expect("refused");
    assert_eq!(stop.code, 1);
    assert_eq!(w.server_json(), before);
    assert_eq!(w.folders(), 1);
}

#[test]
fn a_changed_connector_or_host_file_is_refused_before_any_server_call() {
    let mut w = World::new(Some(DEAD_SERVER.into()));
    w.part(1, false, Some(World::sealed(json!({"model_requests": 1}))));
    let other = w.t.home().join("other.toml");
    std::fs::copy(&w.connector, &other).unwrap();
    let hosts = w.t.home().join("hosts.txt");
    std::fs::write(&hosts, "llm.example.com\n").unwrap();
    let before = w.server_json();
    let cases = [
        w.go(Some(&other), None),
        w.go(None, None),
        w.go(Some(&w.connector), Some(&hosts)),
    ];
    for case in cases {
        let stop = case.err().expect("refused");
        // A server call to the dead address would end with code 3.
        assert_eq!(stop.code, 1, "{}", stop.text);
    }
    assert_eq!(w.server_json(), before);
    assert_eq!(w.folders(), 1);
}

#[test]
fn a_different_folder_is_refused_and_a_part_with_no_recorded_folder_is_not() {
    let mut w = World::new(Some(DEAD_SERVER.into()));
    w.part(1, false, Some(World::sealed(json!({"model_requests": 1}))));
    let elsewhere = w.t.home().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    w.parts[0]["workspace"] = json!(elsewhere.to_string_lossy());
    w.save();
    let (before, folders) = (w.server_json(), w.folders());
    let stop = w.prepare().err().expect("refused");
    assert_eq!(stop.code, 1);
    assert!(
        stop.text.contains(&*elsewhere.to_string_lossy()),
        "{}",
        stop.text
    );
    assert_eq!((w.server_json(), w.folders()), (before, folders));
}

#[test]
fn the_same_folder_or_no_recorded_folder_is_continued() {
    // None: an old server.json has no workspace, so there is nothing to compare. "dot": the
    // same folder spelled with a dot is the same folder. "gone": a moved or renamed folder
    // cannot be gone back to, so the run goes on here.
    for case in ["none", "dot", "gone"] {
        let worker = FakeWorker::start();
        next_part(&worker);
        let mut w = World::new(Some(worker.url()));
        w.part(1, false, Some(World::sealed(json!({"model_requests": 1}))));
        let recorded = match case {
            "dot" => Some(w.project.join(".")),
            "gone" => Some(w.t.home().join("gone")),
            _ => None,
        };
        if let Some(r) = recorded {
            w.parts[0]["workspace"] = json!(r.to_string_lossy());
            w.save();
        }
        w.prepare().unwrap_or_else(|s| panic!("{}", s.text));
    }
}

/// The fake Worker's answer to the continue call: part 2, 3600 s left.
fn next_part(w: &FakeWorker) {
    let answer = json!({"run_id": RUN_ID, "entry_id": e(2), "segment": 1,
        "official_start": "2026-11-06T10:00:00Z", "deadline_at": "2026-11-06T12:00:00Z",
        "seconds_left": 3600});
    w.inject(
        Route::Continue,
        Inject {
            answer: Some((201, answer)),
            ..Inject::default()
        },
    );
}

#[test]
fn continue_adds_the_next_part_and_resumes_the_session() {
    let worker = FakeWorker::start();
    next_part(&worker);
    let mut w = World::new(Some(worker.url()));
    w.part(1, false, Some(World::sealed(json!({"model_requests": 2}))));
    let ready = w.prepare().unwrap_or_else(|s| panic!("{}", s.text));
    assert_eq!(ready.facts.run, RUN_ID);
    assert_eq!(
        (ready.facts.entry.as_str(), ready.facts.segment),
        (&*e(2), 1)
    );
    assert_eq!(ready.facts.entry_dir, w.run_dir().join(e(2)));
    assert!(ready.plan.argv.contains(&"--resume".to_owned()));
    let left = ready.facts.deadline.unwrap() - ready.facts.local_wall;
    assert!(left.abs_diff(3_600_000) < 1_000, "{left}");
    let sj: Value = serde_json::from_slice(&w.server_json()).unwrap();
    let last = sj["entries"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["id"].clone(), last["resumed"].clone()),
        (json!(e(2)), json!(true))
    );
    assert_eq!(sj["entries"].as_array().unwrap().len(), 2);
}

#[test]
fn a_part_that_recorded_nothing_starts_a_new_session() {
    let worker = FakeWorker::start();
    next_part(&worker);
    let mut w = World::new(Some(worker.url()));
    w.part(1, false, Some(World::sealed(json!({}))));
    let ready = w.prepare().unwrap_or_else(|s| panic!("{}", s.text));
    assert!(!ready.plan.argv.contains(&"--resume".to_owned()));
}

#[test]
fn a_run_whose_time_is_up_is_not_continued() {
    let worker = FakeWorker::start();
    worker.inject(
        Route::Continue,
        Inject {
            answer: Some((410, json!({"error": "closed"}))),
            ..Inject::default()
        },
    );
    let mut w = World::new(Some(worker.url()));
    w.part(1, false, Some(World::sealed(json!({"model_requests": 2}))));
    let before = w.server_json();
    let stop = w.prepare().err().expect("refused");
    assert_eq!(stop.code, 1);
    assert_eq!(w.server_json(), before);
    assert_eq!(w.folders(), 1);
}

/// What `continue` prints for a run of `mode` naming `repo` whose start was stopped with Ctrl-C
/// after the open (no part folder, no `first-message.txt`), or whose part recorded `written`.
/// A built-in connector on an empty PATH: nothing real runs and no clipboard tool is found.
fn shown(mode: &str, repo: Option<&str>, written: Option<Value>) -> String {
    let mut w = World::new(None);
    w.part(1, false, written.clone().map(World::sealed));
    if written.is_none() {
        std::fs::remove_dir_all(w.run_dir().join(e(1))).unwrap();
    }
    std::fs::write(w.project.join(".env.local"), "GH_TOKEN=x\n").unwrap();
    let mut sj: Value = serde_json::from_slice(&w.server_json()).unwrap();
    sj["mode"] = json!(mode);
    sj["repo"] = json!(repo);
    sj["connector"] = json!({"id": "codex-cli", "name": "Codex", "source": "builtin"});
    put(&w.run_dir(), "server.json", &sj);
    let bin = w.t.home().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let vars = [("PATH".to_owned(), bin.display().to_string())].into();
    let a = Args {
        run: None,
        connector: None,
        hosts: None,
    };
    let mut out = Vec::new();
    prepare(&a, &w.gc(), &w.project, &vars, &mut out).unwrap_or_else(|s| panic!("{}", s.text));
    String::from_utf8(out).unwrap()
}

#[test]
fn the_push_lines_come_back_after_a_ctrl_c_after_the_open() {
    let said = shown("competition", Some("a/b"), None);
    assert!(said.contains(text::PUSH_FRAME), "{said}");
    assert!(said.contains(text::PROMPT_AFTER_PUSH), "{said}");
    assert!(said.contains("https://github.com/a/b"), "{said}");
    let said = shown("practice", None, None);
    assert!(!said.contains(text::PUSH_FRAME), "{said}");
    // A resumed session has already seen them.
    let said = shown(
        "competition",
        Some("a/b"),
        Some(json!({"model_requests": 2})),
    );
    assert!(!said.contains(text::PUSH_FRAME), "{said}");
}

/// An unreadable harness version goes up on the continue call as the string "unknown", never null.
#[test]
fn an_unread_harness_version_is_sent_as_unknown_on_the_continue_call() {
    use std::io::{BufRead, BufReader, Read, Write};
    let (listener, addr) = common::ports::held();
    let seen = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut r = BufReader::new(stream);
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body).unwrap();
        let answer = json!({"run_id": RUN_ID, "entry_id": e(2), "segment": 1,
            "official_start": "2026-11-06T10:00:00Z", "deadline_at": "2026-11-06T12:00:00Z",
            "seconds_left": 3600})
        .to_string();
        let reply = format!(
            "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        r.get_mut().write_all(reply.as_bytes()).unwrap();
        serde_json::from_slice::<Value>(&body).unwrap()
    });
    let mut w = World::new(Some(format!("http://{addr}")));
    // A program whose version probe fails: the version is unread.
    let script = w.t.home().join("tester-none");
    std::fs::write(&script, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let body = std::fs::read_to_string(&w.connector).unwrap();
    std::fs::write(
        &w.connector,
        body.replace(
            "program = \"true\"",
            &format!("program = \"{}\"", script.display()),
        ),
    )
    .unwrap();
    w.part(1, false, Some(World::sealed(json!({"model_requests": 2}))));
    w.prepare().unwrap_or_else(|s| panic!("{}", s.text));
    assert_eq!(seen.join().unwrap()["harness_version"], json!("unknown"));
    let log = w.run_dir().join(e(2)).join("goalcup.log");
    let log = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(log.contains("version_unread"), "{log}");
}

/// Continues a run started on 1.0 as `opencode2-cli`, whose last part wrote `written`, with a
/// fake `opencode` that prints `line`.
fn continue_opencode2(line: &str, written: Value) -> Result<Ready, Stop> {
    let worker = FakeWorker::start();
    next_part(&worker);
    let mut w = World::new(Some(worker.url()));
    let bin = w.t.home().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("opencode");
    std::fs::write(&fake, format!("#!/bin/sh\necho {line}\n")).unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    w.part(1, false, Some(World::sealed(written)));
    let mut sj: Value = serde_json::from_slice(&w.server_json()).unwrap();
    sj["connector"] = json!({"id": "opencode2-cli", "name": "OpenCode", "source": "builtin"});
    put(&w.run_dir(), "server.json", &sj);
    let a = Args {
        run: None,
        connector: None,
        hosts: None,
    };
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let vars = [("PATH".to_owned(), path)].into();
    prepare(&a, &w.gc(), &w.project, &vars, &mut Vec::new())
}

/// A run started on 1.0 as `opencode2-cli` continues as `opencode-cli`, the one OpenCode connector.
#[test]
fn an_opencode2_run_continues_as_opencode() {
    let ready = continue_opencode2("opencode v2.2.2", json!({"model_requests": 1}))
        .unwrap_or_else(|s| panic!("{}", s.text));
    assert_eq!(ready.plan.connector.id, "opencode-cli");
    assert_eq!(
        ready.plan.harness_version.as_deref(),
        Some("opencode v2.2.2")
    );
}

/// No version refuses a continue, also when it opens a new session (resume off): a 1.x build on a
/// run started as the 2.x id gets the 1.x rules.
#[test]
fn a_continue_without_resume_is_never_refused_on_version() {
    let ready =
        continue_opencode2("opencode 1.19.5", json!({})).unwrap_or_else(|s| panic!("{}", s.text));
    assert!(!ready.plan.argv.contains(&"--continue".to_owned()));
    assert_eq!(
        (&ready.plan.refusal, ready.plan.version_row.as_deref()),
        (&None, Some("1.x"))
    );
}
