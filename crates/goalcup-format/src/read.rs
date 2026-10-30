//! Reading a record stream: one part file, a directory of part files in name order, or a fetched
//! concatenation of chunks. All three are one zstd stream (window 2^27, the default decoder limit).
//!
//! The end is tolerant: a stream that stops inside an open zstd frame (a live entry, or a crashed
//! one before recovery) gives every record up to the last flush and reports [`Tail::open_frame`];
//! a record cut at the end is reported in [`Tail::truncated`], never returned.

use crate::codec::{Codec, Record, get_varint};
use crate::link::{Chain, ZERO};
use crate::{IndexLine, MAX_BODY, invalid, kind};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// One record and where it starts in the entry's record stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The raw offset of the record's first byte.
    pub off: u64,
    pub rec: Record,
    /// For a LINK: whether its hash recomputes over the bytes before it.
    pub link_ok: Option<bool>,
}

/// How the stream ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    /// The zstd stream ended inside an open frame (expected for a live entry).
    pub open_frame: bool,
    /// A record was cut at the end: the raw offset where it starts. It was not returned.
    pub truncated: Option<u64>,
}

/// A streaming record reader over raw (decompressed) bytes. Raw offsets start at the first
/// PART's `raw_offset`, so a lone part file reads with the entry's offsets; the link chain starts
/// from that PART's `prev`.
pub struct Reader<R: Read> {
    r: BufReader<R>,
    codec: Codec,
    chain: Option<Chain>,
    off: u64,
    buf: Vec<u8>,
    tail: Tail,
    done: bool,
    is_open: fn(&R) -> bool,
}

/// A zstd decoder that treats the end of input inside an open frame as the end of the stream.
pub struct Zst<R: Read> {
    d: zstd::stream::read::Decoder<'static, BufReader<R>>,
    open: bool,
}

impl<R: Read> Read for Zst<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.open {
            return Ok(0);
        }
        match self.d.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                self.open = true;
                Ok(0)
            }
            r => r,
        }
    }
}

/// Reads a compressed record stream.
pub fn decode<R: Read>(r: R) -> io::Result<Reader<Zst<R>>> {
    let mut rd = Reader::new(Zst { d: zstd::stream::read::Decoder::new(r)?, open: false });
    rd.is_open = |z| z.open;
    Ok(rd)
}

/// The part files of a `rec` folder, in name order.
pub fn part_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rec-") && n.ends_with(".gcr.zst")))
        .collect();
    v.sort();
    Ok(v)
}

/// Reads every part file of a `rec` folder as one stream.
pub fn open_dir(dir: &Path) -> io::Result<Reader<Zst<Box<dyn Read + Send>>>> {
    let mut all: Box<dyn Read + Send> = Box::new(io::empty());
    for p in part_files(dir)? {
        all = Box::new(all.chain(File::open(p)?));
    }
    decode(all)
}

/// Reads an index file. Only lines that end in a newline count; a last line without one is
/// ignored (section 1).
pub fn read_index(path: &Path) -> io::Result<Vec<IndexLine>> {
    let text = std::fs::read(path)?;
    let whole = match text.iter().rposition(|&b| b == b'\n') {
        Some(i) => &text[..i],
        None => return Ok(Vec::new()),
    };
    let whole = std::str::from_utf8(whole).map_err(|_| invalid("index file is not text"))?;
    whole.split('\n').map(IndexLine::parse).collect()
}

impl<R: Read> Reader<R> {
    /// Reads raw (already decompressed) records.
    pub fn new(r: R) -> Self {
        Self { r: BufReader::with_capacity(1 << 16, r), codec: Codec::default(), chain: None, off: 0, buf: Vec::new(), tail: Tail::default(), done: false, is_open: |_| false }
    }

    /// How the stream ended; final once [`Reader::next`] has returned `None`.
    pub fn tail(&self) -> Tail {
        Tail { open_frame: (self.is_open)(self.r.get_ref()), ..self.tail }
    }

    pub fn get_ref(&self) -> &R {
        self.r.get_ref()
    }

    /// The raw offset of the next record.
    pub fn offset(&self) -> u64 {
        self.off
    }

    fn next_item(&mut self) -> io::Result<Option<Item>> {
        self.buf.clear();
        if !self.fill(1)? {
            return Ok(None);
        }
        let k = self.buf[0];
        if k == 0 {
            return Err(invalid(format!("record kind 0 at raw offset {}", self.off)));
        }
        let len = loop {
            if let Some((v, _)) = get_varint(&self.buf[1..])? {
                break v;
            }
            if !self.fill(1)? {
                return Ok(None);
            }
        };
        if len > MAX_BODY {
            return Err(invalid(format!("record of {len} bytes at raw offset {} is over 16 MiB", self.off)));
        }
        let head = self.buf.len();
        if !self.fill(len as usize)? {
            return Ok(None);
        }
        let rec = self.codec.decode(k, &self.buf[head..])?;
        if self.chain.is_none() {
            let prev = match &rec {
                Record::Part { raw_offset, prev, .. } => {
                    self.off = *raw_offset;
                    *prev
                }
                _ => ZERO,
            };
            self.chain = Some(Chain::new(prev));
        }
        let off = self.off;
        let chain = self.chain.as_mut().expect("set above");
        let link_ok = if k == kind::LINK {
            let n = self.buf.len() - 32;
            let Record::Link { hash, .. } = &rec else { unreachable!() };
            Some(chain.link(&self.buf[..n]) == *hash)
        } else {
            chain.feed(&self.buf);
            None
        };
        self.off += self.buf.len() as u64;
        Ok(Some(Item { off, rec, link_ok }))
    }

    /// Appends `n` more bytes to `buf`. False at the end of the stream: then a partial record is
    /// noted in the tail (an end with nothing read is clean).
    fn fill(&mut self, n: usize) -> io::Result<bool> {
        let want = self.buf.len() + n;
        while self.buf.len() < want {
            let avail = self.r.fill_buf()?;
            if avail.is_empty() {
                if !self.buf.is_empty() {
                    self.tail.truncated = Some(self.off);
                }
                return Ok(false);
            }
            let take = avail.len().min(want - self.buf.len());
            self.buf.extend_from_slice(&avail[..take]);
            self.r.consume(take);
        }
        Ok(true)
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = io::Result<Item>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let r = self.next_item().transpose();
        if !matches!(r, Some(Ok(_))) {
            self.done = true;
        }
        r
    }
}

/// Reads every record of a compressed stream into memory, with how it ended.
pub fn read_all<R: Read>(r: R) -> io::Result<(Vec<Item>, Tail)> {
    let mut rd = decode(r)?;
    let items = rd.by_ref().collect::<io::Result<Vec<_>>>()?;
    Ok((items, rd.tail()))
}
