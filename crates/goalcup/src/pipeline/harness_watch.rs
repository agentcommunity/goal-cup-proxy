//! The harness program on disk, looked at every 300 s on the recorder's control loop: a change
//! (an update, or an npm link moved to another build) is noted once, and nothing else is done.

use super::Mono;
use serde_json::{Value, json};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub(crate) const EVERY_NS: u64 = 300 * 1_000_000_000;

pub(crate) struct HarnessWatch {
    program: PathBuf,
    last: Value,
    at: Mono,
}

/// Size, mtime in ns and inode of what `program` names, links followed; null when it is gone.
fn stat(program: &Path) -> Value {
    std::fs::metadata(program).map_or(Value::Null, |m| {
        let mtime_ns = m
            .mtime()
            .saturating_mul(1_000_000_000)
            .saturating_add(m.mtime_nsec());
        json!({"size": m.len(), "mtime_ns": mtime_ns, "ino": m.ino()})
    })
}

impl HarnessWatch {
    pub(crate) fn new(program: &Path, now: Mono) -> HarnessWatch {
        let program = program.to_path_buf();
        let last = stat(&program);
        HarnessWatch {
            program,
            last,
            at: now,
        }
    }

    /// The `harness_changed` note when the program changed since the last look, at most every
    /// `EVERY_NS`.
    pub(crate) fn check(&mut self, now: Mono) -> Option<Value> {
        if now.ns().saturating_sub(self.at.ns()) < EVERY_NS {
            return None;
        }
        self.at = now;
        let after = stat(&self.program);
        if after == self.last {
            return None;
        }
        let before = std::mem::replace(&mut self.last, after.clone());
        Some(json!({"ev": "note", "what": "harness_changed",
            "program": self.program, "before": before, "after": after}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_noted_once_and_a_symlink_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, prog) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("prog"),
        );
        std::fs::write(&a, "aaa").unwrap();
        std::fs::write(&b, "bbbbb").unwrap();
        std::os::unix::fs::symlink(&a, &prog).unwrap();
        let mut w = HarnessWatch::new(&prog, Mono(0));
        assert_eq!(w.check(Mono(EVERY_NS - 1)), None);
        std::fs::remove_file(&prog).unwrap();
        std::os::unix::fs::symlink(&b, &prog).unwrap();
        let n = w.check(Mono(EVERY_NS)).expect("a note");
        assert_eq!(n["what"], "harness_changed");
        assert_eq!(
            (&n["before"]["size"], &n["after"]["size"]),
            (&json!(3), &json!(5))
        );
        assert_eq!(w.check(Mono(2 * EVERY_NS)), None);
        std::fs::remove_file(&prog).unwrap();
        let n = w.check(Mono(3 * EVERY_NS)).expect("a note");
        assert_eq!(n["after"], Value::Null);
        assert_eq!(w.check(Mono(4 * EVERY_NS)), None);
    }
}
