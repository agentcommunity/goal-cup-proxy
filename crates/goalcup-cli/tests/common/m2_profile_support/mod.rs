//! Fixtures for the capture-launch tests: a fake harness that records what it was
//! launched with, the shipped connectors launching it, and readers for what it recorded.
//!
//! Everything is test-owned: the workspace, the per-run home, the harness home and the CA
//! path live in a temporary directory, and the fake writes only into its working directory.
//! Nothing here reads or writes a real harness home, and nothing sets a process variable.

#![allow(dead_code)]

use goalcup_cli::connector::HarnessConnector;
use goalcup_cli::launch::{CaptureLaunch, ResolvedExecutable, resolve_harness_executable};
use goalcup_protocol::template::TemplateVars;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The fake harness. It records, into `<cwd>/.probe/`:
///
/// * `argv` — every argument it received, each followed by one NUL byte;
/// * `env` — its whole environment, as `/usr/bin/env` prints it;
/// * `stdin-kind` — `pipe`, `tty`, `char` or `other`, and `stdin-head` — up to 16 bytes read
///   from stdin (nothing, at once, for `/dev/null`);
/// * `child-env` — the environment of a child it spawns, to show descendants inherit it.
///
/// With `FAKE_HOLD=<seconds>` in its environment it then stays alive that long, for a test
/// that needs a live process to take a handle of.
///
/// It prints and evaluates nothing it was given: arguments are only ever `printf '%s'`ed.
pub const FAKE_HARNESS: &str = r#"#!/bin/sh
out="$PWD/.probe"
mkdir -p "$out"
: > "$out/argv"
for a in "$@"; do printf '%s\0' "$a" >> "$out/argv"; done
/usr/bin/env > "$out/env"
if [ -p /dev/stdin ]; then kind=pipe
elif [ -t 0 ]; then kind=tty
elif [ -c /dev/stdin ]; then kind=char
else kind=other
fi
printf '%s' "$kind" > "$out/stdin-kind"
/bin/dd bs=1 count=16 2>/dev/null > "$out/stdin-head"
/bin/sh -c '/usr/bin/env > "$1/child-env"' child "$out"
if [ -n "$FAKE_HOLD" ]; then exec /bin/sleep "$FAKE_HOLD"; fi
exit 0
"#;

/// Write [`FAKE_HARNESS`] into `dir` as an executable, and return its absolute path.
pub fn fake_harness(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("fake-harness");
    std::fs::write(&path, FAKE_HARNESS).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::canonicalize(path).unwrap()
}

pub fn connectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// A shipped connector, loaded and validated the way `goalcup start` loads it.
pub fn shipped(id: &str) -> HarnessConnector {
    goalcup_cli::connector::load_harness(&connectors_dir(), id, false)
        .expect("the shipped connector loads")
}

/// One test's directories, all under one temporary root.
pub struct Layout {
    pub root: tempfile::TempDir,
    pub workspace: PathBuf,
    pub run_home: PathBuf,
    pub harness_home: PathBuf,
    pub ca: PathBuf,
    pub bin: PathBuf,
}

impl Layout {
    pub fn new() -> Layout {
        let root = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(root.path()).unwrap();
        let workspace = base.join("workspace");
        let run_home = base.join("run-home");
        let harness_home = base.join("harness-home");
        let bin = base.join("bin");
        for d in [&workspace, &run_home, &harness_home, &bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        let ca = base.join("proxy-ca.pem");
        std::fs::write(&ca, b"not a real certificate\n").unwrap();
        Layout {
            root,
            workspace,
            run_home,
            harness_home,
            ca,
            bin,
        }
    }
}

pub const SESSION: &str = "5f0c2a8e-8a0e-4c1f-9d6b-2b7d2b0e9a11";
pub const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// `http://goalcup:<SECRET>@127.0.0.1:<port>`, the shape `ProxyCredential::proxy_url` makes.
pub fn proxy_url() -> String {
    format!("http://goalcup:{SECRET}@127.0.0.1:47123")
}

/// The template variables a capture launch renders against.
pub fn vars(layout: &Layout) -> TemplateVars {
    let mut v = TemplateVars::new();
    v.set("workspace", layout.workspace.to_str().unwrap());
    v.set("session_id", SESSION);
    v.set("harness_home", layout.harness_home.to_str().unwrap());
    v.set("entry_id", "entry-0001");
    v.set("run_id", "entry-0001");
    v.set("artifact_root", "/nonexistent/artifact-root");
    v.set("status_line_path", "/nonexistent/status-line");
    v
}

/// The tracker environment every test starts from: identity only.
pub fn parent_env() -> BTreeMap<String, String> {
    [
        ("USER", "participant"),
        ("LOGNAME", "participant"),
        ("TMPDIR", "/private/tmp/participant-tmp/"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// `exe` resolved the way a launch resolves the program it finds on `PATH`: any build is
/// accepted (D53).
pub fn resolved(exe: &Path) -> ResolvedExecutable {
    resolve_harness_executable(exe).expect("the fake is a regular file")
}

pub fn launch<'a>(
    layout: &'a Layout,
    goal: &'a [u8],
    exe: &'a ResolvedExecutable,
    proxy: &'a str,
    parent: &'a BTreeMap<String, String>,
) -> CaptureLaunch<'a> {
    CaptureLaunch {
        goal,
        executable: exe,
        run_home: &layout.run_home,
        config_home: &layout.harness_home,
        proxy_url: proxy,
        ca_certificate: &layout.ca,
        parent_env: parent,
    }
}
