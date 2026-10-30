//! Homes for tests. Every test gives a child a temporary `HOME` and `GOALCUP_HOME` ([`TempHome`]),
//! and [`guard`] fails a test that changed the real `~/.claude`, `~/.codex` or `~/.hermes`
//! (AGENTS.md hard rule). Neither helper writes to, or reads a file in, the real home: the guard
//! looks at metadata only (type, mode, size, mtime, link target).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

/// A temporary `HOME`, with `GOALCUP_HOME` inside it. Removed when dropped.
pub struct TempHome {
    dir: tempfile::TempDir,
}

impl TempHome {
    pub fn new() -> TempHome {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        std::fs::create_dir_all(dir.path().join(".goalcup")).expect("create GOALCUP_HOME");
        TempHome { dir }
    }

    /// The temporary `HOME`.
    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    /// The temporary `GOALCUP_HOME`.
    pub fn goalcup_home(&self) -> PathBuf {
        self.dir.path().join(".goalcup")
    }

    /// Points `cmd` at the temporary homes, and drops every variable that could send a harness
    /// back to a real home.
    pub fn apply<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        for var in [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "HERMES_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("HOME", self.home())
            .env("GOALCUP_HOME", self.goalcup_home())
    }
}

impl Default for TempHome {
    fn default() -> Self {
        TempHome::new()
    }
}

/// The owner's desktop apps rewrite these homes all the time, so only their app-wide config and
/// credential files are watched there; they are never walked. `~/.claude.json` is left out:
/// Claude Code rewrites it every few minutes.
const WATCHED_FILES: [&str; 3] = [
    ".codex/config.toml",
    ".codex/auth.json",
    ".claude/settings.json",
];

/// Homes no app of the owner's writes: every name under them, this deep, is watched.
const WALKED: [&str; 1] = [".hermes"];
const WALK_DEPTH: usize = 4;

/// Fails the test, when dropped, if the real home's harness directories changed meanwhile.
/// Hold it for the test's life: `let _guard = common::home::guard();`.
pub fn guard() -> RealHomeGuard {
    RealHomeGuard {
        before: real_listing(),
    }
}

pub struct RealHomeGuard {
    before: String,
}

impl Drop for RealHomeGuard {
    fn drop(&mut self) {
        // A test that already failed keeps its own message.
        if std::thread::panicking() {
            return;
        }
        assert_eq!(
            real_listing(),
            self.before,
            "the real HOME's ~/.claude, ~/.codex or ~/.hermes changed while the test ran"
        );
    }
}

fn real_listing() -> String {
    match std::env::var_os("HOME") {
        Some(home) => listing(Path::new(&home)),
        None => "no HOME".to_string(),
    }
}

/// One line per watched path.
pub fn listing(home: &Path) -> String {
    let mut out = Vec::new();
    for rel in WATCHED_FILES {
        node(&home.join(rel), Path::new(rel), &mut out);
    }
    for rel in WALKED {
        tree(&home.join(rel), Path::new(rel), &mut out, WALK_DEPTH);
    }
    out.join("\n")
}

fn tree(path: &Path, rel: &Path, out: &mut Vec<String>, depth: usize) {
    if !node(path, rel, out) || depth == 0 {
        return;
    }
    let mut names = match std::fs::read_dir(path) {
        Ok(entries) => entries.flatten().map(|e| e.file_name()).collect::<Vec<_>>(),
        Err(e) => {
            out.push(format!("{rel:?}\tread-dir-error={:?}", e.kind()));
            return;
        }
    };
    names.sort();
    for name in names {
        tree(&path.join(&name), &rel.join(&name), out, depth - 1);
    }
}

/// Appends one line for `path`; true when it is a directory.
fn node(path: &Path, rel: &Path, out: &mut Vec<String>) -> bool {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            out.push(format!("{rel:?}\tabsent"));
            return false;
        }
        Err(e) => {
            out.push(format!("{rel:?}\terror={:?}", e.kind()));
            return false;
        }
    };
    let ft = meta.file_type();
    let mode = meta.mode() & 0o7777;
    // A directory's own size and mtime move with its entries, which are listed one by one.
    let line = if ft.is_dir() {
        format!("{rel:?}\tdir mode={mode:o}")
    } else {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| format!("{}.{:09}", d.as_secs(), d.subsec_nanos()))
            .unwrap_or_else(|| "unknown".to_string());
        let target = if ft.is_symlink() {
            format!(" target={:?}", std::fs::read_link(path).ok())
        } else {
            String::new()
        };
        format!(
            "{rel:?}\tsymlink={} mode={mode:o} size={} mtime={mtime}{target}",
            ft.is_symlink(),
            meta.len()
        )
    };
    out.push(line);
    ft.is_dir()
}
