//! The folder checks of `goalcup start` (PRD-10 section 3 step 2): every problem in one refusal,
//! PRD-17 SC1 (the project folder and goalcup's own folders never hold each other) and SC2's folder
//! case (a refused start creates nothing). Facts only: exit codes, names in stderr, the
//! `NOTHING_CREATED` constant, and trees left as they were. No WSL case: there is no WSL box.

mod common;

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::home::TempHome;
use goalcup::cli::{folder, text};

/// A temp `HOME` holding a project folder `a/project` with `files`, and a `GOALCUP_HOME` at
/// `b/gc` that does not exist yet (the two sit in different folders).
struct Setup {
    t: TempHome,
    project: PathBuf,
    gc: PathBuf,
}

impl Setup {
    fn new(files: &[(&str, &str)]) -> Setup {
        let t = TempHome::new();
        let project = t.home().join("a/project");
        std::fs::create_dir_all(&project).unwrap();
        for (name, body) in files {
            std::fs::write(project.join(name), body).unwrap();
        }
        let gc = t.home().join("b/gc");
        Setup { t, project, gc }
    }

    /// `goalcup <args>` run in the project folder with `GOALCUP_HOME=<goalcup_home>`.
    fn start(&self, goalcup_home: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_goalcup"));
        self.t.apply(&mut cmd);
        cmd.args(args)
            .current_dir(&self.project)
            .env("GOALCUP_SERVER", "off")
            .env("XDG_CONFIG_HOME", self.t.home())
            .env("GOALCUP_HOME", goalcup_home)
            .output()
            .unwrap()
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every name in `dir` with its size and mtime, sorted.
fn snapshot(dir: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut all: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let meta = std::fs::symlink_metadata(e.path()).unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            (name, meta.len(), meta.modified().unwrap())
        })
        .collect();
    all.sort();
    all
}

#[test]
fn three_problems_one_refusal() {
    let s = Setup::new(&[
        ("notes.txt", "x"),
        (".env", "A=1\n"),
        (".env.local", "GH_TOKEN=x\n"),
        (".gitignore", ".env.local\n"),
        ("prompt.md", "go"),
    ]);
    let prompt = s.project.join("prompt.md");
    let args = [
        "start",
        "--test",
        "--repo",
        "a/b",
        "--prompt",
        prompt.to_str().unwrap(),
    ];
    let out = s.start(&s.gc, &args);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(err.matches(text::FOLDER_REFUSED).count(), 1, "{err}");
    assert!(err.contains("\"notes.txt\""), "{err}");
    assert!(err.contains("\".env\""), "{err}");
    assert!(err.contains(&prompt.display().to_string()), "{err}");
    assert!(err.contains(text::NOTHING_CREATED), "{err}");
}

/// L-SC1, with a relative `GOALCUP_HOME` among the inputs (PRD-17 note on SC1).
#[test]
fn project_folder_and_goalcup_home_never_hold_each_other() {
    let s = Setup::new(&[(".gitignore", ".env*\n")]);
    let practice = ["start", "--practice"];
    // The same folder with a home elsewhere passes the checks and reaches the stub (exit 3).
    assert_eq!(s.start(&s.gc, &practice).status.code(), Some(3));

    // The project holds the home: a relative GOALCUP_HOME taken against the project folder.
    let out = s.start(Path::new("gc"), &practice);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let real = s.project.canonicalize().unwrap();
    assert!(stderr(&out).contains(&real.join("gc").display().to_string()));
    assert!(!s.project.join("gc").exists());

    // The project inside the home.
    let home = s.t.home().join("h");
    let inner = Setup {
        t: TempHome::new(),
        project: home.join("p"),
        gc: home.clone(),
    };
    std::fs::create_dir_all(&inner.project).unwrap();
    let out = inner.start(&inner.gc, &practice);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains(&home.display().to_string()));

    // Through a link: only the resolved path shows the home is the project, or inside it.
    let link = s.t.home().join("b/link");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&s.project, &link).unwrap();
    for gc in [link.clone(), link.join("gc")] {
        let out = s.start(&gc, &practice);
        assert_eq!(out.status.code(), Some(1), "{gc:?}: {}", stderr(&out));
        assert!(stderr(&out).contains(&gc.display().to_string()));
        assert!(stderr(&out).contains(text::NOTHING_CREATED));
    }
    assert!(!s.project.join("gc").exists());
}

/// L-SC2, the folder part.
#[test]
fn refused_start_creates_nothing() {
    let s = Setup::new(&[("notes.txt", "x"), (".env", "A=1\n")]);
    let before = snapshot(&s.project);
    let out = s.start(&s.gc, &["start", "--practice"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains(text::NOTHING_CREATED));
    assert!(!s.gc.exists());
    assert!(!s.gc.parent().unwrap().exists());
    assert_eq!(snapshot(&s.project), before);
}

#[test]
fn overlayfs_and_tmpfs_are_accepted() {
    for magic in [0x794c_7630, 0x0102_1994] {
        assert_eq!(folder::network_fs(magic, ""), None, "{magic:#x}");
    }
    for magic in [0x6969, 0xFF53_4D42, 0xFE53_4D42, 0x517B] {
        assert!(folder::network_fs(magic, "").is_some(), "{magic:#x}");
    }
    for name in ["nfs", "smbfs"] {
        assert!(folder::network_fs(0, name).is_some(), "{name}");
    }
    assert_eq!(folder::network_fs(0, "apfs"), None);

    // The starting kit, on whatever this machine's temp folder is (overlayfs in a build box).
    let s = Setup::new(&[
        (".gitignore", ".env*\n"),
        (".env.local", "GH_TOKEN=x\n"),
        (".DS_Store", ""),
    ]);
    let got = folder::check(&s.project, &s.gc, true, None);
    assert!(got.is_ok(), "{got:?}");

    // The same kit on tmpfs, where this machine has one.
    let Ok(shm) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    let project = shm.path().join("a/project");
    std::fs::create_dir_all(&project).unwrap();
    for name in [".gitignore", ".env.local", ".DS_Store"] {
        std::fs::copy(s.project.join(name), project.join(name)).unwrap();
    }
    let got = folder::check(&project, &shm.path().join("b/gc"), true, None);
    assert!(got.is_ok(), "{got:?}");
}
