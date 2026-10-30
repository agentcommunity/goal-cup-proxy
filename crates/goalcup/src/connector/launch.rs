//! The harness environment, the isolated home and the program (PRD-11 section 3).

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

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

/// The lock on a harness's isolated home, held as long as the plan lives. The launch slice takes
/// the `flock` on `$GOALCUP_HOME/homes/<id>.goalcup.lock`; until then nothing is held.
#[derive(Debug, Default)]
pub struct HomeLock {
    file: Option<File>,
}

impl HomeLock {
    /// Takes the lock for `home`, or the refusal that another run holds it. The launch slice
    /// fills this in.
    pub(crate) fn take(_home: &Path) -> Result<HomeLock, Refusal> {
        Ok(HomeLock::default())
    }

    /// Whether the lock file is held.
    pub fn is_held(&self) -> bool {
        self.file.is_some()
    }
}

/// Creates `path` and its parents with mode 0700. A failure shows when the harness starts.
pub(crate) fn make_dir(path: &Path) {
    let made = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path);
    if made.is_ok() {
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
    for name in REMOVED
        .iter()
        .copied()
        .chain(c.unset.iter().map(String::as_str))
    {
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

/// The first line of `program --version`. The launch slice fills this in.
pub(crate) fn harness_version(_program: &Path, _vars: &BTreeMap<String, String>) -> Option<String> {
    None
}

/// Writes `no_prompts` or `update_off` rows into the isolated home; a row it cannot write comes
/// back as skipped. The launch slice fills this in.
pub fn write_settings(_home: &Path, _rows: &[SettingRow]) -> Vec<Skipped> {
    Vec::new()
}
