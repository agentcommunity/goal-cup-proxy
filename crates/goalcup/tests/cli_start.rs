//! `goalcup start` up to the hand-over (PRD-10 section 3): the facts the watchdog gets, a plan
//! refusal, the skill's place, the folder refusal, a denied approval and the deadline. Facts only.

mod common;

use std::path::{Path, PathBuf};

use common::home::TempHome;
use common::worker::{FakeWorker, Inject, RUN_ID, Route};
use goalcup::cli::start::{Args, Ctx, prepare};
use goalcup::cli::{Server, isolated_home, text};
use serde_json::json;

const TOKEN: &str = "ghp_START0123456789ABCDEFGH";

struct Fix {
    t: TempHome,
    project: PathBuf,
    connector: PathBuf,
}

impl Fix {
    /// A clean project folder holding the token in a covered `.env.local`, and a test connector
    /// whose skills folder is `skills`.
    fn new(skills: &str, extra: &str) -> Fix {
        Fix::with_program("true", skills, extra)
    }

    fn with_program(program: &str, skills: &str, extra: &str) -> Fix {
        let t = TempHome::new();
        let project = t.home().join("p");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(".env.local"), format!("GH_TOKEN={TOKEN}\n")).unwrap();
        std::fs::write(project.join(".gitignore"), ".env.local\n").unwrap();
        let connector = t.home().join("tester.toml");
        let body = format!(
            "id = \"tester\"\nname = \"Tester\"\nprogram = \"{program}\"\nskills_dir = \"{skills}\"\n\
             first_screen = \"Press Enter.\"\n{extra}"
        );
        std::fs::write(&connector, body).unwrap();
        Fix {
            t,
            project,
            connector,
        }
    }

    /// Away from the project: the two never sit side by side.
    fn gc(&self) -> PathBuf {
        self.t.home().join("b/gc")
    }

    fn ctx(&self, server: Server) -> Ctx {
        let path = std::env::var("PATH").unwrap_or_default();
        Ctx {
            home: self.gc(),
            project: self.project.clone(),
            vars: [("PATH".to_owned(), path)].into(),
            server,
        }
    }

    fn args(&self) -> Args<'_> {
        Args {
            practice: true,
            connector: Some(&self.connector),
            ..Args::default()
        }
    }

    fn listing(&self, dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

#[test]
fn facts_hold_the_workspace_token_value_and_nothing_else_secret() {
    let fix = Fix::new("{home}/skills", "");
    let mut out = Vec::new();
    let ready = prepare(&fix.args(), &fix.ctx(Server::Off), &mut out)
        .unwrap_or_else(|s| panic!("{}", s.text));
    let facts = serde_json::to_value(&ready.facts).unwrap();
    let values = facts["values"].as_array().unwrap();
    assert!(!values.is_empty());
    let mut rest = facts.clone();
    rest.as_object_mut().unwrap().remove("values");
    assert!(!rest.to_string().contains(TOKEN));
    assert!(!String::from_utf8(out.clone()).unwrap().contains(TOKEN));
}

/// PRD-17 S7: the deadline comes from the server's `seconds_left` on the boot clock, so it sits
/// exactly that far after the wall clock read beside it, whatever the wall clock says.
#[test]
fn the_deadline_is_the_servers_seconds_left_from_now() {
    let fix = Fix::new("{home}/skills", "");
    let w = FakeWorker::start();
    let open = json!({"run_id": RUN_ID, "entry_id": "e0000000000000001", "segment": 0,
        "official_start": "2026-11-06T10:00:00Z", "deadline_at": "2026-11-06T12:00:00Z",
        "seconds_left": 7200});
    w.inject(
        Route::Open,
        Inject {
            answer: Some((201, open)),
            ..Inject::default()
        },
    );
    let ready = prepare(&fix.args(), &fix.ctx(Server::Url(w.url())), &mut Vec::new())
        .unwrap_or_else(|s| panic!("{}", s.text));
    let left = ready.facts.deadline.unwrap() - ready.facts.local_wall;
    assert!(left.abs_diff(7_200_000) < 1_000, "{left}");
}

#[test]
fn a_plan_refusal_names_file_and_key_never_the_value_and_creates_no_run() {
    let extra = "[[refuse_if_set]]\nfile = \"{home}/settings.json\"\nkeys = [\"proxy\"]\n";
    let fix = Fix::new("{home}/skills", extra);
    let iso = isolated_home(&fix.gc(), "tester");
    std::fs::create_dir_all(&iso).unwrap();
    std::fs::write(
        iso.join("settings.json"),
        json!({"proxy": "SECRETVALUE-xyz"}).to_string(),
    )
    .unwrap();
    let stop = prepare(&fix.args(), &fix.ctx(Server::Off), &mut Vec::new())
        .err()
        .unwrap();
    assert_eq!(stop.code, 1);
    assert!(stop.text.contains("settings.json") && stop.text.contains("proxy"));
    assert!(!stop.text.contains("SECRETVALUE"));
    assert!(!fix.gc().join("runs").exists());
}

#[test]
fn the_skill_lands_in_the_harness_home_and_never_in_the_project() {
    let fix = Fix::new("{home}/skills", "");
    let before = fix.listing(&fix.project);
    let ready = prepare(&fix.args(), &fix.ctx(Server::Off), &mut Vec::new())
        .unwrap_or_else(|s| panic!("{}", s.text));
    let skill = ready.plan.home.join("skills/goal-cup/SKILL.md");
    assert!(skill.is_file());
    assert_eq!(fix.listing(&fix.project), before);
}

#[test]
fn every_folder_problem_is_listed_at_once_and_nothing_is_created() {
    let fix = Fix::new("{home}/skills", "");
    std::fs::write(fix.project.join("notes.txt"), "x").unwrap();
    std::fs::write(fix.project.join(".env"), "A=1\n").unwrap();
    let stop = prepare(&fix.args(), &fix.ctx(Server::Off), &mut Vec::new())
        .err()
        .unwrap();
    assert!(stop.text.contains("notes.txt") && stop.text.contains(".env"));
    assert!(stop.text.ends_with(text::NOTHING_CREATED));
    assert!(!fix.gc().join("runs").exists());
}

#[test]
fn a_denied_approval_creates_no_run() {
    let fix = Fix::new("{home}/skills", "");
    let w = FakeWorker::start();
    w.inject(
        Route::DeviceToken,
        Inject {
            answer: Some((403, json!({"error": "denied"}))),
            ..Inject::default()
        },
    );
    let stop = prepare(&fix.args(), &fix.ctx(Server::Url(w.url())), &mut Vec::new())
        .err()
        .unwrap();
    assert_eq!(stop.code, 1);
    assert!(!fix.gc().join("runs").exists());
}

/// Runs the real `goalcup start --practice` in `fix`'s project folder, against `server`, and
/// kills it if it has not ended in two minutes.
fn spawn_start(fix: &Fix, server: &str) -> std::process::Output {
    use std::process::{Command, Stdio};
    let child = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["start", "--practice", "--connector"])
        .arg(&fix.connector)
        .current_dir(&fix.project)
        .env("GOALCUP_SERVER", server)
        .env("GOALCUP_HOME", fix.gc())
        .env("HOME", fix.t.home())
        .env("XDG_CONFIG_HOME", fix.t.home())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let id = child.id();
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let timer = std::thread::spawn(move || {
        // A message or a closed channel means the child ended (its pid may be reused); only a
        // timeout kills it.
        if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
            wait.recv_timeout(std::time::Duration::from_secs(120))
        {
            let _ = Command::new("kill").arg("-9").arg(id.to_string()).status();
        }
    });
    let out = child.wait_with_output().unwrap();
    let _ = done.send(());
    let _ = timer.join();
    out
}

