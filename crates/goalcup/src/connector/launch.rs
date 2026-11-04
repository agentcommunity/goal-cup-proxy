//! The harness environment, the isolated home and the program (PRD-11 section 3).

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::hints::{self, yaml_key};
use super::{Connector, Env, Refusal, SettingRow, Skipped};

/// Removed from the participant's environment before anything is set (step 1).
pub(crate) const REMOVED: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
];

/// Loopback stays off the proxy (step 3).
const NO_PROXY: &str = "127.0.0.1,localhost,::1";

/// The `flock` on `<home>.goalcup.lock` (no harness uses the name, P12), held while the plan lives.
#[derive(Debug, Default)]
pub struct HomeLock {
    file: Option<File>,
}

impl HomeLock {
    /// The lock, or the refusal that another run holds it (HF4). Any other failure goes unlocked.
    pub(crate) fn take(home: &Path) -> Result<HomeLock, Refusal> {
        let mut path = home.as_os_str().to_owned();
        path.push(".goalcup.lock");
        make_dir(home.parent().unwrap_or(home));
        let Ok(file) = opts().create(true).open(&path) else {
            return Ok(HomeLock::default());
        };
        // `flock(LOCK_EX | LOCK_NB)` on Linux.
        match file.try_lock() {
            Ok(()) => Ok(HomeLock { file: Some(file) }),
            Err(TryLockError::WouldBlock) => Err(Refusal::HomeLocked { home: home.into() }),
            Err(_) => Ok(HomeLock::default()),
        }
    }

    /// Whether the lock file is held.
    pub fn is_held(&self) -> bool {
        self.file.is_some()
    }
}

/// Read and write, mode 0600 when created, never through a link.
fn opts() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).mode(0o600);
    options.custom_flags(libc::O_NOFOLLOW);
    options
}

/// Creates `path` and its parents with mode 0700. A failure shows when the harness starts.
pub(crate) fn make_dir(path: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    if builder.recursive(true).mode(0o700).create(path).is_ok() {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
}

/// `s` with `{home}` and `{workspace}` filled.
pub(crate) fn fill(s: &str, home: &Path, workspace: &Path) -> String {
    s.replace("{home}", &home.to_string_lossy())
        .replace("{workspace}", &workspace.to_string_lossy())
}

/// The harness environment before the port and the CA paths: steps 1, 3, 4 and 6. `plan` creates
/// the `home` folders.
pub(crate) fn build_env(c: &Connector, env: &Env, home: &Path) -> BTreeMap<String, String> {
    let mut vars = env.vars.clone();
    let unset = c.unset.iter().map(String::as_str);
    for name in REMOVED.iter().copied().chain(unset) {
        vars.remove(name);
    }
    vars.insert("NO_PROXY".into(), NO_PROXY.into());
    vars.insert("no_proxy".into(), NO_PROXY.into());
    vars.insert("NODE_USE_ENV_PROXY".into(), "1".into());
    for (name, template) in &c.home {
        vars.insert(name.clone(), fill(template, home, &env.workspace));
    }
    for (name, value) in &c.env {
        vars.insert(name.clone(), value.clone());
    }
    vars
}

/// Steps 2 and 5: the proxy and the CA files. Never `HTTP_PROXY`; never `SSL_CERT_DIR`,
/// `CURL_CA_BUNDLE` or `NODE_TLS_REJECT_UNAUTHORIZED`, so a value the participant exported passes
/// through untouched.
pub(crate) fn finish_env(
    vars: &mut BTreeMap<String, String>,
    ca_env: &[String],
    port: u16,
    run_ca: &Path,
    bundle: &Path,
) {
    let proxy = format!("http://127.0.0.1:{port}");
    let run_ca = run_ca.to_string_lossy().into_owned();
    let bundle = bundle.to_string_lossy().into_owned();
    vars.insert("HTTPS_PROXY".into(), proxy.clone());
    vars.insert("https_proxy".into(), proxy);
    vars.insert("NODE_EXTRA_CA_CERTS".into(), run_ca.clone());
    vars.insert("SSL_CERT_FILE".into(), bundle.clone());
    vars.insert("REQUESTS_CA_BUNDLE".into(), bundle);
    for name in ca_env {
        vars.insert(name.clone(), run_ca.clone());
    }
}

/// `program` on the participant's `PATH`, else the bare name.
pub(crate) fn resolve_program(program: &str, vars: &BTreeMap<String, String>) -> PathBuf {
    if program.contains('/') {
        return PathBuf::from(program);
    }
    let path = vars.get("PATH").map_or("", String::as_str);
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .unwrap_or_else(|| PathBuf::from(program))
}

/// The first non-empty line of `program --version` run with exactly `vars`, from stdout, else from
/// stderr. A stderr line, or any line on a failed exit, counts only if its first word is the
/// program's file name (any case) and it holds a version. `None` on a spawn failure, 5 s without an exit, or no such line. Never logged.
pub(crate) fn harness_version(program: &Path, vars: &BTreeMap<String, String>) -> Option<String> {
    let mut command = Command::new(program);
    command.arg("--version").env_clear().envs(vars);
    command.process_group(0).stdin(Stdio::null());
    command.stderr(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let Ok(Some(status)) = child.try_wait() else {
        // SAFETY: plain integers; the unreaped pid still names its group, which is ended.
        unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
        let _ = child.wait();
        return None;
    };
    fn first_line(mut pipe: impl Read + AsRawFd) -> Option<String> {
        // SAFETY: `fcntl` on a descriptor `pipe` owns. Non-blocking: a left-behind child cannot
        // hang us.
        unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty());
        line.map(str::to_owned)
    }
    let out = first_line(child.stdout.take()?);
    // A stderr line or a failed exit counts only with a version in it: never record a stray error.
    let checked = out.is_none() || !status.success();
    let line = out.or_else(|| first_line(child.stderr.take()?))?;
    // The first word must be the program's file name (any case): "hermes: needs Python 3.11.4"
    // and "warning: node 2.0.1" name some other version.
    let name = program.file_name().and_then(|n| n.to_str()).unwrap_or("/");
    let named = line
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(name));
    let held = named && super::version::parse(&line).is_some();
    (!checked || held).then_some(line)
}

