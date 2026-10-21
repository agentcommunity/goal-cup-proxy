//! Chunk building: whole lines of the chain from where the server is, checked to continue its
//! copy, compressed into one zstd frame in a spool file.

use super::io_error;
use crate::CliError;
use crate::upload::api::RunPosition;
use goalcup_protocol::export::ChainPart;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The most a chunk body may be, compressed: the server's cap.
pub const MAX_CHUNK_BYTES: u64 = 32 * 1024 * 1024;
/// A chunk stops taking lines once its spool reaches this, leaving room for the frame's end
/// and the encoder's buffered block.
const CHUNK_SOFT_BYTES: u64 = MAX_CHUNK_BYTES - 2 * 1024 * 1024;
/// The zstd level: the library's default, fast and about 10x on a measured chain. No long
/// window.
const ZSTD_LEVEL: i32 = 3;
/// The longest line read: one chain record is at most this (`MAX_ENVELOPE_BYTES`).
pub(super) const MAX_LINE_BYTES: u64 = goalcup_protocol::capture::MAX_ENVELOPE_BYTES as u64;
/// Where the server is: the bytes it holds, the `seq` the next chunk starts at, and the hash
/// the next chunk's first record must extend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
}

impl From<&RunPosition> for Position {
    fn from(p: &RunPosition) -> Self {
        Position {
            offset: p.offset,
            next_seq: p.next_seq,
            head: p.head.clone(),
        }
    }
}

/// One chunk, built and waiting in its spool file.
#[derive(Debug)]
pub struct Chunk {
    pub offset: u64,
    /// Uncompressed bytes of the chain it holds.
    pub length: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub prev: String,
    pub head: String,
    /// SHA-256 of the compressed body.
    pub sha256: String,
    pub compressed: u64,
    pub spool: PathBuf,
}

/// Only the link of a chain record, whose fields an envelope carries at its top level
/// (`Envelope::link` is flattened); the rest of the line is not read here.
#[derive(serde::Deserialize)]
pub(super) struct LinkOnly {
    #[serde(flatten)]
    pub(super) link: goalcup_protocol::chain::Link,
}

/// A writer that counts and hashes what passes through it.
struct Tally<W> {
    inner: W,
    bytes: u64,
    sha: Sha256,
}

impl<W: Write> Write for Tally<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        self.sha.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// How many bytes of `parts` lie past the logical `offset` (a torn last line is not counted,
/// but neither is it looked for: this is the length of the chain, a cheap upper bound).
pub fn waiting_bytes(parts: &[ChainPart], offset: u64) -> u64 {
    parts
        .last()
        .map_or(0, |last| last.start.saturating_add(last.len))
        .saturating_sub(offset)
}

/// The next chunk of the chain `parts` from `pos`, compressed into `spool`, or `None` when no
/// complete line waits. The chunk is cut from the one part that holds `pos.offset` and stops
/// at the end of it; the next chunk starts in the next part. Every line's link is checked to
/// continue from `pos`, so a chunk can only ever extend what the server holds: a local chain
/// that does not is refused, not uploaded.
///
/// A server position below the first surviving part is refused too: those bytes were deleted
/// here, which only happens to bytes the server acknowledged, so the two copies disagree and
/// nothing is sent across the gap.
pub fn build_chunk(
    parts: &[ChainPart],
    pos: &Position,
    spool: &Path,
) -> Result<Option<Chunk>, CliError> {
    let Some(first) = parts.first() else {
        return Err(CliError::Internal(
            "the entry has no chain part to upload from".to_string(),
        ));
    };
    if pos.offset < first.start {
        return Err(CliError::Refused(format!(
            "the server holds {} bytes of this entry's chain, but the first {} bytes were \
             already deleted from this machine, so the bytes in between cannot be sent. The two \
             copies disagree; ask the organisers.",
            pos.offset, first.start
        )));
    }
    // The part that holds the next byte; none when everything on disk is already sent.
    let Some(part) = parts
        .iter()
        .find(|part| pos.offset < part.start.saturating_add(part.len))
    else {
        let _ = std::fs::remove_file(spool);
        return Ok(None);
    };
    let chain = part.path.as_path();
    let mut file = std::fs::File::open(chain).map_err(io_error(chain))?;
    file.seek(SeekFrom::Start(pos.offset - part.start))
        .map_err(io_error(chain))?;
    let mut reader = BufReader::new(file);
    let out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(spool)
        .map_err(io_error(spool))?;
    let tally = Tally {
        inner: out,
        bytes: 0,
        sha: Sha256::new(),
    };
    let mut encoder = zstd::stream::Encoder::new(tally, ZSTD_LEVEL).map_err(io_error(spool))?;
    let (mut length, mut seq, mut head) = (0u64, pos.next_seq, pos.head.clone());
    let mut lines = 0u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(io_error(chain))?;
        if read == 0 || line.last() != Some(&b'\n') {
            if read as u64 > MAX_LINE_BYTES {
                return Err(CliError::Internal(format!(
                    "{} holds a line longer than any chain record at byte {}",
                    chain.display(),
                    pos.offset + length
                )));
            }
            // The end, or a line the daemon is still writing: it goes in a later chunk.
            break;
        }
        let link = serde_json::from_slice::<LinkOnly>(&line[..line.len() - 1])
            .map_err(|e| {
                CliError::Internal(format!(
                    "{} has a record that is not a chain record at byte {}: {e}",
                    chain.display(),
                    pos.offset + length
                ))
            })?
            .link;
        if link.seq != seq || link.prev != head {
            return Err(CliError::Refused(format!(
                "{} does not continue where the server's copy ends (seq {seq}): its record at \
                 byte {} is seq {}. The local chain and the server's copy differ.",
                chain.display(),
                pos.offset + length,
                link.seq
            )));
        }
        encoder.write_all(&line).map_err(io_error(spool))?;
        length += line.len() as u64;
        lines += 1;
        seq += 1;
        head = link.hash;
        if encoder.get_ref().bytes >= CHUNK_SOFT_BYTES {
            break;
        }
    }
    let tally = encoder.finish().map_err(io_error(spool))?;
    tally.inner.sync_all().map_err(io_error(spool))?;
    if lines == 0 {
        let _ = std::fs::remove_file(spool);
        // A closed part ends at a whole record (it is closed at a durable frontier), so bytes
        // before its end that hold no whole line mean the part is damaged: waiting would
        // stall the upload for good, silently.
        if parts.last().is_some_and(|last| last.start != part.start) {
            return Err(CliError::Internal(format!(
                "{} is closed but holds no whole record at byte {}, so the upload cannot go on \
                 past it",
                chain.display(),
                pos.offset
            )));
        }
        return Ok(None);
    }
    if tally.bytes > MAX_CHUNK_BYTES {
        let _ = std::fs::remove_file(spool);
        return Err(CliError::Internal(format!(
            "a chunk compressed to {} bytes, over the {MAX_CHUNK_BYTES}-byte cap",
            tally.bytes
        )));
    }
    Ok(Some(Chunk {
        offset: pos.offset,
        length,
        first_seq: pos.next_seq,
        last_seq: seq - 1,
        prev: pos.head.clone(),
        head,
        sha256: hex::encode(tally.sha.finalize()),
        compressed: tally.bytes,
        spool: spool.to_path_buf(),
    }))
}