/// PRD-10 section 3 steps 7 to 10: the hand-over. The harness runs, its exit code is the command's
/// exit code, the ready line is printed once, and the end screen follows from the sealed part.
#[cfg(target_os = "linux")]
#[test]
fn the_hand_over_passes_the_exit_code_and_ends_on_the_sealed_counts() {
    let fix = Fix::with_program("sh", "{home}/skills", "args = [\"-c\", \"exit 7\"]\n");
    let w = FakeWorker::start();
    let out = spawn_start(&fix, &w.url());
    let shown = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(7),
        "{shown}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        shown.matches(&text::recording_now("Tester")).count(),
        1,
        "{shown}"
    );
    assert!(shown.contains(text::PART_RECORDED), "{shown}");
    let entry = fix.gc().join("runs").join(RUN_ID);
    let sealed = std::fs::read_dir(&entry)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            std::fs::read_to_string(e.path().join("status.json"))
                .is_ok_and(|s| s.contains("\"sealed\""))
        });
    assert!(sealed);
}

/// A refused start leaves the project folder as it was and creates nothing under the home.
#[test]
fn a_refused_start_leaves_the_folder_and_the_home_untouched() {
    let fix = Fix::new("{home}/skills", "");
    std::fs::write(fix.project.join("notes.txt"), "x").unwrap();
    let before = fix.listing(&fix.project);
    let out = spawn_start(&fix, "off");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fix.listing(&fix.project), before);
    assert!(!fix.gc().exists());
}

/// A harness that itself exits 78 is a recorded part, not a masking refusal: the exit code is the
/// harness's and the end screen is shown.
#[cfg(target_os = "linux")]
#[test]
fn a_harness_exit_of_78_is_not_a_masking_refusal() {
    let fix = Fix::with_program("sh", "{home}/skills", "args = [\"-c\", \"exit 78\"]\n");
    let w = FakeWorker::start();
    let out = spawn_start(&fix, &w.url());
    let shown = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(78), "{shown}");
    assert!(shown.contains(text::PART_RECORDED), "{shown}");
}
