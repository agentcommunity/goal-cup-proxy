//! Part files, their flush index and crash recovery (sections 1 and 12).
//!
//! The writer compresses as it writes: zstd level 3, window 2^`window_log` (27 outside tests),
//! long distance matching, no checksum, no content size. The compressor is flushed at every LINK,
//! so every index line is a block boundary. The writer reads no clock: the caller (PRD-02's
//! writer thread) passes [`Now`], writes the events (start, count, clock, suspend, seal) and
//! decides when to link and sync.

use crate::codec::{Codec, Record};
use crate::link::{Chain, ZERO};
use crate::read::{self, part_files, read_index};
use crate::{
    GapReason, IndexKind, IndexLine, LEVEL, WINDOW_LOG, ZSTD_END_BLOCK, hex, index_name, invalid,
    part_name,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use zstd::stream::write::Encoder;
use zstd::zstd_safe::CParameter;

/// Writer settings. The defaults are the recorder's; `window_log` is a parameter for tests only
/// (the recorder always writes window 2^27 with long matching, ledger F2).
#[derive(Clone, Copy, Debug)]
pub struct PartOptions {
    pub window_log: u32,
    /// Close a part file once it holds this many raw bytes.
    pub rotate_at: u64,
    /// Or once it has been open this long on the record clock and holds at least one link.
    pub rotate_after_ns: u64,
}

impl Default for PartOptions {
    fn default() -> Self {
        Self {
            window_log: WINDOW_LOG,
            rotate_at: 512 << 20,
            rotate_after_ns: 3600 * 1_000_000_000,
        }
    }
}

/// A moment: wall time (`CLOCK_REALTIME`, Unix ns) and the record clock (a monotonic clock that
/// includes sleep, section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Now {
    pub wall_ns: u64,
    pub mono_ns: u64,
}

/// A point the uploader may send up to: a part file, its index length and the line there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Durable {
    /// The part file's name, for example `rec-00000000000000000000.gcr.zst`.
    pub part: String,
    /// The index file's length in bytes up to and including this line.
    pub index_len: u64,
    pub line: IndexLine,
}

/// The zstd encoder of a part file: level 3, window 2^`window_log`, long distance matching,
/// no checksum, no content size, so a frame flushed at a block boundary can be closed by hand.
pub fn new_encoder<W: Write>(w: W, window_log: u32) -> io::Result<Encoder<'static, W>> {
    let mut e = Encoder::new(w, LEVEL)?;
    e.set_parameter(CParameter::WindowLog(window_log))?;
    e.set_parameter(CParameter::EnableLongDistanceMatching(true))?;
    e.set_parameter(CParameter::ChecksumFlag(false))?;
    e.set_parameter(CParameter::ContentSizeFlag(false))?;
    Ok(e)
}

/// The part file under the encoder, counting its length.
struct Out {
    f: File,
    n: u64,
}

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.f.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.f.flush()
    }
}

/// Writes one entry's record stream. One writer thread owns it.
pub struct PartWriter {
    dir: PathBuf,
    opts: PartOptions,
    entry: String,
    boot: u64,
    boot_id: [u8; 16],
    part: u64,
    part_start: u64,
    part_mono: u64,
    links_in_part: u64,
    raw: u64,
    raw_at_link: u64,
    enc: Option<Encoder<'static, Out>>,
    idx: Option<File>,
    idx_len: u64,
    codec: Codec,
    chain: Chain,
    seq: u64,
    last: Option<Durable>,
    buf: Vec<u8>,
}

