use crate::{DAEMON_VERSION, DaemonError};
use goalcup_protocol::control::CONTROL_API_VERSION;
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};

/// The name of the file the CLI reads to find the daemon, under the daemon's `config_dir`.
pub const CONTROL_FILE_NAME: &str = "control.json";

// ------------------------------------------------------------------ control.json

/// `<config_dir>/control.json`: where the CLI looks for the daemon.
///
/// **No `socket` field.** There is no Unix transport, so a client has nothing to look for; a
/// field that was always `null` would be a field a client writes a branch for.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlFile {
    /// What the daemon speaks. A client with a different one refuses the daemon rather
    /// than mis-reading its JSON.
    pub control_api_version: u32,
    pub daemon_version: String,
    /// `127.0.0.1:<port>`, ready to be pasted into a URL.
    pub address: String,
    pub port: u16,
    /// The CLI's bearer. Per daemon, not per run, and not D24's receiver token: that one
    /// goes into the workspace where the agent can read it, and this one never does.
    pub token: String,
}

/// Every field as a derive would print it, except `token`, which prints `<redacted>`.
/// This struct is the only place the CLI's bearer lives in memory outside `ControlState`,
/// and a `{:?}` that carried it would hand a reader the credential every authenticated
/// route trusts.
impl std::fmt::Debug for ControlFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlFile")
            .field("control_api_version", &self.control_api_version)
            .field("daemon_version", &self.daemon_version)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// A fresh control bearer: 32 bytes from the operating system's entropy source, hex.
///
/// Separate from [`write_control_file`] because the token has to exist before the router
/// does and the file cannot be written until the listener has bound — the port is the OS's
/// to choose when the configured one is `0`. Minting here and writing there is what keeps
/// `control.json` from ever naming a port nothing is listening on.
pub fn mint_control_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Remove the one temp file a daemon killed mid-write leaves behind, once, at startup.
///
/// [`write_private`]'s temp name carries `std::process::id()`, and its `O_CREAT|O_EXCL`
/// create refuses a temp file that is already there rather than joining it. Both are
/// deliberate — that refusal is what keeps two concurrent writers from publishing each
/// other's bytes — but together they have one bad case: a daemon killed between the create
/// and the rename leaves `control.json.<pid>.tmp` on disk, nothing else ever removes it, and
/// the next daemon the operating system hands that pid is refused at startup by leavings a
/// week old, told that "another writer holds it" about a writer that is long dead.
///
/// So the sweep is here and not inside `write_private`: it runs once, before the daemon's
/// single `write_control_file`, and it removes exactly the one name this process would
/// choose. Every later create still meets the `O_EXCL` refusal with its whole meaning
/// intact.
///
/// Best effort, and silent. A temp file that cannot be removed — a directory planted at the
/// name, an unwritable `config_dir` — is the write's refusal to report, in a message that
/// names the file, rather than a second refusal here saying the same thing first.
pub fn sweep_stale_control_temp(config_dir: &FsPath) {
    crate::fsutil::sweep_process_temp(&config_dir.join(CONTROL_FILE_NAME));
}

/// Write `<config_dir>/control.json` at mode 0600 and answer with the path written.
///
/// The write is [`write_private`]: a fresh temp file at 0600 and a rename, never an open
/// of the target. That is what makes the mode unconditional — a `control.json` an earlier
/// daemon left behind with looser bits is replaced rather than reopened — and what makes a
/// planted symlink a refusal rather than a bearer handed to somebody else's file.
pub fn write_control_file(
    config_dir: &std::path::Path,
    port: u16,
    token: &str,
) -> Result<PathBuf, DaemonError> {
    let path = config_dir.join(CONTROL_FILE_NAME);
    let file = ControlFile {
        control_api_version: CONTROL_API_VERSION,
        daemon_version: DAEMON_VERSION.to_string(),
        address: format!("127.0.0.1:{port}"),
        port,
        token: token.to_string(),
    };
    let text = serde_json::to_string_pretty(&file)?;
    write_private(&path, text.as_bytes())?;
    Ok(path)
}

/// `<config_dir>/control.json` as the running daemon wrote it.
///
/// The read half of [`write_control_file`], and the answer to "where is the daemon" for a
/// process that must not open the registry: `Registry::open` takes a file lock the running
/// daemon holds, so `goalcup-daemon --print-addresses` and `goalcup doctor` would otherwise
/// be refused at exactly the moment they are worth running. Everything they need is in this
/// file, put there after the listener bound.
///
/// A missing file is an ordinary `DaemonError::Io` naming the path, because "there is no
/// daemon" is the answer a doctor came for and the path is where it would have been.
///
/// **What comes back holds the CLI's bearer.** It is the credential every authenticated
/// route trusts; [`ControlFile`]'s `Debug` prints `<redacted>` and no caller in this tree
/// prints the field itself.
pub fn read_control_file(config_dir: &FsPath) -> Result<ControlFile, DaemonError> {
    let path = config_dir.join(CONTROL_FILE_NAME);
    let text = std::fs::read_to_string(&path).map_err(|source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// Put `bytes` at `path` at mode 0600, without ever opening `path` itself.
///
/// The mechanics — a fresh `O_EXCL` temp file at the mode, `fsync`, `rename` — are
/// [`crate::fsutil::write_atomic`]'s and are documented there. What this wrapper decides is
/// the two things `control.json` has an opinion about, and both are about the bearer it
/// carries.
///
/// **It does not follow a link.** Opening `path` with `O_TRUNC` would follow a symlink: a
/// `control.json` planted as a link — and `config_dir` is a directory the participant can
/// reach — would take the CLI's bearer and write it through the link into a
/// file of the planter's choosing, at whatever mode that file already had. So the target is
/// stat'd with `symlink_metadata` (which does not follow) and anything that is not a regular
/// file is refused with nothing written: not a link, not a directory, not a fifo. A caller
/// that meant to redirect this file redirects `config_dir`. [`crate::fsutil::write_atomic`]
/// makes that check for every write.
///
/// **The temp name is per-process**, not per-write, because it is a name a startup sweep
/// has to be able to find again: see [`sweep_stale_control_temp`]. One writer per process
/// writes this file, once.
///
/// The mode needs setting only once: the file that ends up at `path` is always one this
/// call created, so the mode on the create is the whole of it and there is no earlier
/// daemon's permission bits to correct.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), DaemonError> {
    crate::fsutil::write_atomic(path, &crate::fsutil::process_temp(path), bytes, 0o600)
}
