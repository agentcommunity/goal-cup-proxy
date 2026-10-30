//! What `check` and `dump` read (PRD-08 section 1): an entry folder (with `rec/`), a `rec` folder,
//! a fetched entry folder (`rec.gcr.zst`, seam S13), a run folder or fetched outdir whose
//! subfolders are entries, or one part file. Every input is one zstd stream per entry: the part
//! files are concatenated in name order and decoded together, never a chunk alone.

use goalcup_format::read::{Reader, part_files, read_index};
use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The fetched server copy of one entry (seam S13).
pub const FETCHED: &str = "rec.gcr.zst";

/// Where the record came from: the participant's disk, the server copy, or a live entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Local,
    Server,
    Live,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Local => "local",
            Kind::Server => "server",
            Kind::Live => "live",
        }
    }
}

/// One entry: its folder and its compressed files in order, each with a byte limit (`--live`).
#[derive(Clone, Debug)]
pub struct Entry {
    /// The entry folder, where `status.json` lives.
    pub dir: PathBuf,
    pub files: Vec<(PathBuf, Option<u64>)>,
}

/// A resolved input: its kind and its entries in order.
#[derive(Debug)]
pub struct Input {
    pub kind: Kind,
    pub entries: Vec<Entry>,
}

/// Resolves a path. The error is a plain line for exit 3: the path is missing or holds no record.
pub fn resolve(path: &Path, live: bool, server: bool) -> Result<Input, String> {
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let mut entries = if path.is_file() {
        let up = path.parent().unwrap_or(Path::new("."));
        let dir = if up.file_name().is_some_and(|n| n == "rec") {
            up.parent().unwrap_or(up)
        } else {
            up
        };
        vec![Entry {
            dir: dir.to_path_buf(),
            files: vec![(path.to_path_buf(), None)],
        }]
    } else if let Some(e) = entry_at(path) {
        vec![e]
    } else {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .filter_map(|d| d.ok().map(|d| d.path()))
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        let mut found: Vec<(u64, Entry)> = subs
            .iter()
            .filter_map(|p| entry_at(p))
            .map(|e| (first_wall(&e), e))
            .collect();
        found.sort_by_key(|(w, _)| *w);
        found.into_iter().map(|(_, e)| e).collect()
    };
    if entries.is_empty() {
        return Err(format!("no record found in {}", path.display()));
    }
    let fetched = entries
        .iter()
        .any(|e| e.files.iter().any(|(p, _)| p.ends_with(FETCHED)));
    if live {
        for e in &mut entries {
            if let Some((last, limit)) = e.files.last_mut() {
                *limit = Some(durable_end(last));
            }
        }
    }
    let kind = match (live, server || fetched) {
        (true, _) => Kind::Live,
        (false, true) => Kind::Server,
        (false, false) => Kind::Local,
    };
    Ok(Input { kind, entries })
}

/// The entry at `dir`: a fetched entry, an entry with `rec/`, or a `rec` folder itself.
fn entry_at(dir: &Path) -> Option<Entry> {
    let one = |dir: &Path, files: Vec<PathBuf>| Entry {
        dir: dir.to_path_buf(),
        files: files.into_iter().map(|p| (p, None)).collect(),
    };
    let fetched = dir.join(FETCHED);
    if fetched.is_file() {
        return Some(one(dir, vec![fetched]));
    }
    let rec = dir.join("rec");
    if let Ok(parts) = part_files(&rec)
        && !parts.is_empty()
    {
        return Some(one(dir, parts));
    }
    match part_files(dir) {
        Ok(parts) if !parts.is_empty() => Some(one(dir.parent().unwrap_or(dir), parts)),
        _ => None,
    }
}

/// The compressed length up to the last whole index line of a part file (the durable point of a
/// live entry). With no index line, nothing of that file is durable yet.
fn durable_end(part: &Path) -> u64 {
    let name = part.to_string_lossy();
    let idx = PathBuf::from(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    read_index(&idx)
        .ok()
        .and_then(|v| v.last().map(|l| l.c_end))
        .unwrap_or(0)
}

/// The wall time of an entry's first PART, for ordering a run's entries. Unreadable sorts last.
fn first_wall(e: &Entry) -> u64 {
    let first = open(e).ok().and_then(|(mut rd, _)| rd.next());
    match first {
        Some(Ok(it)) => match it.rec {
            goalcup_format::Record::Part { wall_ns, .. } => wall_ns,
            _ => u64::MAX,
        },
        _ => u64::MAX,
    }
}

/// What the decoder saw: raw bytes out, and whether the input ended inside an open frame.
#[derive(Default)]
pub struct Meter {
    pub raw: Cell<u64>,
    pub open_frame: Cell<bool>,
}

/// A zstd decoder (window 2^27, the default limit) that counts raw bytes and treats the end of
/// input inside an open frame as the end of the stream.
pub struct Raw {
    d: zstd::stream::read::Decoder<'static, BufReader<Box<dyn Read>>>,
    m: Rc<Meter>,
}

impl Read for Raw {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.m.open_frame.get() {
            return Ok(0);
        }
        match self.d.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                self.m.open_frame.set(true);
                Ok(0)
            }
            Ok(n) => {
                self.m.raw.set(self.m.raw.get() + n as u64);
                Ok(n)
            }
            r => r,
        }
    }
}

/// Opens an entry as one record stream.
pub fn open(e: &Entry) -> io::Result<(Reader<Raw>, Rc<Meter>)> {
    let mut all: Box<dyn Read> = Box::new(io::empty());
    for (p, limit) in &e.files {
        let f = File::open(p)?;
        all = match limit {
            Some(n) => Box::new(all.chain(f.take(*n))),
            None => Box::new(all.chain(f)),
        };
    }
    let m = Rc::new(Meter::default());
    let d = zstd::stream::read::Decoder::new(all)?;
    Ok((Reader::new(Raw { d, m: m.clone() }), m))
}

/// The compressed bytes this entry reads.
pub fn compressed(e: &Entry) -> u64 {
    e.files
        .iter()
        .map(|(p, limit)| {
            let len = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            limit.map_or(len, |n| n.min(len))
        })
        .sum()
}
