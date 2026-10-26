//! A real run's GitHub half (D69): the open repository check, the repository question, and the
//! older versions' credential helper, which still answers git for the clones they made.
//!
//! The GitHub API is a local fake on loopback; `git` is a wrapper script that runs the real one
//! against a gitconfig of the test's own. Nothing here reads `$HOME`, `~/.gitconfig` or a real
//! token, or reaches GitHub.

use goalcup_cli::CliError;
use goalcup_cli::account::github::{Found, GithubSetup, check_public_empty, credential_helper};
use goalcup_cli::commands::run_repo;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A made-up token: the shape of a fine-grained one, and nothing GitHub would accept.
fn token() -> String {
    format!("github_pat_{}", "A".repeat(60))
}

const REPO_URL: &str = "https://github.com/acme/empty-repo";

fn setup() -> GithubSetup {
    GithubSetup::new(REPO_URL, &token()).unwrap()
}

// ------------------------------------------------------------------ the fake GitHub API

/// One request: its path and `Authorization` header.
type Seen = (String, Option<String>);

struct FakeGithub {
    base: String,
    requests: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl FakeGithub {
    /// Answers every request with `status` and `body`, and records its request line path and
    /// `Authorization` header. The list of commits answers `409`: the repository is empty.
    fn start(status: u16, body: &str) -> FakeGithub {
        FakeGithub::answers(&[(status, body)])
    }

    /// Answers the n-th request with `answers[n]`, and every one after the last with the last;
    /// the list of commits answers `409`.
    fn answers(answers: &[(u16, &str)]) -> FakeGithub {
        FakeGithub::serve(answers, 409)
    }

    /// As [`FakeGithub::answers`], with the list of commits (`.../commits?per_page=1`) answering
    /// `commits`, apart from the n-th count.
    fn serve(answers: &[(u16, &str)], commits: u16) -> FakeGithub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let answers: Vec<(u16, String)> =
            answers.iter().map(|(s, b)| (*s, b.to_string())).collect();
        let (log, halt) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let head = read_request(&mut stream);
                let mut lines = head.lines();
                let path = lines
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("")
                    .to_string();
                let auth = lines
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                    .map(|(_, v)| v.trim().to_string());
                let listed = path.contains("/commits");
                let n = {
                    let mut seen = log.lock().unwrap();
                    seen.push((path, auth));
                    seen.iter().filter(|(p, _)| !p.contains("/commits")).count()
                };
                let commits_answer = (commits, "[]".to_string());
                let (status, body) = if listed {
                    &commits_answer
                } else {
                    &answers[n.saturating_sub(1).min(answers.len() - 1)]
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
                // Send the end of the answer, then let the client be the one to close. A server
                // that closes first, with a byte of the request still on its way, answers that
                // byte with a reset, and macOS then refuses the client's next `setsockopt`
                // (`set_read_timeout` on the answer) with EINVAL: "Invalid argument (os error 22)".
                let _ = stream.shutdown(Shutdown::Write);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut rest = [0u8; 512];
                while matches!(stream.read(&mut rest), Ok(n) if n > 0) {}
            }
        });
        FakeGithub {
            base: format!("http://127.0.0.1:{port}"),
            requests,
            stop,
            port,
        }
    }
}

impl FakeGithub {
    /// How many times the repository itself was asked about (the list of commits not counted).
    fn repo_asks(&self) -> usize {
        let seen = self.requests.lock().unwrap();
        seen.iter().filter(|(p, _)| !p.contains("/commits")).count()
    }
}

impl Drop for FakeGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The whole request, so that nothing of it is left unread when the answer goes out: the head,
/// then the `Content-Length` bytes of the body (ureq sends the two as separate writes). Returns
/// the head.
fn read_request(stream: &mut TcpStream) -> String {
    let head = read_head(stream);
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let _ = std::io::copy(&mut stream.take(length), &mut std::io::sink());
    head
}

fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// GitHub's open API is asked with no login at all: a public, empty repository passes; one
/// that cannot be seen (missing, or private) is `Missing`, for start to create; a rate limit
/// (`403`, `429`) is `Unchecked`, never "used"; a repository with commits, or an answer that is
/// not one, is refused in a sentence of its own.
#[test]
fn the_repository_check_sends_no_login_and_tells_missing_from_used() {
    let cases = [
        (
            200,
            r#"{"private":false,"size":0,"owner":{"id":7}}"#,
            Ok(Found::PublicEmpty { owner_id: Some(7) }),
        ),
        (404, r#"{"message":"Not Found"}"#, Ok(Found::Missing)),
        (200, r#"{"private":true,"size":0}"#, Ok(Found::Missing)),
        (
            200,
            r#"{"private":false,"size":12}"#,
            Err("must be empty when the run opens"),
        ),
        (
            403,
            r#"{"message":"API rate limit exceeded"}"#,
            Ok(Found::Unchecked),
        ),
        (
            429,
            r#"{"message":"Too Many Requests"}"#,
            Ok(Found::Unchecked),
        ),
        (500, r#"{"message":"Server Error"}"#, Err("could not check")),
    ];
    for (status, body, want) in cases {
        let fake = FakeGithub::start(status, body);
        let got = check_public_empty(&fake.base, REPO_URL);
        match (&got, want) {
            (Ok(found), Ok(expected)) => assert_eq!(*found, expected, "{status} {body}"),
            (Err(CliError::Refused(said)), Err(part)) => {
                assert!(said.contains(part), "{status} {body}: {said}")
            }
            other => panic!("{status} {body}: {other:?}"),
        }
        let seen = fake.requests.lock().unwrap().clone();
        assert_eq!(seen[0].0, "/repos/acme/empty-repo");
        assert!(
            seen.iter().all(|(_, auth)| auth.is_none()),
            "a login was sent"
        );
    }
}

// ------------------------------------------------------------------ a real run's repository

/// [`run_repo::Doors`] as a test states them: GitHub's API is a fake, and what the participant
/// types is a queue.
struct Doors {
    api: String,
    typed: RefCell<Vec<&'static str>>,
    opened: RefCell<Vec<String>>,
}

impl run_repo::Doors for Doors {
    fn api(&self) -> Option<String> {
        Some(self.api.clone())
    }
    fn read_line(&self) -> Option<String> {
        let mut typed = self.typed.borrow_mut();
        (!typed.is_empty()).then(|| format!("{}\n", typed.remove(0)))
    }
    fn interrupted(&self) -> bool {
        false
    }
    fn open(&self, url: &str) -> bool {
        self.opened.borrow_mut().push(url.to_string());
        true
    }
}

fn doors(api: &FakeGithub, typed: Vec<&'static str>) -> Doors {
    Doors {
        api: api.base.clone(),
        typed: RefCell::new(typed),
        opened: RefCell::new(Vec::new()),
    }
}

const EMPTY: (u16, &str) = (200, r#"{"private":false,"size":0}"#);
const USED: (u16, &str) = (200, r#"{"private":false,"size":12}"#);
const MISSING: (u16, &str) = (404, r#"{"message":"Not Found"}"#);

/// A real run asks the repository's name with its owner, opens GitHub's "new repository" page
/// with it when it does not exist yet, waits for Enter, and then checks it is public and empty.
/// Nothing is kept until a run opens with it.
#[test]
fn a_real_run_names_its_repository_creates_it_and_checks_it() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let api = FakeGithub::answers(&[MISSING, EMPTY]);
    let doors = doors(&api, vec!["you/my-goal-cup", ""]);
    let mut out = Vec::new();
    let repo = run_repo::choose(&doors, &cfg, None, &mut out).unwrap();
    let said = String::from_utf8(out).unwrap();
    assert_eq!(repo.repo_url, "https://github.com/you/my-goal-cup");
    assert_eq!(
        *doors.opened.borrow(),
        ["https://github.com/new?name=my-goal-cup&owner=you&visibility=public"]
    );
    assert!(said.contains("you/my-goal-cup: public and empty"), "{said}");
    assert_eq!(api.repo_asks(), 2);
    assert!(
        goalcup_cli::prep_file::read_prep_lenient(&cfg)
            .repo_url
            .is_empty()
    );
}

/// A name without its owner is refused with the form to type, and GitHub is not asked.
#[test]
fn a_repository_name_without_its_owner_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let api = FakeGithub::answers(&[EMPTY]);
    let doors = doors(&api, vec!["my-goal-cup"]);
    let refused = run_repo::choose(&doors, &tmp.path().join("cfg"), None, &mut Vec::new())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("<your-user>/my-goal-cup"), "{refused}");
    assert!(api.requests.lock().unwrap().is_empty());
}

/// A repository that is not empty is refused before the approval, naming `--repo`, and nothing is
/// kept.
#[test]
fn a_repository_that_is_used_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let api = FakeGithub::answers(&[USED]);
    let refused = run_repo::choose(
        &doors(&api, vec![]),
        &cfg,
        Some("you/my-goal-cup"),
        &mut Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("must be empty when the run opens") && refused.contains("--repo"),
        "{refused}"
    );
    assert!(
        goalcup_cli::prep_file::read_prep_lenient(&cfg)
            .repo_url
            .is_empty()
    );
}

/// The next real run asks again and suggests a new name once the last run's repository is used.
#[test]
fn the_next_real_run_suggests_a_new_name_once_the_last_is_used() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    goalcup_cli::prep_file::write_prep(
        &cfg,
        &goalcup_cli::prep_file::PrepFile {
            repo_url: "https://github.com/you/my-goal-cup".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    // GitHub is asked once, about the chosen repository; the suggestion asks nothing.
    let api = FakeGithub::answers(&[EMPTY]);
    let mut out = Vec::new();
    let repo = run_repo::choose(&doors(&api, vec![""]), &cfg, None, &mut out).unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("[you/my-goal-cup-2]"), "{said}");
    assert_eq!(repo.repo_url, "https://github.com/you/my-goal-cup-2");
    assert_eq!(api.repo_asks(), 1);
}

/// GitHub's open API allows 60 calls an hour per address. A rate limit (`403`) is not a used
/// repository: start says so and goes on.
#[test]
fn a_rate_limited_github_is_said_and_never_counts_as_used() {
    let tmp = tempfile::tempdir().unwrap();
    let api = FakeGithub::answers(&[(403, r#"{"message":"API rate limit exceeded"}"#)]);
    let mut out = Vec::new();
    let repo = run_repo::choose(
        &doors(&api, vec![]),
        &tmp.path().join("cfg"),
        Some("you/my-goal-cup"),
        &mut out,
    )
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("GitHub's API is busy"), "{said}");
    assert_eq!(repo.repo_url, "https://github.com/you/my-goal-cup");
}

// ------------------------------------------------------------------ the helper

fn ask(cfg: &Path, op: &str, input: &str) -> Result<String, CliError> {
    let mut out = Vec::new();
    credential_helper(cfg, op, &mut input.as_bytes(), &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn the_helper_answers_only_for_its_own_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let stored = setup().store(&cfg).unwrap();
    assert_eq!(stored, cfg.join("github.json"));

    let answer = format!("username=x-access-token\npassword={}\n", token());
    for path in [
        "acme/empty-repo.git",
        "acme/empty-repo",
        "ACME/Empty-Repo.git",
    ] {
        let said = ask(
            &cfg,
            "get",
            &format!("protocol=https\nhost=github.com\npath={path}\n\n"),
        )
        .unwrap();
        assert_eq!(said, answer, "path {path}");
    }
    // The request may end at EOF without a blank line, and the host's case does not matter.
    let said = ask(
        &cfg,
        "get",
        "protocol=https\nhost=GitHub.com\npath=acme/empty-repo.git\n",
    )
    .unwrap();
    assert_eq!(said, answer);

    let nothing = [
        (
            "get",
            "protocol=https\nhost=github.com\npath=acme/other.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=github.com\npath=other/empty-repo.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=github.com\npath=acme/empty-repo.git.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=github.com\npath=acme/empty-repo/extra\n\n",
        ),
        ("get", "protocol=https\nhost=github.com\n\n"),
        (
            "get",
            "protocol=http\nhost=github.com\npath=acme/empty-repo.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=gitlab.com\npath=acme/empty-repo.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=github.com.evil.example\npath=acme/empty-repo.git\n\n",
        ),
        (
            "get",
            "protocol=https\nhost=github.com\npath=acme/empty-repo.git\nhost=evil.example\n\n",
        ),
        ("get", ""),
        (
            "store",
            "protocol=https\nhost=github.com\npath=acme/empty-repo.git\nusername=u\npassword=p\n\n",
        ),
        (
            "erase",
            "protocol=https\nhost=github.com\npath=acme/empty-repo.git\n\n",
        ),
        (
            "unknown",
            "protocol=https\nhost=github.com\npath=acme/empty-repo.git\n\n",
        ),
    ];
    for (op, input) in nothing {
        let said = ask(&cfg, op, input).unwrap();
        assert_eq!(said, "", "{op} {input:?} was answered");
    }
    // A request that never ends within the bound is not answered either.
    let huge = format!(
        "protocol=https\nhost=github.com\npath=acme/empty-repo.git\nx={}\n",
        "a".repeat(200_000)
    );
    assert_eq!(ask(&cfg, "get", &huge).unwrap(), "");

    // The file is read only once a request has matched on protocol and host: a request for
    // anything else is answered with nothing even where there is no github.json at all.
    let empty = tmp.path().join("no-cfg");
    assert_eq!(
        ask(
            &empty,
            "get",
            "protocol=https\nhost=gitlab.com\npath=a/b.git\n\n"
        )
        .unwrap(),
        ""
    );
    assert_eq!(
        ask(&empty, "store", "protocol=https\nhost=github.com\n\n").unwrap(),
        ""
    );
    // ... and a matching request without one is an error git will show.
    let sentence = match ask(
        &empty,
        "get",
        "protocol=https\nhost=github.com\npath=a/b.git\n\n",
    ) {
        Err(CliError::Refused(sentence)) => sentence,
        other => panic!("{other:?}"),
    };
    assert!(sentence.contains("older goalcup"), "{sentence}");
}

// ------------------------------------------------------------------ an old clone

struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    git: PathBuf,
    gitconfig: PathBuf,
}

/// A `git` wrapper that runs the real git against a gitconfig of its own.
fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
    assert!(real.starts_with('/'), "no git on PATH: {real:?}");
    let gitconfig = root.join("gitconfig");
    std::fs::write(&gitconfig, "").unwrap();
    let git = root.join("git");
    std::fs::write(
        &git,
        format!(
            "#!/bin/sh\nGIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=\"{cfg}\" exec \"{real}\" \"$@\"\n",
            cfg = gitconfig.display(),
        ),
    )
    .unwrap();
    make_executable(&git);
    Rig {
        _tmp: tmp,
        root,
        git,
        gitconfig,
    }
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git_out(rig: &Rig, dir: &Path, args: &[&str]) -> String {
    let out = Command::new(&rig.git)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// A clone an older version made names `goalcup git-credential` in its own configuration and
/// pushes with the token in `github.json`. Those clones keep working: the hidden command still
/// answers for that repository, from the stored file, and a helper of the participant's is
/// still reset for it.
#[test]
fn an_old_clone_with_the_goalcup_helper_still_gets_its_token() {
    let rig = rig();
    let dest = rig.root.join("old-clone");
    let cfg = rig.root.join("it's my cfg");
    setup().store(&cfg).unwrap();
    let init = Command::new(&rig.git)
        .args(["init", "--quiet"])
        .arg(&dest)
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    // What versions before D69 wrote into each clone.
    let goalcup = env!("CARGO_BIN_EXE_goalcup");
    let helper = format!(
        "!GOALCUP_CONFIG_DIR='{}' '{goalcup}' git-credential",
        cfg.display().to_string().replace('\'', "'\\''")
    );
    for (key, value) in [
        ("credential.https://github.com.helper", ""),
        ("credential.https://github.com.helper", helper.as_str()),
    ] {
        git_out(&rig, &dest, &["config", "--local", "--add", key, value]);
    }
    git_out(
        &rig,
        &dest,
        &["config", "--local", "credential.useHttpPath", "true"],
    );
    std::fs::write(
        &rig.gitconfig,
        "[credential]\n\thelper = \"!f() { echo username=leak; echo password=leaked; }; f\"\n",
    )
    .unwrap();
    let mut asked = Command::new(&rig.git)
        .args(["credential", "fill"])
        .current_dir(&dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    asked
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=github.com\npath=acme/empty-repo.git\n\n")
        .unwrap();
    let out = asked.wait_with_output().unwrap();
    let said = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(said.contains(&format!("password={}\n", token())), "{said}");
    assert!(!said.contains("leaked"), "{said}");
}

// ------------------------------------------------------------------ the stored form

/// The older versions' stored form, which the hidden helper still reads (D69).
#[test]
fn the_old_setup_is_read_for_the_owner_alone_and_checked_on_the_way_out() {
    for good in [
        "https://github.com/acme/repo",
        "https://github.com/acme/repo.git",
        "https://github.com/acme/repo/",
        "https://github.com/acme/repo.git/",
        "https://github.com/Acme-1/my.repo_x-y",
    ] {
        let s = GithubSetup::new(good, &token()).unwrap();
        assert!(
            !s.repo_url.ends_with(['/']) && !s.repo_url.ends_with(".git"),
            "{good}"
        );
    }
    assert_eq!(
        GithubSetup::new("https://github.com/acme/repo.git/", &token())
            .unwrap()
            .repo_url,
        "https://github.com/acme/repo"
    );
    for bad in [
        "http://github.com/acme/repo",
        "git@github.com:acme/repo.git",
        "https://user:pw@github.com/acme/repo",
        "https://gitlab.com/acme/repo",
        "https://github.com/acme",
        "https://github.com/acme/repo/tree/main",
        "https://github.com/acme/repo?x=1",
        "https://github.com/acme/..",
        "https://github.com/acme/.",
        "https://github.com/acme/repo.git.git",
        "https://github.com/ac_me/repo",
        "https://github.com//repo",
        "https://github.com/acme/re%70o",
    ] {
        match GithubSetup::new(bad, &token()) {
            Err(CliError::Refused(why)) => {
                assert!(!why.contains('@'), "{why}");
                assert!(why.contains("https://github.com/"), "{why}");
            }
            other => panic!("{bad:?} was accepted: {other:?}"),
        }
    }
    for bad in [
        "",
        "short",
        &"A".repeat(256),
        "has space in it, long enough yes",
    ] {
        match GithubSetup::new(REPO_URL, bad) {
            Err(CliError::Refused(why)) => assert!(!why.contains(bad) || bad.is_empty(), "{why}"),
            other => panic!("token {bad:?} was accepted: {other:?}"),
        }
    }

    // A token pasted at the address prompt is not echoed back.
    match GithubSetup::new(&token(), &token()) {
        Err(CliError::Refused(why)) => assert!(!why.contains(&token()), "{why}"),
        other => panic!("{other:?}"),
    }

    let s = setup();
    assert!(
        !format!("{s:?}").contains(&token()),
        "Debug prints the token"
    );

    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let missing = GithubSetup::load(&cfg).unwrap_err().to_string();
    assert!(missing.contains("older goalcup"), "{missing}");
    let path = s.store(&cfg).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "github.json is mode {:o}", mode & 0o777);
        let back = GithubSetup::load(&cfg).unwrap();
        assert_eq!(back.repo_url, REPO_URL);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(GithubSetup::load(&cfg), Err(CliError::Refused(_))));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::write(
        &path,
        format!(
            r#"{{"repo_url":"{REPO_URL}","token":"{}","extra":1}}"#,
            token()
        ),
    )
    .unwrap();
    assert!(matches!(GithubSetup::load(&cfg), Err(CliError::Refused(_))));
}

// ============================================================ the device flow and the login

use crate::common::upload_server::{Approval, ServerOptions, UploadServer};
use goalcup_cli::account::credential::{self, RunCredential};
use goalcup_cli::account::device::{Approved, Purpose, RunMode, approve};
use goalcup_cli::commands::login::{self, LoginArgs};
use goalcup_cli::server::{ServerConfig, read_server_config};
use goalcup_cli::upload::http::{ServerClient, ServerError};
use std::cell::RefCell;

/// The stand-in on loopback, approving every device request as `member-1`, with its store in
/// `dir`.
fn stand_in(dir: &Path) -> UploadServer {
    UploadServer::start(dir, ServerOptions::default())
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Whether `text` holds a run of 64 or more lowercase hex digits (a device code, or the hex of
/// a credential).
fn has_hex64(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_digit() || ('a'..='f').contains(&c) {
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

/// Whether `text` holds an `XXXX-XXXX` user code.
fn has_user_code(text: &str) -> bool {
    const ALPHABET: &str = "BCDFGHJKLMNPQRSTVWXZ";
    text.as_bytes().windows(9).any(|w| {
        w.iter().enumerate().all(|(i, b)| {
            if i == 4 {
                *b == b'-'
            } else {
                ALPHABET.as_bytes().contains(b)
            }
        })
    })
}

/// The secret of `c`. `secret()` is crate-private on purpose, so a test reads what the tracker
/// itself wrote: the `credential` field of the stored file.
fn secret_of(c: &RunCredential) -> String {
    let dir = tempfile::tempdir().unwrap();
    credential::store(dir.path(), c).unwrap();
    let text = std::fs::read_to_string(dir.path().join("credential")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["credential"].as_str().unwrap().to_string()
}

fn browser_opens(_: &str) -> bool {
    true
}

#[test]
fn login_stores_the_handle_and_no_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(&tmp.path().join("store"));
    let cfg = tmp.path().join("cfg");
    let mut out = Vec::new();
    login::run(
        &LoginArgs {
            server: Some(server.url()),
        },
        &cfg,
        &browser_opens,
        &mut out,
    )
    .unwrap();

    let stored = read_server_config(&cfg).unwrap().unwrap();
    assert_eq!(stored.account.as_deref(), Some("member-1"));
    assert_eq!(stored.url, server.url());
    let printed = String::from_utf8(out).unwrap();
    // The login says which approval it is, that it is once per computer, and what comes next.
    assert!(printed.contains("member-1"), "{printed}");
    assert!(printed.contains("once per computer"), "{printed}");
    assert_eq!(
        printed.lines().last(),
        Some("Next: goalcup setup"),
        "{printed}"
    );

    let files = files_under(&cfg);
    assert_eq!(files.len(), 1, "{files:?}");
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("gcr_"), "{}", file.display());
        assert!(!has_hex64(&text), "{}", file.display());
    }
    // The login asked for a login, and said nothing of competition.
    let asked = server.device_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].purpose, "login");
    assert_eq!(asked[0].mode, "practice");
}

#[test]
fn only_the_user_code_is_printed() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(tmp.path());
    let opened = RefCell::new(Vec::new());
    let open = |uri: &str| {
        opened.borrow_mut().push(uri.to_string());
        true
    };
    let mut out = Vec::new();
    let approved = approve(
        &ServerClient::anonymous(&server.url()),
        Purpose::Run {
            mode: RunMode::Practice,
            harness: "Some Agent",
        },
        "codex-cli",
        &open,
        &mut out,
    )
    .unwrap();
    let Approved::Run(credential) = approved else {
        panic!("a run approval returns a credential");
    };
    let printed = String::from_utf8(out).unwrap();
    assert!(has_user_code(&printed), "{printed}");
    assert!(!has_hex64(&printed), "{printed}");
    assert!(!printed.contains("gcr_"), "{printed}");
    assert!(!printed.contains(&secret_of(&credential)), "{printed}");
    let opened = opened.into_inner();
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert!(opened[0].starts_with("https://"), "{opened:?}");
    assert!(opened[0].contains("?code="), "{opened:?}");
    assert!(printed.contains(&opened[0]), "{printed}");
    // It says which run this approves, of which harness, and that a second approval is normal.
    assert!(printed.contains("practice run of Some Agent"), "{printed}");
    assert!(printed.contains("normal"), "{printed}");

    let asked = server.device_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].purpose, "run");
    assert_eq!(asked[0].mode, "practice");
    assert_eq!(asked[0].connector_id, "codex-cli");
    assert_eq!(asked[0].proxy_version, goalcup_cli::CLI_VERSION);
}

#[test]
fn a_browser_that_will_not_open_is_asked_for_by_hand() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(tmp.path());
    let mut out = Vec::new();
    approve(
        &ServerClient::anonymous(&server.url()),
        Purpose::Run {
            mode: RunMode::Competition,
            harness: "Some Agent",
        },
        "codex-cli",
        &|_: &str| false,
        &mut out,
    )
    .unwrap();
    let printed = String::from_utf8(out).unwrap();
    // No browser opened: one line names the address and the code, and none says it opened.
    let line = printed
        .lines()
        .find(|line| line.contains("shows the code"))
        .unwrap_or_else(|| panic!("{printed}"));
    assert!(line.contains("?code="), "{printed}");
    assert!(!printed.contains("browser opened"), "{printed}");
    assert_eq!(server.device_requests()[0].mode, "competition");
}

