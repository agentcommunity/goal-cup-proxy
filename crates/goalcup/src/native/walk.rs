//! One full walk (PRD-04 sections 3 and 9): depth first from each root, every folder listed once,
//! only the chain of open folders held. Only the written notes outlive a pass.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::Noted;
use super::fsx::{self, Dir, Kind};
use super::pattern;
pub use super::pattern::Row;
use crate::pipeline::Recorder;

/// A regular file some row matches; `rel` is relative to `rows[row].root`.
pub struct Found {
    pub row: usize,
    pub rel: PathBuf,
    pub stat: fsx::Stat,
}

/// What one pass saw. `unlisted` holds the absolute folders and names it could not open, list or
/// stat for a reason other than "gone" (EMFILE, EACCES, EIO, ...): nothing under them is gone.
#[derive(Default)]
pub struct Walked {
    pub found: Vec<Found>,
    pub unlisted: Vec<PathBuf>,
}

/// Walks the rows. Only the notes it wrote outlive a pass, so each is written once.
#[derive(Default)]
pub struct Walker {
    noted: Noted,
}

struct Pass<'a> {
    rec: &'a Recorder,
    rows: &'a [Row],
    noted: &'a mut Noted,
    out: Walked,
}

impl Walker {
    /// One full walk. A root that is not there (not made yet, say) is skipped silently.
    pub fn pass(&mut self, rec: &Recorder, rows: &[Row]) -> Walked {
        let (noted, out) = (&mut self.noted, Walked::default());
        let mut p = Pass {
            rec,
            rows,
            noted,
            out,
        };
        for (i, row) in rows.iter().enumerate() {
            if rows[..i].iter().any(|r| r.root == row.root) {
                continue;
            }
            let top = Dir::open_root(&row.root).and_then(|d| Ok((d.stat_at(OsStr::new("."))?, d)));
            match top {
                Ok((st, dir)) => p.folder(&dir, (&row.root, st.dev), Path::new("")),
                Err(e) => p.failed(&e, &row.root),
            }
        }
        p.out
    }
}

impl Pass<'_> {
    /// Lists `dir` once; a name no row could want is skipped without a stat.
    fn folder(&mut self, dir: &Dir, (root, dev): (&Path, u64), rel: &Path) {
        let names = match dir.names() {
            Ok(names) => names,
            Err(e) => return self.failed(&e, &root.join(rel)),
        };
        let rows = self.rows;
        for name in names {
            let rel = rel.join(&name);
            let abs = root.join(&rel);
            let hit = pattern::first_match(rows, &abs);
            let mine = |r: &Row| r.root == root && r.pattern.may_enter(&rel);
            let enter = rows.iter().position(mine);
            if hit.is_none() && enter.is_none() {
                continue;
            }
            let stat = match dir.stat_at(&name) {
                Ok(stat) => stat,
                Err(e) => {
                    self.failed(&e, &abs);
                    continue;
                }
            };
            match (stat.kind, hit, enter) {
                // Nested roots reach a file twice; only the walk of its own row's root takes it.
                (Kind::File, Some((row, r)), _) if rows[row].root == root => {
                    let rel = r.to_path_buf();
                    self.out.found.push(Found { row, rel, stat });
                }
                (Kind::Symlink, Some((row, r)), _) => {
                    let path = pattern::path_id(&rows[row].name, r);
                    let ev = json!({"ev": "note", "what": "native_symlink", "path": path});
                    self.noted.once(self.rec, ev);
                }
                (Kind::Dir, _, Some(row)) if stat.dev != dev => {
                    let (row, path) = (&rows[row].name, pattern::path_id(&rows[row].name, &rel));
                    let what = "native_other_device";
                    let ev = json!({"ev": "note", "what": what, "row": row, "path": path});
                    self.noted.once(self.rec, ev);
                }
                (Kind::Dir, _, Some(_)) => match dir.open_child(&name) {
                    Ok(child) => self.folder(&child, (root, dev), &rel),
                    Err(e) => self.failed(&e, &abs),
                },
                _ => {}
            }
        }
    }

    /// Keeps a folder or name the pass could not read, unless it is gone or no longer a folder.
    fn failed(&mut self, e: &io::Error, abs: &Path) {
        let gone = matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR | libc::ELOOP));
        if !gone {
            self.out.unlisted.push(abs.to_path_buf());
        }
    }
}
