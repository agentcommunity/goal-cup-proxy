//! The push lines of a run with a repository (D73): the lines the participant puts at the start
//! of their prompt ([`crate::participant::prompt_block`]), kept where they cannot be lost.
//!
//! `goalcup start` writes them to `<cfg>/runs/<run_id>/push-lines.txt` (mode 0600) and says the
//! path, copies them to the clipboard when a clipboard tool is there, and `goalcup status` and
//! every `goalcup continue` that opens a new conversation show them again. They name the token
//! file and never hold the token.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The file in the run's directory.
pub const FILE_NAME: &str = "push-lines.txt";

/// The most of the file this build reads: the lines are a few hundred bytes.
const MAX_BYTES: u64 = 64 * 1024;

/// How long a clipboard tool may take before the copy is called failed.
const CLIPBOARD_WITHIN: Duration = Duration::from_secs(3);

/// `<cfg>/runs/<run_id>/push-lines.txt`.
pub fn path(cfg: &Path, run_id: &str) -> PathBuf {
    crate::run::run_dir(cfg, run_id).join(FILE_NAME)
}

/// The lines as one text, one per line, as the participant pastes them.
pub fn text(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Write the lines to `path`, private to the participant.
pub fn write(path: &Path, lines: &[String]) -> Result<(), crate::CliError> {
    crate::config::write_secret_file(path, text(lines).as_bytes())
}

/// The lines kept at `path`; `None` when there is no such file or it cannot be read.
pub fn read(path: &Path) -> Option<Vec<String>> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_BYTES {
        return None;
    }
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    (!lines.is_empty()).then_some(lines)
}

/// The repository URL the lines push to (their first line, `Push to <url>`).
pub fn repository(lines: &[String]) -> Option<&str> {
    lines.first()?.strip_prefix("Push to ").map(str::trim)
}

/// Copy `text` to the clipboard with the machine's own tool, found on `env`'s `PATH`: `pbcopy`
/// on macOS; `clip.exe` under WSL; `wl-copy` with a Wayland display, else `xclip -selection
/// clipboard` with an X display, on Linux. `true` only when the tool took the text and ended
/// well. Nothing is said and nothing fails when there is none.
pub fn copy_to_clipboard(text: &str, env: &BTreeMap<String, String>) -> bool {
    let wsl = cfg!(target_os = "linux") && goalcup_platform::machine::is_wsl();
    let Some((program, args)) = clipboard_tool(env, cfg!(target_os = "macos"), wsl) else {
        return false;
    };
    run_clipboard_tool(&program, &args, text, env)
}

/// The clipboard tool and its arguments for this machine, if one is on `PATH`.
fn clipboard_tool(
    env: &BTreeMap<String, String>,
    macos: bool,
    wsl: bool,
) -> Option<(PathBuf, Vec<String>)> {
    let found = |name: &str| crate::preflight::resolve_on_path(name, env);
    let set = |var: &str| env.get(var).is_some_and(|v| !v.is_empty());
    if macos {
        return found("pbcopy").map(|p| (p, Vec::new()));
    }
    if wsl && let Some(p) = found("clip.exe") {
        return Some((p, Vec::new()));
    }
    if set("WAYLAND_DISPLAY")
        && let Some(p) = found("wl-copy")
    {
        return Some((p, Vec::new()));
    }
    if set("DISPLAY")
        && let Some(p) = found("xclip")
    {
        return Some((p, vec!["-selection".into(), "clipboard".into()]));
    }
    None
}

/// What a clipboard tool is given of the participant's environment: where it is, its display,
/// its locale and the WSL bridge to Windows. Nothing else, so no key of theirs goes with it.
const CLIPBOARD_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "DISPLAY",
    "XAUTHORITY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "WSL_DISTRO_NAME",
    "WSL_INTEROP",
];

/// Run the tool with `text` on its stdin and nothing on its stdout or stderr (`xclip` and
/// `wl-copy` stay in the background to serve the selection, and must not hold the terminal's
/// output open). Ended within [`CLIPBOARD_WITHIN`], or killed and called failed.
fn run_clipboard_tool(
    program: &Path,
    args: &[String],
    text: &str,
    env: &BTreeMap<String, String>,
) -> bool {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(
            env.iter()
                .filter(|(name, _)| CLIPBOARD_ENV.contains(&name.as_str())),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let until = Instant::now() + CLIPBOARD_WITHIN;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return written && status.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_come_back_as_written_and_name_their_repository() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runs/r1").join(FILE_NAME);
        let lines = crate::participant::prompt_block(
            "https://github.com/someone/cup",
            "someone",
            "1+someone@users.noreply.github.com",
            ".env.local",
            &dir.path().join(".env.local"),
        );
        write(&file, &lines).unwrap();
        assert_eq!(read(&file).as_deref(), Some(lines.as_slice()));
        assert_eq!(repository(&lines), Some("https://github.com/someone/cup"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// A clipboard tool found on `PATH` is given the text; with none, the copy is `false` and
    /// nothing fails.
    #[cfg(unix)]
    #[test]
    fn the_copy_goes_to_the_tool_on_path_and_says_false_without_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let got = dir.path().join("got");
        let tool = dir.path().join(if cfg!(target_os = "macos") {
            "pbcopy"
        } else {
            "xclip"
        });
        std::fs::write(&tool, format!("#!/bin/sh\ncat > '{}'\n", got.display())).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = BTreeMap::new();
        env.insert(
            "PATH".to_string(),
            format!("{}:/bin:/usr/bin", dir.path().display()),
        );
        env.insert("DISPLAY".to_string(), ":0".to_string());
        if goalcup_platform::machine::is_wsl() {
            return;
        }
        assert!(copy_to_clipboard("Push to x\n", &env));
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "Push to x\n");

        let mut none = BTreeMap::new();
        none.insert(
            "PATH".to_string(),
            dir.path().join("empty").display().to_string(),
        );
        assert!(!copy_to_clipboard("Push to x\n", &none));
    }
}
