//! The Git state of a workspace that is the root of its own clone (D56): the commit it is at and
//! one digest of what differs from that commit, so the seal of a participant's repository does
//! not walk it.
//!
//! No program the agent can configure runs here. The agent writes `.git/config`,
//! `.gitattributes` and the hooks, so every call runs the `git` found on `PATH` outside the
//! workspace, clears the environment, reads no user or system configuration, turns off the
//! fsmonitor and the hooks, allows no transport (so a promisor remote is never fetched from), and
//! uses only commands that never convert a work-tree file: `rev-parse`, `ls-tree`, `ls-files`,
//! `hash-object --no-filters` and `cat-file --batch` (a committed link's text). `git diff` is
//! not one of them: comparing the work tree runs the repository's clean and process filters
//! (`an_agent_written_clean_filter_is_never_run`). Content is hashed here, never through a diff
//! driver.

use crate::workspace::{ABSENT, HASH_LIMIT, measure, walk};
use goalcup_protocol::seal::WorkspaceGit;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The most bytes one `git` call may print; more is a failure, not a truncation.
const OUTPUT_LIMIT: u64 = 64 * 1024 * 1024;

/// The most files hashed below the untracked directories `ls-files` collapses (a nested
/// repository); more is a failure.
const COLLAPSED_FILE_LIMIT: usize = 100_000;

/// The poll interval while a `git` call runs.
const POLL: Duration = Duration::from_millis(5);

/// The `git` executable the seal runs: the first `git` on `PATH` under an absolute entry that is
/// not inside `workspace`, so a relative entry or one the agent writes cannot supply it.
fn resolve_git(workspace: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let canonical = std::fs::canonicalize(workspace).ok();
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        let git = dir.join("git");
        let inside = |path: &Path| {
            path.starts_with(workspace) || canonical.as_ref().is_some_and(|c| path.starts_with(c))
        };
        if !dir.is_absolute() || inside(&git) {
            return None;
        }
        let real = std::fs::canonicalize(&git).ok()?;
        let executable = real
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        (executable && !inside(&real)).then_some(git)
    })
}

/// `git` in `workspace` with the seal's sanitized environment: the [`resolve_git`] executable,
/// nothing inherited but `PATH`, `home` as `HOME`, no system or global configuration, no
/// fsmonitor, no hooks, no transport and no lazy fetch, and the repository
/// pinned to `<workspace>/.git` so a missing or broken one is never looked for above the
/// workspace. Stdin is null. `None` when no `git` is found.
pub fn command(workspace: &Path, home: &Path) -> Option<Command> {
    let mut git = Command::new(resolve_git(workspace)?);
    git.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        git.env("PATH", path);
    }
    git.env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_DIR", workspace.join(".git"))
        .env("GIT_WORK_TREE", workspace)
        // Empty: every transport is refused, whatever `protocol.*.allow` the repository sets.
        // Git before 2.44 ignores `GIT_NO_LAZY_FETCH`; this holds on every version.
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_NO_LAZY_FETCH", "1")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(["-c", "core.untrackedCache=false"])
        .current_dir(workspace)
        .stdin(Stdio::null());
    Some(git)
}

