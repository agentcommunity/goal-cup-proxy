//! Homes for tests. A child gets a temporary `HOME` and `GOALCUP_HOME` ([`TempHome`]), and
//! [`guard`] fails a test that changed the real `~/.claude`, `~/.codex` or `~/.hermes` (AGENTS.md
//! hard rule). Neither writes to, or reads a file in, the real home: the guard reads metadata only.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    pub fn goalcup_home(&self) -> PathBuf {
        self.dir.path().join(".goalcup")
    }

    /// Points `cmd` at the temporary homes and drops every variable that could send a harness
    /// back to a real home.
    pub fn apply<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        for var in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "HERMES_HOME"] {
            cmd.env_remove(var);
        }
        for xdg in ["CONFIG", "DATA", "STATE", "CACHE"] {
            cmd.env_remove(format!("XDG_{xdg}_HOME"));
        }
        cmd.env("HOME", self.home())
            .env("GOALCUP_HOME", self.goalcup_home())
    }
}

/// The owner's desktop apps rewrite `~/.claude` and `~/.codex` all the time, so only their
/// config and credential files are watched there (`~/.claude.json` is rewritten every few
/// minutes, so it is left out). `~/.hermes` is walked, four names deep.
const WATCHED_FILES: [&str; 3] = [
    ".codex/config.toml",
    ".codex/auth.json",
    ".claude/settings.json",
];
const WALKED: &str = ".hermes";

/// Fails the test, when dropped, if the real home's harness directories changed meanwhile.
/// Hold it for the test's life: `let _guard = common::home::guard();`.
pub fn guard() -> RealHomeGuard {
    RealHomeGuard(real_listing())
}

pub struct RealHomeGuard(String);

impl Drop for RealHomeGuard {
    fn drop(&mut self) {
        // A test that already failed keeps its own message.
        let changed = "the real ~/.claude, ~/.codex or ~/.hermes changed";
        if !std::thread::panicking() {
            assert_eq!(real_listing(), self.0, "{changed}");
        }
    }
}

fn real_listing() -> String {
    let home = std::env::var_os("HOME");
    home.map_or("no HOME".into(), |h| listing(Path::new(&h)))
}

/// One line per watched path.
pub fn listing(home: &Path) -> String {
    let mut out = Vec::new();
    for rel in WATCHED_FILES {
        node(&home.join(rel), Path::new(rel), &mut out);
    }
    tree(&home.join(WALKED), Path::new(WALKED), &mut out, 4);
    out.join("\n")
}

fn tree(path: &Path, rel: &Path, out: &mut Vec<String>, depth: usize) {
    if !node(path, rel, out) || depth == 0 {
        return;
    }
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
            names.sort();
            for name in names {
                tree(&path.join(&name), &rel.join(&name), out, depth - 1);
            }
        }
        Err(e) => out.push(format!("{rel:?}\tread-dir {:?}", e.kind())),
    }
}

/// Appends one line for `path` (type, mode, size, mtime, link target); true for a directory.
fn node(path: &Path, rel: &Path, out: &mut Vec<String>) -> bool {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            out.push(format!("{rel:?}\t{:?}", e.kind()));
            return false;
        }
    };
    let (ft, mode) = (meta.file_type(), meta.mode() & 0o7777);
    // A directory's own size and mtime move with its entries, which are listed one by one.
    out.push(if ft.is_dir() {
        format!("{rel:?}\tdir mode={mode:o}")
    } else {
        let (size, mtime) = (meta.len(), meta.modified().ok());
        let target = std::fs::read_link(path).ok();
        format!("{rel:?}\t{ft:?} mode={mode:o} size={size} mtime={mtime:?} target={target:?}")
    });
    ft.is_dir()
}