/// The most of a version line goalcup keeps, in UTF-16 units: the server's limit, so a long
/// `--version` line never fills the start line.
pub const VERSION_UNITS: usize = 64;

/// `v` cut to `VERSION_UNITS` UTF-16 units at a char boundary.
pub fn cut_version(v: &str) -> &str {
    let mut units = 0;
    for (i, c) in v.char_indices() {
        units += c.len_utf16();
        if units > VERSION_UNITS {
            return &v[..i];
        }
    }
    v
}

/// Writes `no_prompts` or `update_off` rows into the home; a row not written comes back skipped.
pub fn write_settings(home: &Path, rows: &[SettingRow]) -> Vec<Skipped> {
    let mut skipped = Vec::new();
    for row in rows {
        let file = PathBuf::from(fill(&row.file, home, Path::new("")));
        if let Err(why) = write_row(&file, &row.key, &row.value) {
            let (reason, local) = (format!("{}: {}: {why}", file.display(), row.key), false);
            skipped.push(Skipped { reason, local });
        }
    }
    skipped
}

/// Reads `file` without following a link, sets `key`, writes beside it (0600) and renames over.
fn write_row(file: &Path, key: &str, value: &str) -> Result<(), &'static str> {
    fn bad<E>(_: E) -> &'static str {
        "unparsable"
    }
    let mut text = String::new();
    match opts().open(file) {
        Ok(mut f) if f.metadata().is_ok_and(|m| m.is_file()) => {
            f.read_to_string(&mut text).map_err(|_| "unreadable")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("a link, not a file, or unreadable"),
    }
    let text = text.trim_start_matches('\u{feff}');
    let leaf = value.parse().map_or(Value::from(value), Value::Bool);
    let parts: Vec<&str> = key.split('.').collect();
    let ext = file.extension().and_then(|x| x.to_str()).unwrap_or("");
    let out = if matches!(ext, "yaml" | "yml") {
        let out = yaml_set(text, &parts, &format!(" {leaf}"))?;
        if out == text {
            // Already set as written: the file is not rewritten.
            return Ok(());
        }
        out
    } else {
        let doc = match ext {
            // Comments and trailing commas read as `hints::read` reads them.
            "json" if text.trim().is_empty() => Ok(Value::Object(Default::default())),
            "json" => serde_json::from_str(&hints::strip_jsonc(text)),
            "toml" => serde_json::to_value(text.parse::<toml::Table>().map_err(bad)?),
            _ => return Err("format not writable"),
        };
        let Some(doc) = set(doc.map_err(bad)?, &parts, leaf)? else {
            return Ok(());
        };
        match ext {
            "json" => serde_json::to_string_pretty(&doc).map_err(bad)? + "\n",
            _ => {
                let doc: toml::Table = serde_json::from_value(doc).map_err(bad)?;
                toml::to_string(&doc).map_err(bad)?
            }
        }
    };
    replace(file, out.as_bytes()).map_err(|_| "cannot write it")
}

