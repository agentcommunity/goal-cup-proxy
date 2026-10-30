//! One full walk (PRD-04 sections 3 and 9): depth first from each root, every folder listed once,
//! only the chain of open folders held. Nothing but the written notes outlives a pass.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::fsx::{self, Dir, Kind};
use super::pattern::{self, Pattern};
use crate::pipeline::Recorder;

/// One native row, resolved.
pub struct Row {
    pub name: String,
    pub root: PathBuf,
    pub pattern: Pattern,
}

/// A regular file some row matches; `rel` is relative to `rows[row].root`.
pub struct Found {
    pub row: usize,
    pub rel: PathBuf,
    pub stat: fsx::Stat,
}

/// Walks the rows. Only the notes it wrote outlive a pass, so each is written once.
#[derive(Default)]
pub struct Walker {
    noted: HashSet<String>,
    found: Vec<Found>,
    seen: HashSet<PathBuf>,
}

struct Pass<'a> {
    rec: &'a Recorder,
    rows: &'a [Row],
    all: Vec<(&'a Path, &'a Pattern)>,
}

impl Walker {
    /// One full walk. A root that cannot be opened (not made yet, say) is skipped silently.
    pub fn pass(&mut self, rec: &Recorder, rows: &[Row]) -> Vec<Found> {
        let all = rows.iter().map(|r| (&*r.root, &r.pattern)).collect();
        let p = Pass { rec, rows, all };
        for (i, row) in rows.iter().enumerate() {
            let new = rows[..i].iter().all(|r| r.root != row.root);
            let Some(Ok(dir)) = new.then(|| Dir::open_root(&row.root)) else {
                continue;
            };
            let Ok(top) = dir.stat_at(OsStr::new(".")) else {
                continue;
            };
            self.folder(&p, &dir, (&row.root, top.dev), Path::new(""));
        }
        self.seen.clear();
        std::mem::take(&mut self.found)
    }

    /// Lists `dir` once; a name no row could want is skipped without a stat.
    fn folder(&mut self, p: &Pass, dir: &Dir, (root, dev): (&Path, u64), rel: &Path) {
        let Ok(names) = dir.names() else { return };
        for name in names {
            let rel = rel.join(&name);
            let abs = root.join(&rel);
            let (rows, hit) = (p.rows, pattern::first_match(&p.all, &abs));
            let mine = |r: &Row| r.root == root && r.pattern.may_enter(&rel);
            let enter = rows.iter().position(mine);
            let st = (hit.is_some() || enter.is_some()).then(|| dir.stat_at(&name).ok());
            let Some(stat) = st.flatten() else { continue };
            match (stat.kind, hit, enter) {
                // Nested roots reach a file twice; it is found once.
                (Kind::File, Some((row, r)), _) if self.seen.insert(abs.clone()) => {
                    let rel = r.to_path_buf();
                    self.found.push(Found { row, rel, stat });
                }
                (Kind::Symlink, Some((row, r)), _) => {
                    let path = pattern::path_id(&rows[row].name, r);
                    let ev = json!({"ev": "note", "what": "native_symlink", "path": path});
                    self.note(p.rec, ev);
                }
                (Kind::Dir, _, Some(row)) if stat.dev != dev => {
                    let (row, path) = (&rows[row].name, rel.to_string_lossy());
                    let what = "native_other_device";
                    let ev = json!({"ev": "note", "what": what, "row": row, "path": path});
                    self.note(p.rec, ev);
                }
                (Kind::Dir, _, Some(_)) => {
                    if let Ok(child) = dir.open_child(&name) {
                        self.folder(p, &child, (root, dev), &rel);
                    }
                }
                _ => {}
            }
        }
    }

    fn note(&mut self, rec: &Recorder, ev: Value) {
        if self.noted.insert(ev.to_string()) {
            rec.event(ev);
        }
    }
}
