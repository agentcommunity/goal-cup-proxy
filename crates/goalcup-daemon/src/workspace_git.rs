//! The Git state of a workspace that is the root of its own clone (D56): the commit it is at and
//! one digest of what differs from that commit, so the seal of a participant's repository does
//! not walk it.
//!
//! No program the agent can configure runs here. The agent writes `.git/config`,
//! `.gitattributes` and the hooks, so every call clears the environment, reads no user or
//! system configuration, turns off the fsmonitor and the hooks, and uses only commands that
//! never convert a work-tree file: `rev-parse`, `ls-tree`, `ls-files`,
//! `hash-object --no-filters` and `cat-file --batch` (a committed link's text). `git diff` is
//! not one of them: comparing the work tree runs the repository's clean and process filters
//! (`an_agent_written_clean_filter_is_never_run`). Content is hashed here, never through a diff
//! driver.

use crate::workspace::{ABSENT, measure};
use goalcup_protocol::seal::WorkspaceGit;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A changed or untracked file larger than this is described by its size, not its content.
const HASH_LIMIT: u64 = 64 * 1024 * 1024;

/// The poll interval while a `git` call runs.
const POLL: Duration = Duration::from_millis(5);

/// `git` in `workspace` with the seal's sanitized environment: nothing inherited but `PATH`,
/// `home` as `HOME`, no system or global configuration, no fsmonitor, no hooks, no external
/// diff, no lazy fetch, and the repository pinned to `<workspace>/.git` so a missing or broken
/// one is never looked for above the workspace. Stdin is null.
pub fn command(workspace: &Path, home: &Path) -> Command {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        git.env("PATH", path);
    }
    git.env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_DIR", workspace.join(".git"))
        .env("GIT_WORK_TREE", workspace)
        .env("GIT_NO_LAZY_FETCH", "1")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(["-c", "diff.external=", "-c", "core.untrackedCache=false"])
        .current_dir(workspace)
        .stdin(Stdio::null());
    git
}

