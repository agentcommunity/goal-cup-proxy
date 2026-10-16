//! The real-`HOME` guard of every test that launches a stand-in under a shipped connector's
//! rules: the harness contract kit (`it/harness_contract_kit.rs`) and the connector contract's
//! dynamic half (`full/connector_contract.rs`). A run that escaped its isolated homes would write
//! into the participant's app-wide harness homes, so the guard snapshots them before the test and
//! compares after. It records file type, mode, size, mtime and symlink target only; it never
//! reads file contents.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

fn modified_stamp(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| format!("{}.{:09}", duration.as_secs(), duration.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// The app homes the owner's own desktop apps rewrite continuously while tests run (Claude and
/// Codex): their session files, caches, backups and app state churn, so they are watched by the
/// app-wide files below and never walked.
const CHURNING_APP_HOMES: [&str; 2] = [".codex", ".claude"];

/// The app-wide files an escaped harness would write in a churning app home: its config and its
/// credentials. These are what the rule "never write into ~/.claude or ~/.codex" protects, so
/// these are what the guard watches there. `~/.claude.json` is left out: Claude Code rewrites it
/// every few minutes, so watching it would fail the kit at random.
const WATCHED_APP_WIDE_FILES: [&str; 3] = [
    ".codex/config.toml",
    ".codex/auth.json",
    ".claude/settings.json",
];

/// Every other app-wide home is walked this many levels down: no app of the owner's writes
/// there, so anything created under it is a harness write.
const WALK_DEPTH: usize = 4;

/// The app-wide homes the guard walks: every `home.real_dirs` entry of every shipped connector,
/// relative to the user's home, except the churning ones. A new connector's own home is walked
/// with no edit here.
pub fn walked_homes() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let mut homes = Vec::new();
    for id in goalcup_protocol::pins::shipped_connector_ids() {
        let loaded = goalcup_protocol::harness::load_dir(
            &root.join(&id),
            goalcup_protocol::harness::Origin::Pinned,
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        for dir in &loaded.manifest.capture.home.real_dirs {
            if let Some(relative) = dir.strip_prefix("{user_home}/")
                && !CHURNING_APP_HOMES.contains(&relative)
                && !homes.iter().any(|h| h == relative)
            {
                homes.push(relative.to_string());
            }
        }
    }
    homes
}

/// One line per watched path: absent, or its type, mode, size, mtime and symlink target. File
/// contents are never read.
fn append_real_home_node(path: &Path, relative: &Path, listing: &mut Vec<String>) -> bool {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            listing.push(format!("{relative:?}\tabsent"));
            return false;
        }
        Err(error) => {
            listing.push(format!("{relative:?}\terror={:?}", error.kind()));
            return false;
        }
    };
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        "symlink"
    } else if file_type.is_dir() {
        "directory"
    } else if file_type.is_file() {
        "file"
    } else {
        "other"
    };
    // A directory's own size and mtime move with its entries, which are listed one by one.
    let identity = if file_type.is_dir() {
        format!("mode={:o}", metadata.mode() & 0o7777)
    } else {
        format!(
            "mode={:o};size={};mtime={}",
            metadata.mode() & 0o7777,
            metadata.len(),
            modified_stamp(&metadata),
        )
    };
    let symlink_target = if file_type.is_symlink() {
        std::fs::read_link(path)
            .map(|target| format!(";target={target:?}"))
            .unwrap_or_else(|error| format!(";target-error={:?}", error.kind()))
    } else {
        String::new()
    };
    listing.push(format!(
        "{relative:?}\ttype={kind};{identity}{symlink_target}"
    ));
    file_type.is_dir()
}

/// Every name under one walked home, so any creation there shows.
fn append_tree(path: &Path, relative: &Path, listing: &mut Vec<String>, depth: usize) {
    if !append_real_home_node(path, relative, listing) || depth == 0 {
        return;
    }
    let mut children = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>(),
        Err(error) => {
            listing.push(format!("{relative:?}\tread-dir-error={:?}", error.kind()));
            return;
        }
    };
    children.sort();
    for name in children {
        append_tree(&path.join(&name), &relative.join(&name), listing, depth - 1);
    }
}

/// The snapshot of `home`: the watched app-wide files, and every name under each walked home.
pub fn home_listing(home: &Path) -> String {
    let mut listing = Vec::new();
    for relative in WATCHED_APP_WIDE_FILES {
        append_real_home_node(&home.join(relative), Path::new(relative), &mut listing);
    }
    for walked in walked_homes() {
        append_tree(
            &home.join(&walked),
            Path::new(&walked),
            &mut listing,
            WALK_DEPTH,
        );
    }
    listing.join("\n")
}

fn real_home_listing() -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return "no HOME".to_string();
    };
    home_listing(&home)
}

/// Run `test`, and fail if the real `HOME`'s app-wide harness homes changed while it ran. The
/// test body is caught so this comparison runs before any unrelated failure is resumed.
pub fn with_real_home_guard(test: impl FnOnce()) {
    let before = real_home_listing();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));

    assert_eq!(
        real_home_listing(),
        before,
        "the real HOME's app-wide harness directories changed while the test ran"
    );
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
