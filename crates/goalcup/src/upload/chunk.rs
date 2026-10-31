//! Chunks from part files (PRD-06 section 3): where a chunk starts from the server's offset, where
//! it ends by index lines, its headers, and the two read passes. The uploader never compresses and
//! never decodes: a chunk is a byte range of one part file between two index lines.

use super::client::{ChunkHeaders, RunPosition};
use super::state::Confirmed;
use goalcup_format::read::{part_files, read_index};
use goalcup_format::{Durable, IndexKind, IndexLine, hex, index_name, part_name};
use hex::FromHex;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The Worker's chunk cap (`server/worker/src/chunks.ts:22`). There is no cap of our own.
pub const MAX_CHUNK: u64 = 32 << 20;

/// Compressed bytes `c_start..c_end` of one part file. `raw_start` is the server's offset S,
/// `head` the last line's link hash, `prev` the link hash at S (the server's head).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub part: PathBuf,
    pub c_start: u64,
    pub c_end: u64,
    pub raw_start: u64,
    pub raw_end: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub head: [u8; 32],
    pub prev: [u8; 32],
}

/// How far a chunk may reach.
#[derive(Clone, Debug)]
pub enum Bound {
    /// The live writer's durable point (seam S4): never past it.
    Durable(Durable),
    /// No live writer (null mode, `goalcup upload`, a dead entry): the last whole index line
    /// within the part file, which `cut` syncs (part, then index) before it returns the range.
    OnDisk,
}

/// What `cut` found. An I/O error (a malformed index line included) is the caller's to retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cut {
    Chunk(Chunk),
    /// Nothing waits past the server's offset.
    Nothing,
    /// The two copies disagree, or the index is wrong: nothing is sent across it.
    Stuck {
        reason: &'static str,
        offset: u64,
    },
}

/// The next chunk past the server's position (section 3).
pub fn cut(
    rec_dir: &Path,
    pos: &RunPosition,
    bound: &Bound,
    confirmed: Option<&Confirmed>,
) -> io::Result<Cut> {
    let s = pos.offset;
    let stuck = |reason| Ok(Cut::Stuck { reason, offset: s });
    let parts = parts(rec_dir)?;
    // The index line that ends at S, in the part before S.
    let mut at_s = None;
    if let Some(&p) = parts.iter().rev().find(|&&p| p < s) {
        let lines = index(rec_dir, p)?;
        at_s = lines
            .iter()
            .position(|l| l.raw_end == s)
            .map(|i| (p, lines, i));
    }
    let (part, c_start, prev, lines, from) = if parts.contains(&s) {
        // Case 1, first: S names a part (this also covers PRD-01's recovery fallback).
        let prev = match &at_s {
            Some((_, lines, i)) => Some(lines[*i].hash),
            None if s == 0 => Some([0; 32]),
            None => confirmed
                .filter(|c| c.raw == s)
                .and_then(|c| <[u8; 32]>::from_hex(&c.hash).ok()),
        };
        let Some(prev) = prev else {
            return stuck("no local link at the server's offset");
        };
        (s, 0, prev, index(rec_dir, s)?, 0)
    } else if let Some((p, lines, i)) = at_s {
        let line = lines[i];
        if line.kind == IndexKind::L {
            // Case 2: inside the part, after an L line.
            (p, line.c_end, line.hash, lines, i + 1)
        } else if parts.iter().any(|&q| q > s) {
            return stuck("the part after a closed part does not start at its end");
        } else {
            // Case 3: the next part starts at S and is not on disk yet.
            return Ok(Cut::Nothing);
        }
    } else if parts.is_empty()
        && (s == 0 || confirmed.is_some_and(|c| c.raw == s && c.hash == pos.head))
    {
        // Nothing written yet, or case 3 at a pruned boundary: section 7 prunes a prefix, so no
        // part file is left at all, and the server holds the whole entry as `confirmed` says. A
        // part still on disk with no line at S means the server holds bytes the local copy lost.
        return Ok(Cut::Nothing);
    } else {
        return stuck("the server's offset is not at a local index line");
    };
    if hex(&prev) != pos.head {
        return stuck("the server's head is not the local link at its offset");
    }

    let path = rec_dir.join(part_name(part));
    let (limit, flen) = match bound {
        Bound::Durable(d) => (d.line.raw_end, u64::MAX),
        Bound::OnDisk => (u64::MAX, fs::metadata(&path)?.len()),
    };
    let mut taken: Option<(IndexLine, IndexLine)> = None;
    for l in lines[from..]
        .iter()
        .take_while(|l| l.raw_end <= limit && l.c_end <= flen)
    {
        if l.c_end.saturating_sub(c_start) > MAX_CHUNK {
            if taken.is_none() {
                return stuck("an index line lies more than 32 MiB on: a writer bug");
            }
            break;
        }
        taken = Some((taken.map_or(*l, |t| t.0), *l));
    }
    let Some((first, last)) = taken else {
        return Ok(Cut::Nothing);
    };
    if last.raw_end <= s || last.c_end <= c_start {
        return stuck("the cut has no bytes: a writer bug");
    }
    if first.seq != pos.next_seq {
        return stuck("the server's next link is not the local one");
    }
    if let Bound::OnDisk = bound {
        sync_pair(rec_dir, part)?;
    }
    Ok(Cut::Chunk(Chunk {
        part: path,
        c_start,
        c_end: last.c_end,
        raw_start: s,
        raw_end: last.raw_end,
        first_seq: first.seq,
        last_seq: last.seq,
        head: last.hash,
        prev,
    }))
}

