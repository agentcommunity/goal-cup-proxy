//! PRD-10 section 8 (first message, push block, clipboard) and section 9 (sleep lock).

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use goalcup::cli::{awake, push, text};

const TOKEN: &str = "ghp_SECRETVALUE0123456789";

fn block() -> Vec<String> {
    push::block(
        "https://github.com/a/b",
        "A",
        "a@x.org",
        ".env.local",
        Path::new("/p/.env.local"),
    )
}

#[test]
fn first_message_is_block_blank_line_prompt_at_0600() {
    let dir = tempfile::tempdir().unwrap();
    let prompt = dir.path().join("p.txt");
    std::fs::write(&prompt, "Build it.\nKeep going.\n").unwrap();
    let b = block();
    let got = push::write_first_message(dir.path(), Some(&b), &prompt).unwrap();
    let file = dir.path().join("first-message.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), got);
    assert_eq!(got, format!("{}\n\nBuild it.\nKeep going.\n", b.join("\n")));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // a large prompt is not refused
    std::fs::write(&prompt, "x".repeat(1 << 20)).unwrap();
    assert!(push::write_first_message(dir.path(), Some(&b), &prompt).is_ok());
}

#[test]
fn practice_message_is_the_prompt_alone() {
    let dir = tempfile::tempdir().unwrap();
    let prompt = dir.path().join("p.txt");
    std::fs::write(&prompt, "Only this.").unwrap();
    let got = push::write_first_message(dir.path(), None, &prompt).unwrap();
    assert_eq!(got, "Only this.");
    assert!(!got.contains("Push to"));
}

#[test]
fn a_prompt_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let prompt = dir.path().join("p.txt");
    std::fs::write(&prompt, [0xff, 0xfe, 0x41]).unwrap();
    let err = push::write_first_message(dir.path(), None, &prompt).unwrap_err();
    assert_eq!(err, text::prompt_not_utf8(&prompt));
    assert!(!dir.path().join("first-message.txt").exists());
}

#[test]
fn the_token_value_is_never_in_the_block_or_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env.local"), format!("GH_TOKEN={TOKEN}\n")).unwrap();
    let b = push::block(
        "https://github.com/a/b",
        "A",
        "a@x.org",
        ".env.local",
        &dir.path().join(".env.local"),
    );
    let prompt = dir.path().join("p.txt");
    std::fs::write(&prompt, "go").unwrap();
    let got = push::write_first_message(dir.path(), Some(&b), &prompt).unwrap();
    let shown = push::shown(&b, false).join("\n");
    assert!(!got.contains(TOKEN) && !shown.contains(TOKEN));
    assert!(got.contains(".env.local"));
}

#[test]
fn no_clipboard_tool_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let vars = BTreeMap::from([("PATH".to_string(), dir.path().display().to_string())]);
    assert!(!push::clipboard::copy("text", &vars));
    assert!(push::show_again(0) && !push::show_again(3));
}

#[test]
fn a_missing_inhibitor_gives_one_line_and_no_problem() {
    let dir = tempfile::tempdir().unwrap();
    let vars = BTreeMap::from([("PATH".to_string(), dir.path().display().to_string())]);
    let (lock, line) = awake::take(&vars);
    if cfg!(target_os = "macos") && Path::new("/usr/bin/caffeinate").exists() {
        return;
    }
    assert!(lock.is_none());
    let line = line.expect("a line");
    assert!(line.starts_with("This computer may sleep during the run (no sleep lock: "));
    assert!(!line.contains(text::PROBLEM) && !line.contains('\n'));
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    fn alive(pid: u32) -> bool {
        // a reaped or zombie process counts as gone
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
    }

    fn fake(dir: &Path) -> impl Fn(&str) -> Option<std::path::PathBuf> {
        let tool = dir.join("systemd-inhibit");
        std::fs::write(&tool, "#!/bin/sh\necho ready\nexec sleep 300\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        move |_| Some(tool.clone())
    }

    fn wait_gone(pid: u32) -> bool {
        for _ in 0..100 {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn dropping_the_lock_ends_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let lock = awake::hold_with(&fake(dir.path())).unwrap();
        let pid = lock.pid();
        assert!(alive(pid));
        drop(lock);
        assert!(wait_gone(pid));
    }

    /// Helper, run by `lock_ends_when_the_owner_dies` as a separate process.
    #[test]
    fn owner_helper() {
        let Ok(dir) = std::env::var("GC_TEST_FAKE_DIR") else {
            return;
        };
        let lock = awake::hold_with(&fake(Path::new(&dir))).unwrap();
        println!("PID {}", lock.pid());
        std::thread::sleep(std::time::Duration::from_secs(60));
    }

    #[test]
    fn lock_ends_when_the_owner_dies() {
        use std::io::{BufRead as _, BufReader};
        let dir = tempfile::tempdir().unwrap();
        let mut owner = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "linux::owner_helper",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("GC_TEST_FAKE_DIR", dir.path())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(owner.stdout.take().unwrap()).lines();
        let pid = lines
            .find_map(|l| l.ok()?.split_once("PID ")?.1.trim().parse::<u32>().ok())
            .expect("the owner reports the inhibitor pid");
        assert!(alive(pid));
        owner.kill().unwrap();
        let _ = owner.wait();
        assert!(wait_gone(pid), "the inhibitor outlived its owner");
    }
}