#[test]
fn an_expired_code_says_start_again_and_creates_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(&tmp.path().join("store"));
    server.set_approval(Approval::Expire);
    let cfg = tmp.path().join("cfg");
    let refusal = login::run(
        &LoginArgs {
            server: Some(server.url()),
        },
        &cfg,
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    assert!(
        refusal
            .to_string()
            .starts_with(goalcup_cli::participant::RUN_AGAIN),
        "{refusal}"
    );
    assert!(files_under(&cfg).is_empty());
    assert!(!cfg.exists(), "a failed login created the config directory");
}

#[test]
fn a_denied_approval_creates_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(&tmp.path().join("store"));
    server.set_approval(Approval::Deny);
    let cfg = tmp.path().join("cfg");
    let refusal = login::run(
        &LoginArgs {
            server: Some(server.url()),
        },
        &cfg,
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    assert!(refusal.to_string().contains("denied"), "{refusal}");
    assert!(files_under(&cfg).is_empty());
    assert!(!cfg.exists(), "a failed login created the config directory");
}

#[test]
fn an_unreachable_server_says_nothing_was_created() {
    // A port nothing listens on: bind, note it, drop.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let refusal = approve(
        &ServerClient::anonymous(&format!("http://127.0.0.1:{port}")),
        Purpose::Login,
        "none",
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    let said = refusal.to_string();
    assert!(said.contains("could not be reached"), "{said}");
    assert!(said.contains("Nothing was created"), "{said}");
}

