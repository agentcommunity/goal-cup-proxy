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
