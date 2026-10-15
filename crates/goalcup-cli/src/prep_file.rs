//! `<cfg>/prep.json`: what `goalcup prep` settled for this machine, and what `goalcup start`
//! reads before it asks for anything.
//!
//! The file holds the harness the participant picked, the address of their GitHub repository and
//! the folder their runs are cloned into. None of it is a secret (the GitHub token is in
//! `github.json`, `account::github`). `goalcup prep` writes it; `goalcup start` for a run on the
//! server reads it with [`read_prep`] and refuses, before it asks for approval or creates
//! anything, when it is missing or incomplete.

use crate::CliError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `<cfg>/prep.json`.
pub const PREP_FILE_NAME: &str = "prep.json";

/// The most of the file [`read_prep`] reads. The document is three short strings.
const PREP_FILE_MAX_BYTES: u64 = 16 * 1024;

/// The sentence for a machine that has not been prepared. One sentence, so a script and a
/// participant read the same words.
pub const PREP_FIRST: &str = "run `goalcup prep` first";

/// What `goalcup prep` settled. A field a hand-edited file leaves out reads as empty, and
/// [`read_prep`] refuses an empty `repo_url` or `work_root`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepFile {
    /// The connector id of the harness picked.
    #[serde(default)]
    pub harness: String,
    /// `https://github.com/<owner>/<repo>`, the participant's empty public repository.
    #[serde(default)]
    pub repo_url: String,
    /// The folder each run's clone is made in, an absolute path.
    #[serde(default)]
    pub work_root: String,
    /// The participant has opened the harness once in its tracker home, signed in and added
    /// what they want there (`goalcup prep`, step 3). The tracker cannot see a vendor sign-in,
    /// so this is the person's word, recorded when the harness they were given exited 0. It is
    /// for `harness` alone: another harness starts again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home_ready: bool,
    /// The repository at `repo_url` was found public, empty and pushable with the stored
    /// token. Kept because a practice run pushes to it, and a repository that is no longer
    /// empty would fail the same check on the next `goalcup prep`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub github_ok: bool,
}

/// `<cfg>/prep.json`.
pub fn prep_path(cfg: &Path) -> PathBuf {
    cfg.join(PREP_FILE_NAME)
}

/// The prep file, or the refusal [`PREP_FIRST`] when there is none, it cannot be read as one,
/// or it names no repository or no work folder.
pub fn read_prep(cfg: &Path) -> Result<PrepFile, CliError> {
    use std::io::Read;
    let refused = || CliError::Refused(PREP_FIRST.to_string());
    let file = std::fs::File::open(prep_path(cfg)).map_err(|_| refused())?;
    let mut text = Vec::new();
    file.take(PREP_FILE_MAX_BYTES + 1)
        .read_to_end(&mut text)
        .map_err(|_| refused())?;
    if text.len() as u64 > PREP_FILE_MAX_BYTES {
        return Err(refused());
    }
    let prep: PrepFile = serde_json::from_slice(&text).map_err(|_| refused())?;
    if prep.repo_url.trim().is_empty() || prep.work_root.trim().is_empty() {
        return Err(refused());
    }
    Ok(prep)
}

/// The prep file as it is, or the empty one when there is none or it cannot be read as one:
/// `goalcup prep` starts from whatever the last prep settled, and a damaged file settles nothing.
pub fn read_prep_lenient(cfg: &Path) -> PrepFile {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(prep_path(cfg)) else {
        return PrepFile::default();
    };
    let mut text = Vec::new();
    if file
        .take(PREP_FILE_MAX_BYTES + 1)
        .read_to_end(&mut text)
        .is_err()
        || text.len() as u64 > PREP_FILE_MAX_BYTES
    {
        return PrepFile::default();
    }
    serde_json::from_slice(&text).unwrap_or_default()
}

/// Write `prep` to `<cfg>/prep.json` (mode 0600, replacing the earlier file whole).
pub fn write_prep(cfg: &Path, prep: &PrepFile) -> Result<(), CliError> {
    let mut text = serde_json::to_vec_pretty(prep)?;
    text.push(b'\n');
    crate::config::write_secret_file(&prep_path(cfg), &text)
}