impl PartWriter {
    /// Starts a new entry in `dir` (boot 0, part file 0 at raw offset 0) and writes its PART.
    pub fn open(
        dir: &Path,
        opts: PartOptions,
        entry: &str,
        boot_id: [u8; 16],
        now: Now,
    ) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let mut w = Self::empty(dir, opts, entry, boot_id);
        w.start_part(now, ZERO)?;
        Ok(w)
    }

    fn empty(dir: &Path, opts: PartOptions, entry: &str, boot_id: [u8; 16]) -> Self {
        Self {
            dir: dir.to_path_buf(),
            opts,
            entry: entry.to_string(),
            boot: 0,
            boot_id,
            part: 0,
            part_start: 0,
            part_mono: 0,
            links_in_part: 0,
            raw: 0,
            raw_at_link: 0,
            enc: None,
            idx: None,
            idx_len: 0,
            codec: Codec::default(),
            chain: Chain::new(ZERO),
            seq: 0,
            last: None,
            buf: Vec::new(),
        }
    }

    /// The boot this writer writes.
    pub fn boot(&self) -> u64 {
        self.boot
    }

    /// Raw bytes written since the last LINK.
    pub fn raw_since_link(&self) -> u64 {
        self.raw - self.raw_at_link
    }

    /// Writes one record. PART and LINK are the writer's own and are refused here.
    pub fn write(&mut self, r: &Record) -> io::Result<()> {
        if matches!(r, Record::Part { .. } | Record::Link { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "PART and LINK are written by the writer",
            ));
        }
        self.put(r)
    }

    fn put(&mut self, r: &Record) -> io::Result<()> {
        self.buf.clear();
        self.codec.encode(r, &mut self.buf)?;
        self.chain.feed(&self.buf);
        self.emit()
    }

    fn emit(&mut self) -> io::Result<()> {
        self.enc.as_mut().ok_or_else(closed)?.write_all(&self.buf)?;
        self.raw += self.buf.len() as u64;
        Ok(())
    }

    /// Writes a LINK, flushes the compressor to a block boundary, then writes an `L` line.
    pub fn link(&mut self, now: Now) -> io::Result<IndexLine> {
        let (seq, hash) = self.put_link(now)?;
        let enc = self.enc.as_mut().ok_or_else(closed)?;
        enc.flush()?;
        let c_end = enc.get_ref().n;
        self.line(IndexLine {
            kind: IndexKind::L,
            seq,
            raw_end: self.raw,
            c_end,
            hash,
        })
    }

    fn put_link(&mut self, now: Now) -> io::Result<(u64, [u8; 32])> {
        let seq = self.seq;
        self.buf.clear();
        let r = Record::Link {
            seq,
            wall_ns: now.wall_ns,
            mono_ns: now.mono_ns,
            hash: [0; 32],
        };
        self.codec.encode(&r, &mut self.buf)?;
        let n = self.buf.len() - 32;
        let hash = self.chain.link(&self.buf[..n]);
        self.buf[n..].copy_from_slice(&hash);
        self.emit()?;
        self.seq += 1;
        self.links_in_part += 1;
        self.raw_at_link = self.raw;
        Ok((seq, hash))
    }

    fn line(&mut self, line: IndexLine) -> io::Result<IndexLine> {
        let text = format!("{line}\n");
        self.idx
            .as_mut()
            .ok_or_else(closed)?
            .write_all(text.as_bytes())?;
        self.idx_len += text.len() as u64;
        self.last = Some(Durable {
            part: part_name(self.part_start),
            index_len: self.idx_len,
            line,
        });
        Ok(line)
    }

    /// `fdatasync` the part file, then its index. Returns the durable point: the last index line.
    pub fn sync(&mut self) -> io::Result<Option<Durable>> {
        if let Some(enc) = &self.enc {
            enc.get_ref().f.sync_data()?;
        }
        if let Some(idx) = &self.idx {
            idx.sync_data()?;
        }
        Ok(self.last.clone())
    }

    /// Rotates when the part file holds `rotate_at` raw bytes, or has been open `rotate_after_ns`
    /// and holds a link. Returns the closed part file's durable point.
    pub fn rotate_if_due(&mut self, now: Now) -> io::Result<Option<Durable>> {
        let full = self.raw - self.part_start >= self.opts.rotate_at;
        let old = now.mono_ns.saturating_sub(self.part_mono) >= self.opts.rotate_after_ns
            && self.links_in_part > 0;
        if !(full || old) {
            return Ok(None);
        }
        let d = self.close(now, IndexKind::E)?;
        self.part += 1;
        self.start_part(now, d.line.hash)?;
        Ok(Some(d))
    }

    /// Ends the entry: a LINK, the frame end, an `E` line, both files synced. The caller writes
    /// the seal event (and what was still open) first.
    pub fn seal(mut self, now: Now) -> io::Result<Durable> {
        self.close(now, IndexKind::E)
    }

    /// Closes the part file: LINK, frame end, then the line, `fdatasync` part file then index,
    /// then the compressor is dropped. Never an `L` line for this last link.
    fn close(&mut self, now: Now, kind: IndexKind) -> io::Result<Durable> {
        let (seq, hash) = self.put_link(now)?;
        let out = self.enc.take().ok_or_else(closed)?.finish()?;
        let line = IndexLine {
            kind,
            seq,
            raw_end: self.raw,
            c_end: out.n,
            hash,
        };
        self.line(line)?;
        out.f.sync_data()?;
        self.idx.take().ok_or_else(closed)?.sync_data()?;
        Ok(self.last.clone().expect("line written"))
    }

    /// Opens the part file at the current raw offset and writes its PART.
    fn start_part(&mut self, now: Now, prev: [u8; 32]) -> io::Result<()> {
        let dir = self.dir.clone();
        let create = |name: String| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(name))
        };
        let f = create(part_name(self.raw))?;
        self.idx = Some(create(index_name(self.raw))?);
        self.idx_len = 0;
        self.enc = Some(new_encoder(Out { f, n: 0 }, self.opts.window_log)?);
        self.part_start = self.raw;
        self.part_mono = now.mono_ns;
        self.links_in_part = 0;
        self.chain = Chain::new(prev);
        let part = Record::Part {
            entry: self.entry.clone(),
            boot: self.boot,
            part: self.part,
            raw_offset: self.raw,
            wall_ns: now.wall_ns,
            mono_ns: now.mono_ns,
            boot_id: self.boot_id,
            next_link: self.seq,
            prev,
        };
        self.put(&part)
    }

    /// Crash recovery (section 12, steps 1 to 4): closes what the dead boot left and starts the
    /// next boot, or, with `seal`, closes the entry.
    pub fn recover(dir: &Path, opts: PartOptions, rc: Recover<'_>) -> io::Result<Recovered> {
        let files = part_files(dir)?;
        // Step 1: after a reboot, drop index lines past the durable index length.
        if let Some((name, len)) = rc.durable {
            for p in &files {
                let fname = file_name(p);
                if fname.as_str() > name {
                    cut(&idx_path(p), 0)?;
                } else if fname == name {
                    cut(&idx_path(p), len)?;
                }
            }
        }
        // A part file with no usable index line (its boot died before its first LINK there) is
        // deleted with its index; lines past the file's length are dropped.
        let mut dead_mtime = None;
        let mut last = None;
        for p in files.iter().rev() {
            let flen = fs::metadata(p)?.len();
            let lines: Vec<IndexLine> = match read_index(&idx_path(p)) {
                Ok(l) => l.into_iter().take_while(|l| l.c_end <= flen).collect(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e),
            };
            if lines.is_empty() {
                if dead_mtime.is_none() {
                    dead_mtime = fs::metadata(p)?.modified().ok();
                }
                fs::remove_file(p)?;
                let _ = fs::remove_file(idx_path(p));
                continue;
            }
            last = Some((p.clone(), lines));
            break;
        }

        let Some((path, lines)) = last else {
            // Nothing with an index line: start boot 0 afresh, with a recorder_down gap that the
            // caller writes right after its start event.
            // The deleted file's time is taken on this boot's clock.
            let (start, id) = rc.dead_start.unwrap_or_else(|| {
                let wall = dead_mtime
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(rc.now.wall_ns, |d| d.as_nanos() as u64);
                let back = rc.now.wall_ns.saturating_sub(wall);
                let mono_ns = rc.now.mono_ns.saturating_sub(back);
                (
                    Now {
                        wall_ns: wall,
                        mono_ns,
                    },
                    rc.boot_id,
                )
            });
            let down = gap(start, &rc, id);
            let writer = Self::open(dir, opts, rc.entry, rc.boot_id, rc.now)?;
            return Ok(Recovered {
                writer: Some(writer),
                gap_after_start: Some(down),
                line: None,
            });
        };

        let line = *lines.last().expect("not empty");
        keep_lines(&idx_path(&path), lines.len())?;
        let f = OpenOptions::new().read(true).write(true).open(&path)?;
        f.set_len(line.c_end)?;

        // The last PART and the last LINK before the kept line.
        let mut part = None;
        let mut link = None;
        for it in read::decode((&f).take(line.c_end))? {
            match it?.rec {
                p @ Record::Part { .. } => part = Some(p),
                Record::Link {
                    seq,
                    wall_ns,
                    mono_ns,
                    hash,
                } => link = Some((seq, Now { wall_ns, mono_ns }, hash)),
                _ => {}
            }
        }
        let (
            Some(Record::Part {
                entry,
                boot,
                part,
                raw_offset,
                boot_id,
                ..
            }),
            Some((seq, at, hash)),
        ) = (part, link)
        else {
            return Err(invalid(
                "part file has no PART or no LINK before its last index line",
            ));
        };
        if seq != line.seq || hash != line.hash {
            return Err(invalid("the last LINK does not match the last index line"));
        }

        // Step 2: close an open frame with an empty last block.
        let mut f = f;
        f.seek(SeekFrom::End(0))?;
        let mut n = line.c_end;
        if line.kind == IndexKind::L {
            f.write_all(&ZSTD_END_BLOCK)?;
            n += ZSTD_END_BLOCK.len() as u64;
        }

        // Step 3: the recovery frame: the dead boot's COUNT (unless an earlier recovery frame
        // already holds it), the recorder_down GAP, the seal if asked, a LINK, an `R` line.
        let mut w = Self::empty(dir, opts, &entry, rc.boot_id);
        w.boot = boot;
        w.part = part;
        w.part_start = raw_offset;
        w.raw = line.raw_end;
        w.seq = line.seq + 1;
        w.chain = Chain::new(line.hash);
        w.codec.t = at.mono_ns;
        w.idx = Some(OpenOptions::new().append(true).open(idx_path(&path))?);
        w.idx_len = fs::metadata(idx_path(&path))?.len();
        w.enc = Some(new_encoder(Out { f, n }, opts.window_log)?);
        if line.kind != IndexKind::R {
            let json = (rc.count_at)(line.seq);
            w.write(&Record::Event {
                t: at.mono_ns,
                json,
            })?;
        }
        w.write(&gap(at, &rc, boot_id))?;
        if let Some(seal) = rc.seal {
            w.write(&Record::Event {
                t: rc.now.mono_ns,
                json: seal,
            })?;
            let d = w.close(rc.now, IndexKind::R)?;
            return Ok(Recovered {
                writer: None,
                gap_after_start: None,
                line: Some(d.line),
            });
        }
        let d = w.close(rc.now, IndexKind::R)?;

        // Step 4: the next boot's part file.
        w.boot += 1;
        w.part += 1;
        w.start_part(rc.now, d.line.hash)?;
        Ok(Recovered {
            writer: Some(w),
            gap_after_start: None,
            line: Some(d.line),
        })
    }
}