/// The commit `workspace` is at and the digest of what differs from it, or `None` when
/// `<workspace>/.git` is not a directory, a call fails, or `budget` runs out (the seal then
/// walks the workspace). The digest is SHA-256 over
///
/// `b"diff\0"` ‖ for each path, sorted bytewise, in the commit's tree or in the index whose
/// work-tree content differs from the commit: `path ‖ 0x00 ‖ value ‖ b"\n"` ‖ `b"untracked\0"`
/// ‖ for each untracked, not ignored path, sorted bytewise: `path ‖ 0x00 ‖ value ‖ b"\n"`,
///
/// where `value` is the hex SHA-256 of the content, `deleted`, or `size:<n>` above 64 MiB.
/// A clean clone digests to `sha256(b"diff\0untracked\0")`.
pub fn state(workspace: &Path, budget: Duration) -> Option<WorkspaceGit> {
    if !workspace
        .join(".git")
        .symlink_metadata()
        .is_ok_and(|m| m.is_dir())
    {
        return None;
    }
    let home = FreshHome::create()?;
    let git = Git {
        workspace,
        home: &home.0,
        deadline: Instant::now() + budget,
    };

    let (ok, head) = git.run(&["rev-parse", "--verify", "-q", "HEAD^{commit}"], None)?;
    // A failed `rev-parse` is an unborn `HEAD`; a broken repository fails the calls below.
    let commit = if ok {
        let id = String::from_utf8(head).ok()?.trim().to_string();
        let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
        if !matches!(id.len(), 40 | 64) || !id.chars().all(hex) {
            return None;
        }
        Some(id)
    } else {
        None
    };
    // The commit's files: path → (mode, object id). An unborn HEAD has none.
    let mut committed = BTreeMap::new();
    if commit.is_some() {
        for entry in git
            .ok(&["ls-tree", "-r", "-z", "--full-tree", "HEAD"], None)?
            .split(|b| *b == 0)
        {
            // `<mode> SP <type> SP <id> TAB <path>`
            let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
                continue;
            };
            let mut meta = entry[..tab].split(|b| *b == b' ');
            let (Some(mode), Some(_), Some(id)) = (meta.next(), meta.next(), meta.next()) else {
                return None;
            };
            committed.insert(entry[tab + 1..].to_vec(), (mode.to_vec(), id.to_vec()));
        }
    }
    // The index's files, `<mode> SP <id> SP <stage> TAB <path>`: a staged new file is tracked.
    let mut tracked: BTreeSet<Vec<u8>> = BTreeSet::new();
    for entry in git
        .ok(&["ls-files", "-z", "--stage"], None)?
        .split(|b| *b == 0)
    {
        if let Some(tab) = entry.iter().position(|b| *b == b'\t')
            && !entry.starts_with(b"160000 ")
        {
            tracked.insert(entry[tab + 1..].to_vec());
        }
    }
    // A submodule's content belongs to another repository.
    committed.retain(|_, (mode, _)| mode.as_slice() != b"160000");
    tracked.extend(committed.keys().cloned());

    // Each tracked path, by what the work tree holds there.
    let mut diff = BTreeMap::new();
    let mut to_hash = Vec::new();
    let mut links = Vec::new();
    for path in tracked {
        let full = workspace.join(OsStr::from_bytes(&path));
        let in_commit = committed.get(&path);
        match full.symlink_metadata() {
            Err(_) => {
                if in_commit.is_some() {
                    diff.insert(path, b"deleted".to_vec());
                }
            }
            Ok(meta) => match in_commit {
                Some((mode, id)) if meta.is_file() && mode.as_slice() != b"120000" => {
                    to_hash.push((path, id.clone()))
                }
                Some((mode, id)) if meta.is_symlink() && mode.as_slice() == b"120000" => {
                    links.push((path, id.clone()))
                }
                _ => {
                    let value = describe(&full, &meta);
                    diff.insert(path, value);
                }
            },
        }
    }
    // Regular files: the object id of the bytes as they are, never filtered.
    if !to_hash.is_empty() {
        let mut input = Vec::new();
        for (path, _) in &to_hash {
            input.extend_from_slice(&stdin_path(path));
            input.push(b'\n');
        }
        let ids = git.ok(
            &["hash-object", "--no-filters", "--stdin-paths"],
            Some(input),
        )?;
        let ids: Vec<&[u8]> = ids
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        if ids.len() != to_hash.len() {
            return None;
        }
        for ((path, committed_id), id) in to_hash.into_iter().zip(ids) {
            if id != committed_id.as_slice() {
                git.in_time()?;
                let full = workspace.join(OsStr::from_bytes(&path));
                let meta = full.symlink_metadata().ok()?;
                diff.insert(path, describe(&full, &meta));
            }
        }
    }
    // Symbolic links: the link text against the committed blob.
    if !links.is_empty() {
        let mut input = Vec::new();
        for (_, id) in &links {
            input.extend_from_slice(id);
            input.push(b'\n');
        }
        let out = git.ok(&["cat-file", "--batch"], Some(input))?;
        let mut rest = out.as_slice();
        for (path, _) in links {
            // `<id> SP blob SP <size> LF <content> LF`
            let header_end = rest.iter().position(|b| *b == b'\n')?;
            let size: usize =
                std::str::from_utf8(rest[..header_end].rsplit(|b| *b == b' ').next()?)
                    .ok()?
                    .parse()
                    .ok()?;
            let content = rest.get(header_end + 1..header_end + 1 + size)?;
            rest = rest.get(header_end + 2 + size..)?;
            let full = workspace.join(OsStr::from_bytes(&path));
            let target = std::fs::read_link(&full).ok()?;
            if target.as_os_str().as_bytes() != content {
                diff.insert(path, ABSENT.as_bytes().to_vec());
            }
        }
    }

    let mut untracked = BTreeMap::new();
    let others = git.ok(&["ls-files", "-z", "--others", "--exclude-standard"], None)?;
    for path in others.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        git.in_time()?;
        let full = workspace.join(OsStr::from_bytes(path));
        let value = match full.symlink_metadata() {
            Ok(meta) => describe(&full, &meta),
            Err(_) => ABSENT.as_bytes().to_vec(),
        };
        untracked.insert(path.to_vec(), value);
    }

    let mut digest = Sha256::new();
    digest.update(b"diff\0");
    for (path, value) in &diff {
        digest.update(path);
        digest.update([0]);
        digest.update(value);
        digest.update(b"\n");
    }
    digest.update(b"untracked\0");
    for (path, value) in &untracked {
        digest.update(path);
        digest.update([0]);
        digest.update(value);
        digest.update(b"\n");
    }
    Some(WorkspaceGit {
        commit,
        dirty_sha256: hex::encode(digest.finalize()),
    })
}