/// The commit `workspace` is at and the digest of what differs from it, or why it could not be
/// read, in a sentence that names no path: `<workspace>/.git` is not a directory, `git` is
/// missing, a call failed or printed more than 64 MiB, or `budget` ran out. The digest is SHA-256 over
///
/// `b"diff\0"` ‖ for each path, sorted bytewise, in the commit's tree or in the index whose
/// work-tree content differs from the commit: `path ‖ 0x00 ‖ value ‖ b"\n"` ‖ `b"untracked\0"`
/// ‖ for each untracked path no `.gitignore` excludes, sorted bytewise:
/// `path ‖ 0x00 ‖ value ‖ b"\n"`,
///
/// where `value` is the hex SHA-256 of the content, `deleted`, `unreadable` (it is there but its
/// metadata cannot be read), `size:<n>` above 64 MiB, for
/// anything but a readable regular file (a link, a directory, a FIFO) the walk's absent digest
/// (64 zeros), or for an index gitlink that is not the commit's, `gitlink:<id>`. A directory `ls-files` collapses
/// (a nested repository) is listed file by file. A clean clone digests to
/// `sha256(b"diff\0untracked\0")`.
pub fn state(workspace: &Path, budget: Duration) -> Result<WorkspaceGit, String> {
    if !workspace
        .join(".git")
        .symlink_metadata()
        .is_ok_and(|m| m.is_dir())
    {
        return Err("it has no .git directory".into());
    }
    let home = FreshHome::create().ok_or("no empty HOME could be made for git")?;
    let git = Git {
        workspace,
        home: &home.0,
        deadline: Instant::now() + budget,
    };

    let (ok, head, _) = git.run(&["rev-parse", "--verify", "-q", "HEAD^{commit}"], None)?;
    // A failed `rev-parse` is an unborn `HEAD`; a broken repository fails the calls below.
    let commit = if ok {
        let id = String::from_utf8(head)
            .unwrap_or_default()
            .trim()
            .to_string();
        let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
        if !matches!(id.len(), 40 | 64) || !id.chars().all(hex) {
            return Err(format!("HEAD names no commit id: {id:?}"));
        }
        Some(id)
    } else {
        None
    };
    // The commit's files: path → (mode, object id). An unborn HEAD has none.
    let mut committed = BTreeMap::new();
    if commit.is_some() {
        let tree = git.ok(&["ls-tree", "-r", "-z", "--full-tree", "HEAD"], None)?;
        for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            // `<mode> SP <type> SP <id> TAB <path>`
            let (meta, path) = split_tab(entry)?;
            let mut meta = meta.split(|b| *b == b' ');
            let (Some(mode), Some(_), Some(id)) = (meta.next(), meta.next(), meta.next()) else {
                return Err("`git ls-tree` printed an entry it should not".into());
            };
            committed.insert(path.to_vec(), (mode.to_vec(), id.to_vec()));
        }
    }
    // The index's files, `<mode> SP <id> SP <stage> TAB <path>`: a staged new file is tracked. A
    // gitlink's content belongs to another repository: one the commit does not hold is recorded
    // by its id.
    let mut tracked: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut diff = BTreeMap::new();
    for entry in git
        .ok(&["ls-files", "-z", "--stage"], None)?
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
    {
        let (meta, path) = split_tab(entry)?;
        let mut meta = meta.split(|b| *b == b' ');
        let (Some(mode), Some(id)) = (meta.next(), meta.next()) else {
            return Err("`git ls-files` printed an entry it should not".into());
        };
        if mode == b"160000" {
            let same = committed
                .get(path)
                .is_some_and(|(m, i)| m.as_slice() == mode && i.as_slice() == id);
            if !same {
                diff.insert(path.to_vec(), [b"gitlink:", id].concat());
            }
        } else {
            tracked.insert(path.to_vec());
        }
    }
    committed.retain(|_, (mode, _)| mode.as_slice() != b"160000");
    tracked.extend(committed.keys().cloned());

    // Each tracked path, by what the work tree holds there.
    let mut to_hash = Vec::new();
    let mut links = Vec::new();
    for path in tracked {
        git.in_time()?;
        let full = workspace.join(OsStr::from_bytes(&path));
        let in_commit = committed.get(&path);
        match full.symlink_metadata() {
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
                if in_commit.is_some() {
                    diff.insert(path, b"deleted".to_vec());
                }
            }
            // There, but closed to the seal (a directory above it the agent made unreadable).
            Err(_) => {
                diff.insert(path, b"unreadable".to_vec());
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
            return Err("`git hash-object` answered for a different number of files".into());
        }
        for ((path, committed_id), id) in to_hash.into_iter().zip(ids) {
            if id != committed_id.as_slice() {
                git.in_time()?;
                let full = workspace.join(OsStr::from_bytes(&path));
                let meta = full.symlink_metadata().map_err(|e| e.to_string())?;
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
        let unreadable = || "`git cat-file` printed what it should not".to_string();
        for (path, _) in links {
            // `<id> SP blob SP <size> LF <content> LF`
            let header_end = rest
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(unreadable)?;
            let size: usize = rest[..header_end]
                .rsplit(|b| *b == b' ')
                .next()
                .and_then(|s| std::str::from_utf8(s).ok())
                .and_then(|s| s.parse().ok())
                .ok_or_else(unreadable)?;
            let content = rest
                .get(header_end + 1..header_end + 1 + size)
                .ok_or_else(unreadable)?;
            rest = rest.get(header_end + 2 + size..).ok_or_else(unreadable)?;
            let full = workspace.join(OsStr::from_bytes(&path));
            let target = std::fs::read_link(&full).map_err(|e| e.to_string())?;
            if target.as_os_str().as_bytes() != content {
                diff.insert(path, ABSENT.as_bytes().to_vec());
            }
        }
    }

    // Only `.gitignore` files hide an untracked file: they are themselves in the digest, while
    // `.git/info/exclude` and `core.excludesFile` would hide work unseen.
    let mut untracked = BTreeMap::new();
    // A directory it cannot open is skipped with only a warning, and the call still succeeds:
    // any warning makes the state unreadable rather than leave that work out of the digest.
    let (ok, others, warned) = git.run(
        &[
            "ls-files",
            "-z",
            "--others",
            "--exclude-per-directory=.gitignore",
        ],
        None,
    )?;
    if !ok {
        return Err("`git ls-files` failed".into());
    }
    if warned {
        return Err("an untracked directory could not be read".into());
    }
    let mut collapsed = 0usize;
    for path in others.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        git.in_time()?;
        let full = workspace.join(OsStr::from_bytes(path));
        if path.ends_with(b"/") {
            let files =
                collapsed_files(&full, path, COLLAPSED_FILE_LIMIT - collapsed, git.deadline)?;
            collapsed += files.len();
            untracked.extend(files);
            continue;
        }
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
    Ok(WorkspaceGit {
        commit,
        dirty_sha256: hex::encode(digest.finalize()),
    })
}

/// One digest line: a path and its value.
type Line = (Vec<u8>, Vec<u8>);

/// A directory `ls-files` does not enter (a nested repository): every file below `full`, as
/// `path ‖ rel` and its value, at most `left` of them, read before `deadline`.
fn collapsed_files(
    full: &Path,
    path: &[u8],
    left: usize,
    deadline: Instant,
) -> Result<Vec<Line>, String> {
    // The walk's own error names the directory; the reason recorded must not.
    let files = walk(full, &[], left + 1, HASH_LIMIT, Some(deadline)).map_err(|e| match e {
        crate::DaemonError::Io { source, .. } => {
            format!("an untracked nested repository could not be read: {source}")
        }
        _ => "the time to read it ran out".to_string(),
    })?;
    if files.len() > left {
        return Err(format!(
            "more than {COLLAPSED_FILE_LIMIT} files below untracked directories"
        ));
    }
    Ok(files
        .into_iter()
        .map(|file| {
            let value = if file.bytes > HASH_LIMIT {
                format!("size:{}", file.bytes)
            } else {
                file.sha256
            };
            ([path, file.path.as_bytes()].concat(), value.into_bytes())
        })
        .collect())
}

/// `meta TAB path` of one `ls-tree` or `ls-files --stage` entry.
fn split_tab(entry: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let tab = entry
        .iter()
        .position(|b| *b == b'\t')
        .ok_or("git printed an entry without a path")?;
    Ok((&entry[..tab], &entry[tab + 1..]))
}

/// A file's value in the digest: `size:<n>` above [`HASH_LIMIT`], else what the seal walk
/// records for it (the absent digest for anything but a readable regular file).
fn describe(full: &Path, meta: &std::fs::Metadata) -> Vec<u8> {
    if meta.is_file() && meta.len() > HASH_LIMIT {
        return format!("size:{}", meta.len()).into_bytes();
    }
    measure(full, u64::MAX).1.into_bytes()
}

/// A path as `hash-object --stdin-paths` reads it: as is, or C-quoted when it starts with a
/// quote or holds a line break or a carriage return (which a line read would strip).
fn stdin_path(path: &[u8]) -> Vec<u8> {
    if !path.starts_with(b"\"") && !path.contains(&b'\n') && !path.contains(&b'\r') {
        return path.to_vec();
    }
    let mut quoted = vec![b'"'];
    for b in path {
        match b {
            b'"' => quoted.extend_from_slice(b"\\\""),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
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
    fn in_time(&self) -> Result<(), String> {
        if Instant::now() < self.deadline {
            Ok(())
        } else {
            Err("the time to read it ran out".into())
        }
    }

    /// Stdout of a call that succeeded.
    fn ok(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
        let (ok, out, _) = self.run(args, input)?;
        if ok {
            Ok(out)
        } else {
            Err(format!("`git {}` failed", args[0]))
        }
    }

    /// Whether the call succeeded, its stdout, and whether it wrote anything to stderr (drained,
    /// never kept). An error when it could not start, printed more than [`OUTPUT_LIMIT`], or
    /// outlived the deadline; it is then killed and reaped.
    fn run(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<(bool, Vec<u8>, bool), String> {
        self.in_time()?;
        let mut cmd =
            command(self.workspace, self.home).ok_or("no git was found on PATH outside it")?;
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        if input.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("git could not start: {e}"))?;
        if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
            std::thread::spawn(move || stdin.write_all(&input));
        }
        let stderr = child.stderr.take().ok_or("git has no stderr")?;
        let (err_tx, err_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = err_tx.send(std::io::copy(&mut { stderr }, &mut std::io::sink()));
        });
        let stdout = child.stdout.take().ok_or("git has no stdout")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let read = stdout.take(OUTPUT_LIMIT + 1).read_to_end(&mut out);
            let _ = tx.send(read.map(|_| out));
        });
        let failed = |child: &mut std::process::Child, why: String| {
            let _ = child.kill();
            let _ = child.wait();
            Err(why)
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < self.deadline => std::thread::sleep(POLL),
                Ok(None) => return failed(&mut child, "the time to read it ran out".into()),
                Err(e) => return failed(&mut child, e.to_string()),
            }
        };
        let left = self.deadline.saturating_duration_since(Instant::now());
        let out = rx
            .recv_timeout(left)
            .map_err(|_| "the time to read it ran out".to_string())?
            .map_err(|e| e.to_string())?;
        if out.len() as u64 > OUTPUT_LIMIT {
            return Err(format!("`git {}` printed more than 64 MiB", args[0]));
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        let warned = err_rx
            .recv_timeout(left)
            .map_err(|_| "the time to read it ran out".to_string())?
            .map_err(|e| e.to_string())?
            > 0;
        Ok((status.success(), out, warned))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Review fix 2: the walk below a nested repository keeps the state's deadline. Past it, the
    /// state is not read, as for any other timeout.
    #[test]
    fn a_nested_walk_past_its_deadline_is_not_read() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("work.rs"), b"one").unwrap();
        let soon = Instant::now() + Duration::from_secs(60);
        let read = collapsed_files(t.path(), b"nested/", 10, soon).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].0, b"nested/work.rs");
        let past = Instant::now();
        std::thread::sleep(Duration::from_millis(2));
        assert!(collapsed_files(t.path(), b"nested/", 10, past).is_err());
    }
}
