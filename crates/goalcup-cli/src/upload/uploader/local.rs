//! What the uploader reads and notes on this machine: the entry's upload lock, the local
//! chain's records at an offset and at its end, the workspace's commit, and the notes kept
//! beside the spool.

use super::chunk::{LinkOnly, MAX_LINE_BYTES};
use super::io_error;
use crate::CliError;
use crate::upload::UploadRecord;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::export::ChainPart;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// `<cfg>/uploads/<entry>.lock`: held by whichever process uploads the entry.
pub fn lock_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.lock"))
}

/// Take the entry's upload lock, held until the returned file is dropped. Only one process
/// uploads an entry at a time: two would truncate each other's spool and race on
/// `acks.jsonl`.
pub fn lock_entry(spool_dir: &Path, entry_id: &str) -> Result<std::fs::File, CliError> {
    crate::config::create_private_dir(spool_dir)?;
    let path = lock_path(spool_dir, entry_id);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(io_error(&path))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(CliError::Refused(format!(
                "part {entry_id} is being uploaded by another goalcup: its `goalcup start` is \
                 still running, or another `goalcup upload` is. Run this again once it ends."
            )));
        }
        Err(std::fs::TryLockError::Error(source)) => return Err(io_error(&path)(source)),
    }
    Ok(file)
}

/// The `seq` and `acked_at` of the last acknowledgement in `acks`, or `None` for none.
pub fn last_ack(acks: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(acks).ok()?;
    text.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<AckCheckpoint>(line).ok())
        .and_then(|ack| Some((ack.seq, ack.acked_at_secs()?)))
}

/// The `seq` and `hash` of the chain record whose line ends exactly at the logical byte
/// `offset`, or `None` when no whole record ends there (or the part holding it is gone).
pub fn local_link_before(parts: &[ChainPart], offset: u64) -> Option<(u64, String)> {
    // The part whose last byte is at `offset - 1`; the last part may have grown since it was
    // measured, and a record that ends past what was measured is then just not found.
    let last_start = parts.last()?.start;
    let part = parts.iter().find(|part| {
        part.start < offset
            && (offset <= part.start.saturating_add(part.len) || part.start == last_start)
    })?;
    link_before_io(&part.path, offset - part.start)
        .ok()
        .flatten()
}

/// [`local_link_before`] within one part file: the record whose line ends at `offset` in it,
/// telling a file that could not be read (`Err`) from one that holds no record there
/// (`Ok(None)`).
fn link_before_io(file_path: &Path, offset: u64) -> std::io::Result<Option<(u64, String)>> {
    if offset == 0 {
        return Ok(None);
    }
    let start = offset.saturating_sub(MAX_LINE_BYTES + 1);
    let mut file = std::fs::File::open(file_path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes)?;
    let Some(body) = bytes.strip_suffix(b"\n") else {
        return Ok(None);
    };
    let line = match body.iter().rposition(|b| *b == b'\n') {
        Some(at) => &body[at + 1..],
        None if start == 0 => body,
        None => return Ok(None),
    };
    Ok(serde_json::from_slice::<LinkOnly>(line)
        .ok()
        .map(|record| (record.link.seq, record.link.hash)))
}

/// The last whole record of the chain `parts`: the logical bytes up to the end of it, its
/// `seq` and its `hash`. `None` for a chain with no whole record. A last part that holds no
/// whole record yet (just rotated) answers from the part before it.
pub fn local_end(parts: &[ChainPart]) -> Option<(u64, u64, String)> {
    local_end_io(parts).ok().flatten()
}