/// A file's value in the digest: `size:<n>` above [`HASH_LIMIT`], else what the seal walk
/// records for it (the absent digest for anything but a readable regular file).
fn describe(full: &Path, meta: &std::fs::Metadata) -> Vec<u8> {
    if meta.is_file() && meta.len() > HASH_LIMIT {
        return format!("size:{}", meta.len()).into_bytes();
    }
    measure(full).1.into_bytes()
}

/// A path as `hash-object --stdin-paths` reads it: as is, or C-quoted when it starts with a
/// quote or holds a line break.
fn stdin_path(path: &[u8]) -> Vec<u8> {
    if !path.starts_with(b"\"") && !path.contains(&b'\n') {
        return path.to_vec();
    }
    let mut quoted = vec![b'"'];
    for b in path {
        match b {
            b'"' => quoted.extend_from_slice(b"\\\""),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            other => quoted.push(*other),
        }
    }
    quoted.push(b'"');
    quoted
}

/// One state's calls: the workspace, the empty `HOME` and the shared deadline.
struct Git<'a> {
    workspace: &'a Path,
    home: &'a Path,
    deadline: Instant,
}

impl Git<'_> {
    fn in_time(&self) -> Option<()> {
        (Instant::now() < self.deadline).then_some(())
    }

    /// Stdout of a call that succeeded.
    fn ok(&self, args: &[&str], input: Option<Vec<u8>>) -> Option<Vec<u8>> {
        let (ok, out) = self.run(args, input)?;
        ok.then_some(out)
    }

    /// Whether the call succeeded, and its stdout. `None` when it could not start or outlived
    /// the deadline; it is then killed and reaped.
    fn run(&self, args: &[&str], input: Option<Vec<u8>>) -> Option<(bool, Vec<u8>)> {
        self.in_time()?;
        let mut cmd = command(self.workspace, self.home);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
        if input.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().ok()?;
        if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
            std::thread::spawn(move || stdin.write_all(&input));
        }
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = tx.send(stdout.read_to_end(&mut out).map(|_| out));
        });
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < self.deadline => std::thread::sleep(POLL),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        let left = self.deadline.saturating_duration_since(Instant::now());
        let out = rx.recv_timeout(left).ok()?.ok()?;
        Some((status.success(), out))
    }
}

/// A new, empty, private directory to serve as `HOME`, removed when dropped: with it, no
/// attributes or ignore file of the participant's reaches the repository.
struct FreshHome(PathBuf);

impl FreshHome {
    fn create() -> Option<FreshHome> {
        use rand::RngCore;
        use std::os::unix::fs::DirBuilderExt;
        let mut name = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut name);
        let path = std::env::temp_dir().join(format!("goalcup-git-home-{}", hex::encode(name)));
        std::fs::DirBuilder::new().mode(0o700).create(&path).ok()?;
        Some(FreshHome(path))
    }
}

impl Drop for FreshHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
