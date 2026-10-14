//! `goalcup prep`'s GitHub half: the repository check, the clone with its credential helper,
//! and the helper that answers git for that one repository.
//!
//! The GitHub API is a local fake on loopback and the clone source is a local bare
//! repository; `git` is a wrapper script that logs its argv and runs the real one against a
//! gitconfig of the test's own. Nothing here reads `$HOME`, `~/.gitconfig` or a real token.

use goalcup_cli::CliError;
use goalcup_cli::account::github::{GithubSetup, check, clone_from, credential_helper};
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
    /// `Authorization` header.
    fn start(status: u16, body: &str) -> FakeGithub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, halt, body) = (requests.clone(), stop.clone(), body.to_string());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let head = read_head(&mut stream);
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
                log.lock().unwrap().push((path, auth));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.shutdown(Shutdown::Both);
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

impl Drop for FakeGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
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

fn refusal(result: Result<(), CliError>) -> String {
    match result {
        Err(CliError::Refused(sentence)) => sentence,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn private_non_empty_or_read_only_repos_are_refused_with_distinct_sentences() {
    let ok = r#"{"private":false,"size":0,"permissions":{"admin":false,"push":true,"pull":true}}"#;
    let cases = [
        (
            200,
            r#"{"private":true,"size":0,"permissions":{"push":true}}"#,
        ),
        (
            200,
            r#"{"private":false,"size":12,"permissions":{"push":true}}"#,
        ),
        (
            200,
            r#"{"private":false,"size":0,"permissions":{"push":false}}"#,
        ),
        (200, r#"{"private":false,"size":0}"#),
        (404, r#"{"message":"Not Found"}"#),
        (401, r#"{"message":"Bad credentials"}"#),
        (
            301,
            r#"{"message":"Moved Permanently","url":"https://api.github.com/repositories/1"}"#,
        ),
    ];
    let mut sentences = Vec::new();
    for (status, body) in cases {
        let fake = FakeGithub::start(status, body);
        sentences.push(refusal(check(&fake.base, &setup())));
        let seen = fake.requests.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].0, "/repos/acme/empty-repo");
        assert_eq!(
            seen[0].1.as_deref(),
            Some(format!("Bearer {}", token()).as_str())
        );
    }
    // Private, non-empty, read-only, not found, refused token, moved: six different sentences, and
    // a missing `permissions` object reads as read-only.
    assert!(sentences[0].contains("private"), "{}", sentences[0]);
    assert!(sentences[1].contains("not empty"), "{}", sentences[1]);
    assert!(sentences[2].contains("cannot push"), "{}", sentences[2]);
    assert_eq!(sentences[2], sentences[3]);
    assert!(sentences[4].contains("not found"), "{}", sentences[4]);
    assert!(
        sentences[5].contains("refused the token"),
        "{}",
        sentences[5]
    );
    let mut distinct = vec![
        &sentences[0],
        &sentences[1],
        &sentences[2],
        &sentences[4],
        &sentences[5],
    ];
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "{sentences:#?}");
    for sentence in &sentences {
        assert!(
            !sentence.contains(&token()),
            "the token was repeated: {sentence}"
        );
    }

    // A public, empty, pushable repository passes.
    let fake = FakeGithub::start(200, ok);
    check(&fake.base, &setup()).unwrap();
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
    assert!(sentence.contains("goalcup prep"), "{sentence}");
}

// ------------------------------------------------------------------ the clone

struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    git: PathBuf,
    log: PathBuf,
    gitconfig: PathBuf,
    source: String,
}

/// A `git` wrapper that logs its argv and runs the real git against a gitconfig of its own,
/// and an empty bare repository to clone from.
fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
    assert!(real.starts_with('/'), "no git on PATH: {real:?}");
    let log = root.join("git-argv.log");
    let gitconfig = root.join("gitconfig");
    std::fs::write(&gitconfig, "").unwrap();
    let git = root.join("git");
    std::fs::write(
        &git,
        format!(
            "#!/bin/sh\n\
             for a in \"$@\"; do printf '[%s]' \"$a\" >> \"{log}\"; done\n\
             printf '\\n' >> \"{log}\"\n\
             GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=\"{cfg}\" exec \"{real}\" \"$@\"\n",
            log = log.display(),
            cfg = gitconfig.display(),
        ),
    )
    .unwrap();
    make_executable(&git);
    let bare = root.join("source.git");
    let init = Command::new(&git)
        .args(["init", "--bare", "--quiet"])
        .arg(&bare)
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    std::fs::write(&log, "").unwrap();
    Rig {
        _tmp: tmp,
        root,
        git,
        log,
        gitconfig,
        source: bare.display().to_string(),
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

fn all_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            all_files(&path, found);
        } else {
            found.push(path);
        }
    }
}