/// What recovery needs from the caller.
pub struct Recover<'a> {
    /// The entry id, for an entry that left no index line at all.
    pub entry: &'a str,
    /// This machine's current `boot_id`.
    pub boot_id: [u8; 16],
    pub now: Now,
    /// `cursors.json`'s durable index length (part file name, index bytes). Pass it only when
    /// the machine rebooted since it was written (a new `boot_id`).
    pub durable: Option<(&'a str, u64)>,
    /// The dead boot's COUNT event JSON at the given LINK `seq`: its `counts.jsonl` line, or a
    /// recount of the dead boot from its part files when that line is missing.
    pub count_at: &'a dyn Fn(u64) -> Vec<u8>,
    /// The tail of the `recorder_down` gap: `{"cause": ..., "prev": {...}}`, plus `"exits": n`
    /// for an entry that left no index line. Recovery adds `boot_id` when the gap's start was
    /// taken under another `boot_id` (section 6).
    pub tail: serde_json::Map<String, serde_json::Value>,
    /// For an entry that left no index line: the dead boot's start time and the `boot_id` it was
    /// taken under, if the watchdog has them (else the deleted part file's modification time).
    pub dead_start: Option<(Now, [u8; 16])>,
    /// A `recovered` seal event JSON: close the entry instead of starting a boot. For an entry
    /// that left no index line, the caller writes it after the start event and the gap instead.
    pub seal: Option<Vec<u8>>,
}

