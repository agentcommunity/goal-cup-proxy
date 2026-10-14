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