#[test]
fn the_clone_has_a_helper_and_no_token_in_its_config() {
    let rig = rig();
    let dest = rig.root.join("work");
    let cfg = rig.root.join("it's my cfg");
    // A goalcup that answers the way the real helper does, to see git ask it.
    let exe = rig.root.join("goalcup");
    std::fs::write(
        &exe,
        "#!/bin/sh\ncat >/dev/null\necho username=x-access-token\n\
         echo \"password=ours:$GOALCUP_CONFIG_DIR\"\n",
    )
    .unwrap();
    make_executable(&exe);

    clone_from(&rig.git, &rig.source, &setup(), &dest, &exe, &cfg).unwrap();

    // The clone looks like production's: origin is the public URL, with no user info.
    assert_eq!(
        git_out(&rig, &dest, &["remote", "get-url", "origin"]).trim(),
        REPO_URL
    );
    let helpers = git_out(
        &rig,
        &dest,
        &[
            "config",
            "--local",
            "--get-all",
            "credential.https://github.com.helper",
        ],
    );
    let ours = format!(
        "!GOALCUP_CONFIG_DIR='{}/it'\\''s my cfg' '{}' git-credential",
        rig.root.display(),
        exe.display()
    );
    assert_eq!(helpers.lines().collect::<Vec<_>>(), vec!["", ours.as_str()]);
    assert_eq!(
        git_out(
            &rig,
            &dest,
            &["config", "--local", "credential.useHttpPath"]
        )
        .trim(),
        "true"
    );

    // (a) The token is in no file of the clone.
    let mut files = Vec::new();
    all_files(&dest.join(".git"), &mut files);
    assert!(files.iter().any(|f| f.ends_with("config")));
    for file in files {
        let bytes = std::fs::read(&file).unwrap();
        let needle = token();
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "the token is in {}",
            file.display()
        );
    }

    // (b) Only our helper answers: a global helper that would leak is reset for this clone.
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
    // The configuration directory, with a space and a quote in its name, arrives intact.
    assert!(
        said.contains(&format!("password=ours:{}\n", cfg.display())),
        "{said}"
    );
    assert!(!said.contains("leaked"), "{said}");

    // A relative goalcup is refused: the agent's git runs the helper from anywhere.
    let other = rig.root.join("other");
    let relative = clone_from(
        &rig.git,
        &rig.source,
        &setup(),
        &other,
        Path::new("goalcup"),
        &cfg,
    );
    assert!(
        matches!(relative, Err(CliError::Refused(_))),
        "{relative:?}"
    );
    assert!(!other.exists());
    let relative_cfg = clone_from(
        &rig.git,
        &rig.source,
        &setup(),
        &other,
        &exe,
        Path::new("cfg"),
    );
    assert!(
        matches!(relative_cfg, Err(CliError::Refused(_))),
        "{relative_cfg:?}"
    );
    assert!(!other.exists());
}