/// Writes `bytes` beside `file` (0600, never through a link) and renames it over `file`.
fn replace(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = file.parent().unwrap_or(Path::new("."));
    make_dir(parent);
    let tmp = parent.join(format!(".{}.goalcup-tmp", std::process::id()));
    let written = opts().create(true).truncate(true).open(&tmp);
    let written = written.and_then(|mut f| f.write_all(bytes));
    let done = written.and_then(|()| std::fs::rename(&tmp, file));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// The first line of every `.profile` goalcup writes.
const PROFILE_HEADER: &str = "# Written by goalcup: the PATH handed to the harness.";

/// `<dir>/.profile` putting the handed `path` back, which a login shell's `/etc/profile` resets
/// (PRD-17 P11). Only a `.profile` goalcup wrote is replaced; a link or any other file is kept,
/// which returns `AlreadyExists` so the participant is told. Any other open error is returned.
pub(crate) fn write_profile(dir: &Path, path: &str) -> std::io::Result<()> {
    let file = dir.join(".profile");
    let kept = || Err(std::io::ErrorKind::AlreadyExists.into());
    match opts().open(&file) {
        Ok(f) if f.metadata().is_ok_and(|m| m.is_file()) => {
            let mut head = String::new();
            let _ = f
                .take(PROFILE_HEADER.len() as u64 + 1)
                .read_to_string(&mut head);
            if head.strip_suffix('\n') != Some(PROFILE_HEADER) {
                return kept();
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) if std::fs::symlink_metadata(&file).is_ok_and(|m| m.is_file()) => return Err(e),
        _ => return kept(),
    }
    let quoted = path.replace('\'', r"'\''");
    let body = format!("{PROFILE_HEADER}\nexport PATH='{quoted}'\n");
    replace(&file, body.as_bytes())
}

/// `doc` with the dotted path `parts` set to `leaf`, every other key kept; `None` when it holds
/// `leaf` already, so the file is not rewritten (a TOML file keeps its comments).
fn set(mut doc: Value, parts: &[&str], leaf: Value) -> Result<Option<Value>, &'static str> {
    let mut at = &mut doc;
    for part in parts {
        let map = at.as_object_mut().ok_or("a key part is not a table")?;
        at = map.entry(*part).or_insert(serde_json::json!({}));
    }
    let same = *at == leaf;
    *at = leaf;
    Ok((!same).then_some(doc))
}

/// Sets `parts` in block YAML: replaces the key's line, else adds the rest under its parent. A
/// number part is an existing item of a block list (`- ` at its key's indent or deeper).
fn yaml_set(text: &str, parts: &[&str], leaf: &str) -> Result<String, &'static str> {
    let indent = |line: &str| {
        let body = line.trim_start_matches(' ');
        (!body.is_empty() && !body.starts_with('#')).then_some(line.len() - body.len())
    };
    let dash = |line: &str| {
        matches!(
            line.trim_start_matches(' ').as_bytes(),
            [b'-'] | [b'-', b' ', ..]
        )
    };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    // Each list item entered has its `-` blanked while its keys are looked up, then put back.
    let mut dashes: Vec<(usize, usize)> = Vec::new();
    let (mut from, mut end, mut pad) = (0, lines.len(), 0);
    for (n, part) in parts.iter().enumerate() {
        let is_item = |i: &usize| indent(&lines[*i]) == Some(pad) && dash(&lines[*i]);
        if let Ok(k) = part.parse::<usize>() {
            let i = (from..end)
                .filter(is_item)
                .nth(k)
                .ok_or("no such list item")?;
            let next = (i + 1..end).find(|&j| indent(&lines[j]).is_some_and(|d| d <= pad));
            let next = next.unwrap_or(end);
            // Blank and comment lines after the item belong to what follows it.
            let next = (i + 1..next).rev().find(|&j| indent(&lines[j]).is_some());
            let next = next.map_or(i + 1, |j| j + 1);
            lines[i].replace_range(pad..=pad, " ");
            dashes.push((i, pad));
            let child = (i..next).find_map(|j| indent(&lines[j]));
            (from, end, pad) = (i, next, child.unwrap_or(pad + 2));
            continue;
        }
        let entry = |line: &str| {
            let (_, _, rest) = yaml_key(line).filter(|(_, k, _)| k == *part)?;
            Some(rest.trim().to_owned())
        };
        let at_pad = |i: &usize| indent(&lines[*i]) == Some(pad) && entry(&lines[*i]).is_some();
        let Some(i) = (from..end).find(at_pad) else {
            for (m, p) in parts[n..].iter().enumerate() {
                let value = if n + m + 1 == parts.len() { leaf } else { "" };
                lines.insert(end + m, format!("{}{p}:{value}", " ".repeat(pad + 2 * m)));
            }
            break;
        };
        let ends = |j: &usize| {
            indent(&lines[*j]).is_some_and(|d| d < pad || d == pad && !dash(&lines[*j]))
        };
        let block = (i + 1..end).find(ends);
        let block = block.unwrap_or(end);
        // Blank and comment lines after the block belong to what follows it.
        let block = (i + 1..block).rev().find(|&j| indent(&lines[j]).is_some());
        let block = block.map_or(i + 1, |j| j + 1);
        if n + 1 == parts.len() {
            lines.splice(i..block, [format!("{}{part}:{leaf}", " ".repeat(pad))]);
            break;
        }
        if entry(&lines[i]).is_some_and(|v| !v.is_empty() && !v.starts_with('#')) {
            return Err("a key part is not a block mapping");
        }
        let child = (i + 1..block).find_map(|j| indent(&lines[j]));
        let list = (i + 1..block).any(|j| indent(&lines[j]) == child && dash(&lines[j]));
        if list && parts[n + 1].parse::<usize>().is_err() {
            return Err("a key part is not a block mapping");
        }
        (from, end, pad) = (i + 1, block, child.unwrap_or(pad + 2));
    }
    for (i, at) in dashes {
        lines[i].replace_range(at..=at, "-");
    }
    Ok(lines.join("\n") + "\n")
}