/// [`local_end`], telling a chain that could not be read (`Err`) from one with no whole record
/// (`Ok(None)`): the first is asked again, the second is just not there yet.
pub(super) fn local_end_io(parts: &[ChainPart]) -> std::io::Result<Option<(u64, u64, String)>> {
    for part in parts.iter().rev() {
        match end_in_io(&part.path) {
            Ok(Some((end, seq, hash))) => return Ok(Some((part.start + end, seq, hash))),
            Ok(None) => {}
            // A part deleted since it was listed holds no record any more: the one before it
            // answers, as for a part that holds none yet.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// The end of the last whole line of one part file, with the record that ends there.
fn end_in_io(file_path: &Path) -> std::io::Result<Option<(u64, u64, String)>> {
    let len = std::fs::metadata(file_path)?.len();
    let start = len.saturating_sub(2 * (MAX_LINE_BYTES + 1));
    let mut file = std::fs::File::open(file_path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take(len - start).read_to_end(&mut tail)?;
    // The end of the last whole line; a torn tail after it is not a record yet.
    let Some(newline) = tail.iter().rposition(|b| *b == b'\n') else {
        return Ok(None);
    };
    let end = start + newline as u64 + 1;
    Ok(link_before_io(file_path, end)?.map(|(seq, hash)| (end, seq, hash)))
}

/// The 40-hex commit `workspace`'s repository has checked out, read from its `.git` files (no
/// `git` process): `HEAD` naming a commit, or a branch found under `refs/heads/` or in
/// `packed-refs`. `None` when there is no repository or nothing is committed.
pub fn repo_head(workspace: &Path) -> Option<String> {
    let git = workspace.join(".git");
    // The agent controls this directory: only regular files are read (a FIFO would block), and
    // never more than a bound.
    let read = |path: PathBuf| regular_file_text(&path, 64 * 1024);
    let is_commit = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let head = read(git.join("HEAD"))?;
    let head = head.trim();
    if is_commit(head) {
        return Some(head.to_ascii_lowercase());
    }
    let reference = head.strip_prefix("ref: ")?.trim();
    // A reference names a path under `.git`; one that tries to leave it is not followed.
    if !reference.starts_with("refs/") || reference.split('/').any(|part| part == "..") {
        return None;
    }
    if let Some(commit) = read(git.join(reference)) {
        let commit = commit.trim();
        return is_commit(commit).then(|| commit.to_ascii_lowercase());
    }
    regular_file_text(&git.join("packed-refs"), 64 * 1024 * 1024)?
        .lines()
        .find_map(|line| {
            let (commit, name) = line.split_once(' ')?;
            (name.trim() == reference && is_commit(commit)).then(|| commit.to_ascii_lowercase())
        })
}

/// The text of `path` if it is a regular file (never a FIFO, device or directory, and opened
/// without blocking), at most `max` bytes, or `None`.
fn regular_file_text(path: &Path, max: u64) -> Option<String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(max + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= max).then_some(text)
}

/// `<cfg>/uploads/<entry>.sent`: written when the entry's upload is complete (the whole chain
/// and its seal; for a segment closed from its tail, the chain up to its committed end).
pub(super) fn sent_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.sent"))
}

/// Whether the entry's upload finished: nothing of it is left to close or send.
pub fn is_sent(spool_dir: &Path, entry_id: &str) -> bool {
    sent_path(spool_dir, entry_id).is_file()
}

/// `<cfg>/uploads/<entry>.repo-head`: the commit noted when the run stopped.
fn repo_head_path(spool_dir: &Path, entry_id: &str) -> PathBuf {
    spool_dir.join(format!("{entry_id}.repo-head"))
}

/// Note the workspace repository's checked-out commit now, the moment the run stops, for the
/// final heartbeat to send whenever it goes (a later `goalcup upload` included). Nothing is
/// noted when there is no repository.
pub fn note_repo_head(spool_dir: &Path, record: &UploadRecord) {
    note_repo_head_saying(spool_dir, record, &mut crate::host_log::say);
}

/// [`note_repo_head`], saying through `say` when the note cannot be written: the final
/// heartbeat then goes without the commit, and the participant is told so, not left to find the
/// commit missing from the entry.
fn note_repo_head_saying(spool_dir: &Path, record: &UploadRecord, say: &mut dyn FnMut(&str)) {
    if record.workspace.is_empty() {
        return;
    }
    if let Some(commit) = repo_head(Path::new(&record.workspace))
        && let Err(error) = crate::config::write_secret_file(
            &repo_head_path(spool_dir, &record.entry_id),
            commit.as_bytes(),
        )
    {
        say(&format!(
            "upload: the workspace's commit could not be noted ({error}); the final heartbeat \
             will go without it"
        ));
    }
}

/// [`note_repo_head`], unless a commit was already noted for the entry. A `goalcup continue`
/// notes the commit of every earlier segment it is about to send before the next segment's
/// agent can change the workspace; a host that noted the commit as its segment ended has said
/// it better than this can.
pub fn note_repo_head_unless_noted(spool_dir: &Path, record: &UploadRecord) {
    if noted_repo_head_in(spool_dir, &record.entry_id).is_none() {
        note_repo_head(spool_dir, record);
    }
}

/// The commit [`note_repo_head`] noted for the entry whose spool is `spool`, if any.
pub(super) fn noted_repo_head(spool: &Path, entry_id: &str) -> Option<String> {
    noted_repo_head_in(spool.parent()?, entry_id)
}

/// [`noted_repo_head`] given the spool directory itself.
fn noted_repo_head_in(spool_dir: &Path, entry_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(repo_head_path(spool_dir, entry_id)).ok()?;
    let text = text.trim();
    (text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::record_in;
    use super::*;

    #[test]
    fn a_commit_noted_as_a_segment_ended_is_not_replaced_by_a_later_one() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        let head = |commit: &str| {
            std::fs::write(workspace.join(".git/HEAD"), format!("{commit}\n")).unwrap();
        };
        let record = UploadRecord {
            run_id: "r0123456789abcdef".into(),
            entry_id: "e1".into(),
            server_url: String::new(),
            trace_dir: String::new(),
            official_start: String::new(),
            deadline_at: String::new(),
            workspace: workspace.display().to_string(),
            tunnels_file: None,
        };
        let (first, later) = ("a".repeat(40), "b".repeat(40));
        head(&first);
        note_repo_head_unless_noted(dir.path(), &record);
        assert_eq!(noted_repo_head_in(dir.path(), "e1"), Some(first.clone()));
        // The next segment's agent commits; the earlier segment's note stands.
        head(&later);
        note_repo_head_unless_noted(dir.path(), &record);
        assert_eq!(noted_repo_head_in(dir.path(), "e1"), Some(first));
        // An entry with no note takes the commit the workspace has.
        let other = UploadRecord {
            entry_id: "e2".into(),
            ..record
        };
        note_repo_head_unless_noted(dir.path(), &other);
        assert_eq!(noted_repo_head_in(dir.path(), "e2"), Some(later));
    }

    #[test]
    fn a_sent_entry_is_noted_and_only_that_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_sent(dir.path(), "e1"));
        std::fs::write(sent_path(dir.path(), "e1"), b"sent\n").unwrap();
        assert!(is_sent(dir.path(), "e1"));
        assert!(!is_sent(dir.path(), "e2"));
    }

    /// A commit that cannot be noted is said, so the participant is not left to find it missing.
    #[test]
    fn a_commit_that_cannot_be_noted_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        std::fs::write(workspace.join(".git/HEAD"), format!("{}\n", "a".repeat(40))).unwrap();
        let record = UploadRecord {
            workspace: workspace.display().to_string(),
            ..record_in(dir.path())
        };
        // A spool directory that is a file: nothing can be written beneath it.
        let spool = dir.path().join("uploads");
        std::fs::write(&spool, b"not a directory").unwrap();
        let mut said = Vec::new();
        note_repo_head_saying(&spool, &record, &mut |line| said.push(line.to_string()));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("could not be noted") && said[0].contains("without it"),
            "{said:?}"
        );
        assert_eq!(noted_repo_head_in(&spool, "e1"), None);

        // Written, nothing is said.
        let spool = dir.path().join("spool");
        std::fs::create_dir_all(&spool).unwrap();
        let mut said = Vec::new();
        note_repo_head_saying(&spool, &record, &mut |line| said.push(line.to_string()));
        assert!(said.is_empty(), "{said:?}");
        assert_eq!(noted_repo_head_in(&spool, "e1"), Some("a".repeat(40)));
    }
}