#[test]
fn a_login_keeps_the_server_it_was_given_before() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(&tmp.path().join("store"));
    let cfg = tmp.path().join("cfg");
    login::run(
        &LoginArgs {
            server: Some(server.url()),
        },
        &cfg,
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap();
    // No `--server` the second time: the stored one is used.
    login::run(
        &LoginArgs { server: None },
        &cfg,
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(server.device_requests().len(), 2);
    let stored = read_server_config(&cfg).unwrap().unwrap();
    assert_eq!(stored.url, server.url());
}

#[test]
fn a_run_approval_is_stored_and_loaded_back() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(&tmp.path().join("store"));
    let anonymous = ServerClient::anonymous(&server.url());
    let Approved::Run(credential) = approve(
        &anonymous,
        Purpose::Run {
            mode: RunMode::Practice,
            harness: "Some Agent",
        },
        "codex-cli",
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap() else {
        panic!("a run approval returns a credential");
    };
    let secret = secret_of(&credential);
    assert!(secret.starts_with("gcr_"));
    assert_eq!(secret.len(), 68);
    let shown = format!("{credential:?}");
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(!shown.contains(&secret), "{shown}");
    assert!(shown.contains(&credential.expires_at), "{shown}");

    let run_dir = tmp.path().join("runs").join("run-1");
    credential::store(&run_dir, &credential).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let file = run_dir.join("credential");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "credential is mode {:o}", mode & 0o777);
        let dir = std::fs::metadata(&run_dir).unwrap().permissions().mode();
        assert_eq!(dir & 0o777, 0o700, "run dir is mode {:o}", dir & 0o777);
    }
    let back = credential::load(&run_dir).unwrap();
    assert_eq!(secret_of(&back), secret_of(&credential));
    assert_eq!(back.expires_at, credential.expires_at);

    // The credential authenticates; no header at all does not.
    let with = ServerClient::with_credential(&server.url(), &back);
    let unknown_run: Result<serde_json::Value, ServerError> =
        with.json("GET", "/v1/runs/r0000000000000000", None::<&()>);
    assert!(
        matches!(unknown_run, Err(ServerError::Status { status: 404, .. })),
        "{unknown_run:?}"
    );
    let without: Result<serde_json::Value, ServerError> =
        anonymous.json("GET", "/v1/runs/r0000000000000000", None::<&()>);
    assert!(
        matches!(without, Err(ServerError::Status { status: 401, .. })),
        "{without:?}"
    );
}

