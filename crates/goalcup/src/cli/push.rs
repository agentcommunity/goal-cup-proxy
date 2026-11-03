//! The push block and the first message (PRD-10 section 8). The block is 0.7's `prompt_block`
//! unchanged: the token stays in `.env.local` and the credential helper reads it at each push.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use super::text;

/// The repository, the git identity and the credential helper, as the lines the participant's
/// first message starts with. `token_path` is where the helper reads the token (the file's bare
/// name when the path holds a quote, `$`, `\` or a backquote, which the line cannot carry).
pub fn block(
    repo_url: &str,
    name: &str,
    email: &str,
    token_file: &str,
    token_path: &Path,
) -> Vec<String> {
    let path = token_path.display().to_string();
    let path = if path.contains(['\'', '"', '$', '\\', '`']) {
        token_file.to_string()
    } else {
        path
    };
    vec![
        format!("Push to {repo_url}"),
        format!(
            "The token is GH_TOKEN in {token_file} in this folder. Never print it, and never write \
             it into a file, a git setting or a remote address."
        ),
        "After git init, run:".to_string(),
        format!("  git config user.name \"{name}\""),
        format!("  git config user.email \"{email}\""),
        format!(
            "  git config credential.https://github.com.helper '!f() {{ echo \
             username=x-access-token; echo \"password=$(sed -n s/^GH_TOKEN=//p \"{path}\")\"; }}; f'"
        ),
        format!("  git remote add origin {repo_url}.git"),
    ]
}

/// Read `prompt_file` (UTF-8, any size). `start` calls this with the folder checks, before
/// anything is created.
pub fn read_prompt(prompt_file: &Path) -> Result<String, String> {
    let bytes = std::fs::read(prompt_file)
        .map_err(|e| text::prompt_unreadable(prompt_file, &e.to_string()))?;
    String::from_utf8(bytes).map_err(|_| text::prompt_not_utf8(prompt_file))
}

/// The first message: the block, a blank line, the prompt (the prompt alone without a block).
/// Written to `dir/first-message.txt` at 0600; returns the text, for the clipboard.
pub fn write_first_message(
    dir: &Path,
    block: Option<&[String]>,
    prompt: &str,
) -> Result<String, String> {
    let message = match block {
        Some(lines) => format!("{}\n\n{prompt}", lines.join("\n")),
        None => prompt.to_string(),
    };
    let file = dir.join("first-message.txt");
    let fail = |_: std::io::Error| text::not_written(&file);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&file)
        .map_err(fail)?;
    out.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(fail)?;
    out.write_all(message.as_bytes()).map_err(fail)?;
    Ok(message)
}

pub mod clipboard {
    use super::BTreeMap;
    use crate::connector::launch::resolve_program;
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    const WITHIN: Duration = Duration::from_secs(5);
    /// What a clipboard tool is given of the participant's environment, so no key goes with it.
    const KEEP: &[&str] = &[
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

    fn tool(vars: &BTreeMap<String, String>) -> Option<(std::path::PathBuf, Vec<&'static str>)> {
        let found = |name: &str| Some(resolve_program(name, vars)).filter(|p| p.is_absolute());
        let set = |v: &str| vars.get(v).is_some_and(|x| !x.is_empty());
        if cfg!(target_os = "macos") {
            return found("pbcopy").map(|p| (p, vec![]));
        }
        if super::super::folder::wsl_version().is_some()
            && let Some(p) = found("clip.exe")
        {
            return Some((p, vec![]));
        }
        if set("WAYLAND_DISPLAY")
            && let Some(p) = found("wl-copy")
        {
            return Some((p, vec![]));
        }
        if set("DISPLAY")
            && let Some(p) = found("xclip")
        {
            return Some((p, vec!["-selection", "clipboard"]));
        }
        None
    }

    /// Copy `text` with the machine's own tool; true only when it took the text and ended well.
    /// No tool is normal: nothing is said and nothing fails.
    pub fn copy(text: &str, vars: &BTreeMap<String, String>) -> bool {
        let Some((program, args)) = tool(vars) else {
            return false;
        };
        let Ok(mut child) = std::process::Command::new(&program)
            .args(args)
            .env_clear()
            .envs(vars.iter().filter(|(k, _)| KEEP.contains(&k.as_str())))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
        let until = Instant::now() + WITHIN;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return wrote && status.success(),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}
