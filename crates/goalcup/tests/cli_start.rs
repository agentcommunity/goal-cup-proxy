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
        let t = TempHome::new();
        let project = t.home().join("p");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(".env.local"), format!("GH_TOKEN={TOKEN}\n")).unwrap();
        std::fs::write(project.join(".gitignore"), ".env.local\n").unwrap();
        let connector = t.home().join("tester.toml");
        let body = format!(
            "id = \"tester\"\nname = \"Tester\"\nprogram = \"true\"\nskills_dir = \"{skills}\"\n\
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
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["source"], "workspace");
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
    let ready = prepare(
        &fix.args(),
        &fix.ctx(Server::Url(w.url())),
        &mut Vec::new(),
    )
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
    assert!(
        std::fs::read_to_string(skill)
            .unwrap()
            .contains("/goal Cup")
    );
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