#[test]
fn a_credential_file_readable_by_others_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("run");
    let secret = format!("gcr_{}", "ab".repeat(32));
    let credential = RunCredential::new(secret.clone(), "2026-10-14T12:00:00Z".into()).unwrap();
    credential::store(&run_dir, &credential).unwrap();
    let file = run_dir.join("credential");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refusal = credential::load(&run_dir).unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    let said = refusal.to_string();
    assert!(said.contains("chmod 600"), "{said}");
    assert!(said.contains(&file.display().to_string()), "{said}");
    assert!(!said.contains(&secret), "{said}");
}

#[test]
fn a_missing_or_damaged_credential_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("run");
    let missing = credential::load(&run_dir).unwrap_err();
    assert!(matches!(missing, CliError::Refused(_)), "{missing:?}");
    assert!(
        missing.to_string().contains("no saved credential"),
        "{missing}"
    );

    // Constructor: exactly gcr_ + 64 lowercase hex, and the refusal never repeats the value.
    let bad = ["gcr_abc", &format!("gcr_{}", "AB".repeat(32)), "xyz"];
    for value in bad {
        let refusal = RunCredential::new(value.into(), "t".into()).unwrap_err();
        assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
        assert!(!refusal.to_string().contains(value), "{refusal}");
    }

    // An unknown field, or a bad credential inside a well-formed file, is refused on load.
    let good = format!("gcr_{}", "ab".repeat(32));
    let credential = RunCredential::new(good.clone(), "2026-10-14T12:00:00Z".into()).unwrap();
    credential::store(&run_dir, &credential).unwrap();
    let file = run_dir.join("credential");
    std::fs::write(
        &file,
        format!(r#"{{"credential":"{good}","expires_at":"x","extra":1}}"#),
    )
    .unwrap();
    assert!(matches!(
        credential::load(&run_dir),
        Err(CliError::Refused(_))
    ));
    std::fs::write(&file, r#"{"credential":"gcr_short","expires_at":"x"}"#).unwrap();
    assert!(matches!(
        credential::load(&run_dir),
        Err(CliError::Refused(_))
    ));
}

#[test]
fn the_server_file_keeps_an_account_and_no_token() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        url: "https://ingest.example.org".into(),
        account: Some("ada".into()),
    };
    let path = goalcup_cli::server::write_server_config(tmp.path(), &config).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("token"), "{text}");
    assert_eq!(read_server_config(tmp.path()).unwrap().unwrap(), config);
    std::fs::write(
        &path,
        r#"{"url":"https://ingest.example.org","account":"a\u0007b"}"#,
    )
    .unwrap();
    assert!(read_server_config(tmp.path()).is_err());
}