/// What recovery did.
pub struct Recovered {
    /// The next boot's writer, its PART written. The caller writes `start` (`resume: true`),
    /// then `prev_boot`. None after a `recovered` seal in the recovery frame.
    pub writer: Option<PartWriter>,
    /// For an entry that left no index line: boot 0's `recorder_down` GAP, to write right after
    /// its start event.
    pub gap_after_start: Option<Record>,
    /// The recovery frame's `R` line.
    pub line: Option<IndexLine>,
}

/// The `recorder_down` GAP from `start` (taken under `boot_id`) to now. A GAP has an extent
/// (invariant 12): on one clock the record clock moves at least 1 ns; across a reboot the tail
/// names the new clock and the wall time moves at least 1 ns.
fn gap(start: Now, rc: &Recover<'_>, boot_id: [u8; 16]) -> Record {
    let mut tail = rc.tail.clone();
    let end = rc.now;
    let (mut start_wall, mut start_mono) = (start.wall_ns.min(end.wall_ns), start.mono_ns);
    if boot_id == rc.boot_id {
        start_mono = start_mono.min(end.mono_ns.saturating_sub(1));
    } else {
        tail.insert("boot_id".into(), hex(&rc.boot_id).into());
        start_wall = start_wall.min(end.wall_ns.saturating_sub(1));
    }
    Record::Gap {
        sid: 0,
        reason: GapReason::RecorderDown,
        start_wall,
        end_wall: end.wall_ns,
        start_mono,
        end_mono: end.mono_ns,
        off: 0,
        lost: 0,
        tail: serde_json::Value::Object(tail).to_string().into_bytes(),
    }
}

fn closed() -> io::Error {
    io::Error::other("the part file is closed")
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

fn idx_path(part: &Path) -> PathBuf {
    let name = file_name(part);
    part.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")))
}

/// Shortens a file to at most `len` bytes; a missing file stays missing.
fn cut(p: &Path, len: u64) -> io::Result<()> {
    match OpenOptions::new().write(true).open(p) {
        Ok(f) if f.metadata()?.len() > len => f.set_len(len),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Truncates an index file to its first `n` lines.
fn keep_lines(p: &Path, n: usize) -> io::Result<()> {
    let text = fs::read(p)?;
    let end = text
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(n - 1)
        .map_or(text.len(), |(i, _)| i + 1);
    cut(p, end as u64)
}
