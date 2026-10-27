//! The SSH-key warning of `goalcup setup` and `goalcup doctor` (D73, D71's correction).
//!
//! The agent runs as the participant, and OpenSSH reads `~/.ssh` from the user database, not
//! from `HOME`, so a key there with no passphrase lets the agent push to any repository the
//! participant can. This looks, on this machine only: no network, no SSH connection, and no
//! signing by an SSH agent (so no 1Password prompt). A private key file in `~/.ssh` that
//! `ssh-keygen -y -P "" -f <key>` opens has no passphrase. An SSH agent that lists keys
//! (`ssh-add -l`) is a softer note. A warning only, never a refusal; with no `ssh-keygen` there
//! is nothing to say.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A test build's `~/.ssh` stand-in: the folder whose `.ssh` is looked in. A test build that does
/// not name one looks nowhere, so a test never reads the machine's own keys.
#[cfg(feature = "test-support")]
pub const TEST_HOME_VAR: &str = "GOALCUP_TEST_SSH_HOME";

/// How long one `ssh-keygen` or `ssh-add` may take.
const TOOL_WITHIN: Duration = Duration::from_secs(5);

/// The most files in `~/.ssh` this looks at, and the largest it reads.
const MAX_FILES: usize = 64;
const MAX_KEY_BYTES: u64 = 64 * 1024;

/// What was found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Findings {
    /// Private key files that open with an empty passphrase.
    pub bare_keys: Vec<PathBuf>,
    /// The SSH agent lists at least one key.
    pub agent_has_keys: bool,
}

impl Findings {
    /// The `[warn]` sentence for the keys with no passphrase; `None` when there is none.
    pub fn key_warning(&self) -> Option<String> {
        let keys: Vec<String> = self
            .bare_keys
            .iter()
            .map(|k| k.display().to_string())
            .collect();
        match keys.as_slice() {
            [] => None,
            [one] => Some(format!(
                "Your SSH key {one} has no passphrase. During a run your agent could use it to \
                 push to any of your GitHub repositories. Add a passphrase (ssh-keygen -p -f \
                 {one}), use an SSH agent that asks before each use, or run in Docker."
            )),
            many => Some(format!(
                "Your SSH keys {} have no passphrase. During a run your agent could use them to \
                 push to any of your GitHub repositories. Add a passphrase (ssh-keygen -p -f \
                 <key>, for each), use an SSH agent that asks before each use, or run in Docker.",
                many.join(", ")
            )),
        }
    }

    /// The softer note for keys an SSH agent holds; `None` when it holds none.
    pub fn agent_note(&self) -> Option<&'static str> {
        self.agent_has_keys.then_some(
            "Your SSH agent holds keys. During a run your agent could use them to push, unless \
             your SSH agent asks before each use.",
        )
    }
}

/// Look in this account's `~/.ssh` (from the user database) and at the SSH agent `env` names.
/// `None` when `ssh-keygen` is not on `env`'s `PATH`, or there is no home to look in.
pub fn look(env: &BTreeMap<String, String>) -> Option<Findings> {
    #[cfg(feature = "test-support")]
    let home = env.get(TEST_HOME_VAR).map(PathBuf::from);
    #[cfg(not(feature = "test-support"))]
    let home = goalcup_platform::machine::account_home();
    look_in(&home?.join(".ssh"), env)
}

/// [`look`] with the `.ssh` folder given.
pub fn look_in(ssh_dir: &Path, env: &BTreeMap<String, String>) -> Option<Findings> {
    let keygen = crate::preflight::resolve_on_path("ssh-keygen", env)?;
    let mut findings = Findings::default();
    for key in private_key_files(ssh_dir) {
        if opens_without_passphrase(&keygen, &key, env) {
            findings.bare_keys.push(key);
        }
    }
    findings.agent_has_keys = agent_lists_keys(env);
    Some(findings)
}

/// Regular files in `dir` (not `*.pub`) whose first line is a PEM or OpenSSH private key header.
fn private_key_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut keys: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_none_or(|e| e != "pub"))
        .take(MAX_FILES)
        .filter(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_KEY_BYTES)
                && first_line(p).is_some_and(|line| {
                    line.starts_with("-----BEGIN ") && line.contains("PRIVATE KEY-----")
                })
        })
        .collect();
    keys.sort();
    keys
}

fn first_line(path: &Path) -> Option<String> {
    let mut head = [0u8; 80];
    let mut file = std::fs::File::open(path).ok()?;
    let n = file.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]);
    text.lines().next().map(str::to_string)
}

/// `ssh-keygen -y -P "" -f <key>` prints the public key: the file opens with no passphrase. A
/// security-key (`sk-`) key needs a touch on its device to sign, so it is not counted.
fn opens_without_passphrase(keygen: &Path, key: &Path, env: &BTreeMap<String, String>) -> bool {
    let args = [
        "-y".as_ref(),
        "-P".as_ref(),
        "".as_ref(),
        "-f".as_ref(),
        key.as_os_str(),
    ];
    match run(keygen, &args, env, &["PATH"]) {
        Some((true, stdout)) => !stdout.trim_start().starts_with("sk-"),
        _ => false,
    }
}

/// `ssh-add -l` lists keys: exit 0. Only the agent `SSH_AUTH_SOCK` names is asked, and only for
/// its list, never to sign.
fn agent_lists_keys(env: &BTreeMap<String, String>) -> bool {
    if env.get("SSH_AUTH_SOCK").is_none_or(String::is_empty) {
        return false;
    }
    let Some(ssh_add) = crate::preflight::resolve_on_path("ssh-add", env) else {
        return false;
    };
    matches!(
        run(&ssh_add, &["-l".as_ref()], env, &["PATH", "SSH_AUTH_SOCK"]),
        Some((true, _))
    )
}

/// Run `program` with `args` and only the `keep` variables of `env`, no input, within
/// [`TOOL_WITHIN`]: whether it ended well, and what it printed.
fn run(
    program: &Path,
    args: &[&std::ffi::OsStr],
    env: &BTreeMap<String, String>,
    keep: &[&str],
) -> Option<(bool, String)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.iter().filter(|(name, _)| keep.contains(&name.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stdout).take(MAX_KEY_BYTES).read_to_string(&mut text);
        text
    });
    let until = Instant::now() + TOOL_WITHIN;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    Some((status.success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key made with no passphrase is named; the same key with one is not. Nothing is asked
    /// of an SSH agent when no `SSH_AUTH_SOCK` is set.
    #[test]
    fn a_key_with_no_passphrase_is_named_and_one_with_a_passphrase_is_not() {
        let mut env = BTreeMap::new();
        env.insert(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        );
        if crate::preflight::resolve_on_path("ssh-keygen", &env).is_none() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        let key = ssh.join("id_ed25519");
        let made = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
            .arg(&key)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        std::fs::write(ssh.join("config"), "Host *\n").unwrap();

        let found = look_in(&ssh, &env).unwrap();
        assert_eq!(found.bare_keys, vec![key.clone()]);
        assert!(!found.agent_has_keys);
        let warning = found.key_warning().unwrap();
        assert!(warning.contains(&key.display().to_string()), "{warning}");
        assert!(warning.contains("ssh-keygen -p -f"), "{warning}");
        assert!(!warning.contains('\u{2014}'), "{warning}");

        let set = Command::new("ssh-keygen")
            .args(["-q", "-p", "-P", "", "-N", "a passphrase", "-f"])
            .arg(&key)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(set.success());
        let found = look_in(&ssh, &env).unwrap();
        assert!(found.bare_keys.is_empty(), "{found:?}");
        assert_eq!(found.key_warning(), None);
    }
}