#[test]
fn an_approval_address_that_is_not_the_servers_is_refused_unopened() {
    let device_code = "ab".repeat(32);
    for uri in [
        "http://localhost.attacker.example/device?code=BCDF-GHJK",
        "http://127.0.0.1@evil.example/device?code=BCDF-GHJK",
        "http://approve.example/device?code=BCDF-GHJK",
        "ftp://approve.example/device?code=BCDF-GHJK",
        "https://user:pw@approve.example/device?code=BCDF-GHJK",
        "https://approve.example/device?code=BCDF-GHJK#frag",
    ] {
        let body = format!(
            r#"{{"device_code":"{device_code}","user_code":"BCDF-GHJK",
                "verification_uri_complete":"{uri}","interval":1,"expires_in":5}}"#
        );
        let fake = FakeGithub::start(200, &body);
        let opened = RefCell::new(0);
        let refusal = approve(
            &ServerClient::anonymous(&fake.base),
            Purpose::Login,
            "none",
            &|_: &str| {
                *opened.borrow_mut() += 1;
                true
            },
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            matches!(refusal, CliError::Internal(_)),
            "{uri}: {refusal:?}"
        );
        assert!(!refusal.to_string().contains(&device_code), "{refusal}");
        assert_eq!(*opened.borrow(), 0, "{uri} was opened");
    }
}

#[test]
fn an_unusable_server_file_is_named_and_not_asked_for_again() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("server.json"), "not json").unwrap();
    let refusal = login::run(
        &LoginArgs { server: None },
        &cfg,
        &browser_opens,
        &mut Vec::new(),
    )
    .unwrap_err();
    let said = refusal.to_string();
    assert!(matches!(refusal, CliError::Refused(_)), "{said}");
    assert!(said.contains("server.json"), "{said}");
    assert!(said.contains("goalcup login --server <URL>"), "{said}");
    assert_eq!(
        std::fs::read_to_string(cfg.join("server.json")).unwrap(),
        "not json"
    );
}

#[test]
fn an_account_handle_that_is_blank_or_invisible_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    for handle in [
        "   ",
        "\t",
        "ada\u{200b}",
        "\u{202e}ada",
        "a\u{2066}da",
        "\u{feff}ada",
    ] {
        let doc = serde_json::json!({"url": "https://ingest.example.org", "account": handle});
        std::fs::write(tmp.path().join("server.json"), doc.to_string()).unwrap();
        assert!(
            read_server_config(tmp.path()).is_err(),
            "{handle:?} was accepted"
        );
    }
}