/// The headers of the chunk's PUT (section 3).
pub fn headers(entry_id: &str, c: &Chunk, sha256: String) -> ChunkHeaders {
    ChunkHeaders {
        entry_id: entry_id.to_string(),
        offset: c.raw_start,
        raw_len: c.raw_end.saturating_sub(c.raw_start),
        first_seq: c.first_seq,
        last_seq: c.last_seq,
        head: hex(&c.head),
        prev: hex(&c.prev),
        sha256,
    }
}

/// The first read pass: SHA-256 of the body, lowercase hex, through a 64 KiB buffer.
pub fn sha256(c: &Chunk) -> io::Result<String> {
    let mut h = Sha256::new();
    let n = io::copy(&mut BufReader::with_capacity(64 << 10, body(c)?), &mut h)?;
    if n != c.c_end.saturating_sub(c.c_start) {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(hex(&h.finalize()))
}

/// The second read pass: the body, through a descriptor of its own (never the writer's).
pub fn body(c: &Chunk) -> io::Result<io::Take<File>> {
    let mut f = File::open(&c.part)?;
    f.seek(SeekFrom::Start(c.c_start))?;
    Ok(f.take(c.c_end.saturating_sub(c.c_start)))
}

/// The bound's raw end and the compressed bytes between the server's offset `s` and it, over the
/// parts on disk (`upload.json` `durable_raw` and `waiting_bytes`). Reads index files only.
pub fn waiting(rec_dir: &Path, s: u64, bound: &Bound) -> io::Result<(u64, u64)> {
    let (mut end, mut bytes) = (0, 0);
    let parts = parts(rec_dir)?;
    for (i, &p) in parts.iter().enumerate() {
        // A part that ends at or before S adds no bytes: its index is not read.
        if let Some(&next) = parts.get(i + 1).filter(|&&n| n <= s) {
            end = end.max(next);
            continue;
        }
        let (limit, flen) = match bound {
            Bound::Durable(d) => (d.line.raw_end, u64::MAX),
            Bound::OnDisk => (u64::MAX, fs::metadata(rec_dir.join(part_name(p)))?.len()),
        };
        let (mut from, mut to) = (0, 0);
        for l in index(rec_dir, p)?
            .iter()
            .take_while(|l| l.raw_end <= limit && l.c_end <= flen)
        {
            if l.raw_end <= s {
                from = l.c_end;
            }
            (to, end) = (l.c_end, end.max(l.raw_end));
        }
        bytes += to.saturating_sub(from);
    }
    if let Bound::Durable(d) = bound {
        end = d.line.raw_end;
    }
    Ok((end, bytes))
}

/// The index line that ends at raw offset `raw`, when a part on disk holds one.
pub fn line_at(rec_dir: &Path, raw: u64) -> io::Result<Option<IndexLine>> {
    let Some(&p) = parts(rec_dir)?.iter().rev().find(|&&p| p < raw) else {
        return Ok(None);
    };
    Ok(index(rec_dir, p)?.into_iter().find(|l| l.raw_end == raw))
}

/// The point a beat names when no live writer publishes one (null mode, `goalcup upload`, a dead
/// entry): the last whole index line of the last part file whose `c_end` is within that file, its
/// part and index synced (section 3). Two syncs a call, so it is called only when a beat is due.
pub fn last_on_disk(rec_dir: &Path) -> io::Result<Option<IndexLine>> {
    for &p in parts(rec_dir)?.iter().rev() {
        let len = fs::metadata(rec_dir.join(part_name(p)))?.len();
        if let Some(l) = index(rec_dir, p)?.into_iter().rev().find(|l| l.c_end <= len) {
            sync_pair(rec_dir, p)?;
            return Ok(Some(l));
        }
    }
    Ok(None)
}

/// Syncs a part, then its index.
fn sync_pair(rec_dir: &Path, part: u64) -> io::Result<()> {
    File::open(rec_dir.join(part_name(part)))?.sync_data()?;
    File::open(rec_dir.join(index_name(part)))?.sync_data()
}

/// The part files' first raw offsets, in order: the names are zero-padded, so the listing that
/// recovery uses is already in number order.
fn parts(rec_dir: &Path) -> io::Result<Vec<u64>> {
    let first = |p: &PathBuf| {
        let name = p.file_name()?.to_str()?;
        let n = name
            .strip_prefix("rec-")?
            .strip_suffix(".gcr.zst")?
            .parse()
            .ok()?;
        (part_name(n) == name).then_some(n)
    };
    Ok(part_files(rec_dir)?.iter().filter_map(first).collect())
}

/// A part's index lines; none while it has no index yet.
fn index(rec_dir: &Path, part: u64) -> io::Result<Vec<IndexLine>> {
    match read_index(&rec_dir.join(index_name(part))) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        r => r,
    }
}