#[test]
fn the_token_never_appears_in_a_git_argv() {
    let rig = rig();
    let dest = rig.root.join("work");
    clone_from(
        &rig.git,
        &rig.source,
        &setup(),
        &dest,
        &rig.root.join("goalcup"),
        &rig.root.join("cfg"),
    )
    .unwrap();
    let argv = std::fs::read_to_string(&rig.log).unwrap();
    assert!(argv.contains("[clone]"), "{argv}");
    assert!(argv.contains("[--add]"), "{argv}");
    assert!(argv.contains("[credential.useHttpPath][true]"), "{argv}");
    for secret in [
        token(),
        "x-access-token@".to_string(),
        "password=".to_string(),
    ] {
        assert!(
            !argv.contains(&secret),
            "{secret} is in a git argv:\n{argv}"
        );
    }
}

// ------------------------------------------------------------------ the stored form

#[test]
fn the_setup_is_stored_for_the_owner_alone_and_checked_on_the_way_in_and_out() {
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
    assert_eq!(setup().owner_repo(), ("acme", "empty-repo"));
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

    // The token is read as one bounded line.
    let s = GithubSetup::read_token(
        REPO_URL,
        &mut format!("  {}  \nsecond line\n", token()).as_bytes(),
    )
    .unwrap();
    assert!(
        !format!("{s:?}").contains(&token()),
        "Debug prints the token"
    );
    let flood = "A".repeat(100_000);
    assert!(GithubSetup::read_token(REPO_URL, &mut flood.as_bytes()).is_err());

    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let missing = GithubSetup::load(&cfg).unwrap_err().to_string();
    assert!(missing.contains("goalcup prep"), "{missing}");
    let path = s.store(&cfg).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "github.json is mode {:o}", mode & 0o777);
        let back = GithubSetup::load(&cfg).unwrap();
        assert_eq!(back.repo_url, REPO_URL);
        assert_eq!(back.owner_repo(), ("acme", "empty-repo"));

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
use goalcup_cli::account::device::{Approved, Purpose, approve};
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

fn no_browser(_: &str) -> bool {
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
        &no_browser,
        &mut out,
    )
    .unwrap();

    let stored = read_server_config(&cfg).unwrap().unwrap();
    assert_eq!(stored.account.as_deref(), Some("member-1"));
    assert_eq!(stored.url, server.url());
    assert!(stored.token.is_empty());
    let printed = String::from_utf8(out).unwrap();
    assert!(printed.contains("Logged in as member-1"), "{printed}");

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
        Purpose::Run { practice: true },
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

    let asked = server.device_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].purpose, "run");
    assert_eq!(asked[0].mode, "practice");
    assert_eq!(asked[0].connector_id, "codex-cli");
    assert_eq!(asked[0].tracer_version, goalcup_cli::CLI_VERSION);
}

#[test]
fn a_browser_that_will_not_open_is_asked_for_by_hand() {
    let tmp = tempfile::tempdir().unwrap();
    let server = stand_in(tmp.path());
    let mut out = Vec::new();
    approve(
        &ServerClient::anonymous(&server.url()),
        Purpose::Run { practice: false },
        "codex-cli",
        &|_: &str| false,
        &mut out,
    )
    .unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert!(printed.contains("yourself"), "{printed}");
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
        &no_browser,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    assert!(refusal.to_string().contains("start again"), "{refusal}");
    assert!(files_under(&cfg).is_empty());
    assert!(!cfg.join("server.json").exists());
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
        &no_browser,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(refusal, CliError::Refused(_)), "{refusal:?}");
    assert!(refusal.to_string().contains("denied"), "{refusal}");
    assert!(files_under(&cfg).is_empty());
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
        &no_browser,
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
        &no_browser,
        &mut Vec::new(),
    )
    .unwrap();
    // No `--server` the second time: the stored one is used.
    login::run(
        &LoginArgs { server: None },
        &cfg,
        &no_browser,
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
        Purpose::Run { practice: true },
        "codex-cli",
        &no_browser,
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
        token: String::new(),
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
