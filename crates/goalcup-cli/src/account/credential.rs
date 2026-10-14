//! The credential the upload server mints when the participant approves a run, and where the
//! tracker keeps it.
//!
//! A [`RunCredential`] (`gcr_` and 64 lowercase hex digits) is bound to one run on the server.
//! It lives in one file, `<run_dir>/credential`, at mode 0600 in a directory of mode 0700, and
//! is handed out in exactly one place: [`crate::upload::http::ServerClient::with_credential`],
//! as the bearer of calls to the upload server.
//!
//! It is never in a harness's environment, a command line or a URL (nothing reads this file
//! into any of them); never printed or logged (`Debug` prints `<redacted>`, the type has no
//! `Display` and no `Serialize`, and no message in this module formats the value); and never in
//! a trace (the daemon never reads this file).
//!
//! Mode 0600 keeps other users out, not the agent: the harness runs as the participant and can
//! read any file they can. What bounds the damage is the credential itself: it uploads one run's
//! trace to the server and does nothing else, it ends 24 hours after the run's deadline, and
//! the server stores only its SHA-256 (docs/CHEAT-CHECKLIST.md).

use crate::CliError;
use crate::config::{create_private_dir, write_secret_file};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `<run_dir>/credential`.
pub const CREDENTIAL_FILE_NAME: &str = "credential";

/// The most of the file [`load`] reads. The document is a 68-character credential and a time.
const CREDENTIAL_FILE_MAX_BYTES: u64 = 16 * 1024;

const PREFIX: &str = "gcr_";

/// A run's credential for the upload server, and when the server says it ends.
#[derive(Clone, PartialEq, Eq)]
pub struct RunCredential {
    secret: String,
    /// When the server ends the credential, as it sent it (RFC 3339, UTC). Not a secret.
    pub expires_at: String,
}

impl RunCredential {
    /// `secret` as the server minted it: `gcr_` and exactly 64 lowercase hex digits. A refusal
    /// never repeats the value.
    pub fn new(secret: String, expires_at: String) -> Result<RunCredential, CliError> {
        let hex = secret.strip_prefix(PREFIX).unwrap_or("");
        let well_formed = hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !well_formed {
            return Err(CliError::Refused(
                "the run credential is not `gcr_` and 64 lowercase hex digits, so it is not a \
                 Goal Cup run credential"
                    .into(),
            ));
        }
        if expires_at.chars().any(|c| c.is_control()) || expires_at.len() > 64 {
            return Err(CliError::Refused(
                "the run credential's end time is not a short printable text".into(),
            ));
        }
        Ok(RunCredential { secret, expires_at })
    }

    /// The bearer value. Only the HTTP client reads it.
    pub(crate) fn secret(&self) -> &str {
        &self.secret
    }
}

impl std::fmt::Debug for RunCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunCredential")
            .field("secret", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// What the file holds. Private, so the public type has no `Serialize`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCredential {
    credential: String,
    expires_at: String,
}

/// `<run_dir>/credential`.
pub fn credential_path(run_dir: &Path) -> PathBuf {
    run_dir.join(CREDENTIAL_FILE_NAME)
}

/// Store `c` at `<run_dir>/credential`, mode 0600, replacing any earlier one. `run_dir` is the
/// tracker's own directory: it is created at mode 0700, and an existing one is set to 0700.
pub fn store(run_dir: &Path, c: &RunCredential) -> Result<(), CliError> {
    create_private_dir(run_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(run_dir, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| CliError::Io {
                path: run_dir.display().to_string(),
                source,
            },
        )?;
    }
    let stored = StoredCredential {
        credential: c.secret.clone(),
        expires_at: c.expires_at.clone(),
    };
    write_secret_file(
        &credential_path(run_dir),
        &serde_json::to_vec_pretty(&stored)?,
    )
}

/// The credential stored in `run_dir`.
///
/// The file must be the owner's alone: one the group or others can read is refused, naming the
/// fix. Its content is checked again as [`RunCredential::new`] checks a fresh one.
pub fn load(run_dir: &Path) -> Result<RunCredential, CliError> {
    use std::io::Read;

    let path = credential_path(run_dir);
    let io = |source: std::io::Error| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(CliError::Refused(format!(
                "this run has no saved credential on this machine ({})",
                path.display()
            )));
        }
        Err(source) => return Err(io(source)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file.metadata().map_err(io)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(CliError::Refused(format!(
                "{} can be read by other users (mode {:o}), so the credential in it is no \
                 longer yours alone. Run `chmod 600 {}`.",
                path.display(),
                mode & 0o777,
                path.display()
            )));
        }
    }
    let mut text = String::new();
    let read = file
        .take(CREDENTIAL_FILE_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if read as u64 > CREDENTIAL_FILE_MAX_BYTES {
        return Err(CliError::Refused(format!(
            "{} is larger than {CREDENTIAL_FILE_MAX_BYTES} bytes, so it is not a credential \
             this tracker saved",
            path.display()
        )));
    }
    let stored: StoredCredential = serde_json::from_str(&text).map_err(|_| {
        CliError::Refused(format!(
            "{} is not a document this tracker saved",
            path.display()
        ))
    })?;
    RunCredential::new(stored.credential, stored.expires_at).map_err(|_| {
        CliError::Refused(format!(
            "{} does not hold a Goal Cup run credential",
            path.display()
        ))
    })
}
