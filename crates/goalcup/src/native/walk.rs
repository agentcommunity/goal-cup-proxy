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

/// Walks the rows; remembers the notes it wrote, so each is written once.
#[derive(Default)]
pub struct Walker {
    noted: HashSet<String>,
}

struct Pass<'a> {
    rec: &'a Recorder,
    rows: &'a [Row],
    all: Vec<(&'a Path, &'a Pattern)>,
    found: Vec<Found>,
    seen: HashSet<PathBuf>,
}

impl Walker {
    /// One full walk. A root that cannot be opened (not made yet, say) is skipped silently.
    pub fn pass(&mut self, rec: &Recorder, rows: &[Row]) -> Vec<Found> {
        let all = rows.iter().map(|r| (r.root.as_path(), &r.pattern)).collect();
        let mut p = Pass { rec, rows, all, found: Vec::new(), seen: HashSet::new() };
        for (i, row) in rows.iter().enumerate() {
            let new = rows[..i].iter().all(|r| r.root != row.root);
            let Some(Ok(dir)) = new.then(|| Dir::open_root(&row.root)) else { continue };
            let Ok(top) = dir.stat_at(OsStr::new(".")) else { continue };
            self.folder(&mut p, &dir, (&row.root, top.dev), Path::new(""));
        }
        p.found
    }

    /// Lists `dir` once; a name no row could want is skipped without a stat.
    fn folder(&mut self, p: &mut Pass, dir: &Dir, (root, dev): (&Path, u64), rel: &Path) {
        let Ok(names) = dir.names() else { return };
        for name in names {
            let rel = rel.join(&name);
            let abs = root.join(&rel);
            let (rows, hit) = (p.rows, pattern::first_match(&p.all, &abs));
            let enter = rows.iter().position(|r| r.root == root && r.pattern.may_enter(&rel));
            let st = (hit.is_some() || enter.is_some()).then(|| dir.stat_at(&name).ok());
            let Some(st) = st.flatten() else { continue };
            match (st.kind, hit, enter) {
                // Nested roots reach a file twice; it is found once.
                (Kind::File, Some((row, r)), _) if p.seen.insert(abs.clone()) => {
                    p.found.push(Found { row, rel: r.to_path_buf(), stat: st });
                }
                (Kind::Symlink, Some((row, r)), _) => {
                    let path = pattern::path_id(&rows[row].name, r);
                    self.note(p.rec, json!({"ev": "note", "what": "native_symlink", "path": path}));
                }
                (Kind::Dir, _, Some(row)) if st.dev != dev => {
                    let (row, path) = (&rows[row].name, rel.to_string_lossy());
                    let what = "native_other_device";
                    self.note(p.rec, json!({"ev": "note", "what": what, "row": row, "path": path}));
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
