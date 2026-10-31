//! `goalcup start` through the hand-over to the watchdog (PRD-10 section 3, steps 9 and 10):
//! the real binary, a stub harness, a run with no server. Facts only: exit codes, which lines
//! come in which order, files on disk, and that no secret value reaches the screen or the log.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::home::TempHome;
use goalcup::cli::text::READY_MARK;

const TOKEN: &str = "ghp_HANDOVER0123456789ABCDEF";

struct Fix {
    t: TempHome,
    project: PathBuf,
}

impl Fix {
    /// A clean project folder holding the token in a covered `.env.local`, and a connector whose
    /// program is the stub script `body`.
    fn new(body: &str) -> Fix {
        let t = TempHome::new();
        let project = t.home().join("p");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(".env.local"), format!("GH_TOKEN={TOKEN}\n")).unwrap();
        std::fs::write(project.join(".gitignore"), ".env.local\n").unwrap();
        let stub = t.home().join("stub.sh");
        std::fs::write(&stub, body).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let toml = format!(
            "id = \"stub\"\nname = \"Stub\"\nprogram = \"{}\"\nskills_dir = \"{{home}}/skills\"\n\
             first_screen = \"Press Enter.\"\n",
            stub.display()
        );
        std::fs::write(t.home().join("stub.toml"), toml).unwrap();
        Fix { t, project }
    }

    fn gc(&self) -> PathBuf {
        self.t.home().join("b/gc")
    }

    fn start(&self) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_goalcup"));
        self.t.apply(&mut cmd);
        cmd.args(["start", "--practice", "--connector"])
            .arg(self.t.home().join("stub.toml"))
            .current_dir(&self.project)
            .env("GOALCUP_HOME", self.gc())
            .env("GOALCUP_SERVER", "off")
            .output()
            .unwrap()
    }

    /// The one entry folder the run made.
    fn entry(&self) -> PathBuf {
        let only = |d: &Path| {
            let mut v: Vec<PathBuf> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.is_dir())
                .collect();
            assert_eq!(v.len(), 1, "{d:?}");
            v.remove(0)
        };
        only(&only(&self.gc().join("runs")))
    }
}

fn texts(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_stub_harness_ends_with_the_end_screen_and_its_own_exit_code() {
    let fix = Fix::new("#!/bin/sh\nexit 7\n");
    let out = fix.start();
    let all = texts(&out);
    assert_eq!(out.status.code(), Some(7), "{all}");
    // The ready line comes before the end screen, which ends with the path of the log.
    let entry = fix.entry();
    let log = entry.join("goalcup.log");
    let (ready, end) = (all.find(READY_MARK), all.find(&*log.to_string_lossy()));
    assert!(ready.is_some() && end.is_some() && ready < end, "{all}");
    // The part is sealed on disk.
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(entry.join("status.json")).unwrap()).unwrap();
    assert_eq!(status["mode"], "sealed", "{status}");
    assert_eq!(status["harness"]["exit"]["code"], 7, "{status}");
    // No secret on screen or in the log.
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!all.contains(TOKEN) && !log_text.contains(TOKEN));
}

#[test]
fn a_harness_that_never_starts_gives_exit_3_and_a_plain_line() {
    // An interpreter that is not there: the program exists but cannot be executed.
    let fix = Fix::new("#!/nonexistent/interpreter\n");
    let out = fix.start();
    let all = texts(&out);
    assert_eq!(out.status.code(), Some(3), "{all}");
    assert!(!all.trim().is_empty(), "nothing was said");
    assert!(!all.contains("panicked") && !all.contains(TOKEN), "{all}");
}
