//! HTTP body decoding (PRD-03 section 7, the `decoder_window` guard of section 11).
//!
//! The HTTP parsers hand each body's wire bytes (already de-chunked) to a [`BodyDecoder`] and get
//! decoded pieces of at most 32 KiB back as they are produced. Every coding in `content-encoding`,
//! and every transfer coding other than `chunked`, is undone in reverse order of application
//! (PRD-01 section 4); `identity` is skipped. This module opens no stream and writes no GAP: each
//! call answers a [`Step`], and the caller records what it says.
//!
//! There is no budget of our own. Deflate expands by at most about 1032 to 1, so the work per
//! push stays bounded by its input (PRD-17 D7), and PRD-02's memory guard decides about the
//! decoded bytes. A decoder's memory is counted in that guard through `Recorder::reserve`: the
//! 32 KiB window of gzip and deflate, the window of each zstd frame, and every allocation the
//! brotli decoder makes.

use crate::pipeline::{Recorder, Reserved};
use brotli_decompressor::{
    Allocator, BrotliDecompressStream, BrotliResult, BrotliState, SliceWrapper, SliceWrapperMut,
};
use bytes::Bytes;
use flate2::{Crc, Decompress, FlushDecompress, Status};

/// Decoded bytes per piece handed to the caller, and so per inflate call.
const PIECE: usize = 32 * 1024;
/// The window counted for gzip and deflate.
const DEFLATE_WINDOW: u64 = 32 * 1024;
/// The largest zstd window accepted, as a power of two: 8 MiB, the most an HTTP `zstd` content
/// coding may use (RFC 9659 section 3). A larger frame is a decode failure.
const ZSTD_WINDOW_LOG_MAX: u32 = 23;
/// The longest zstd frame header prefix that holds the window: magic, descriptor, dictionary ID
/// and frame content size.
const ZSTD_HEAD_MAX: usize = 17;
/// Unsupported coding tokens kept for GAP 3's `codings` tail, and the bytes kept of each.
const MAX_CODINGS: usize = 4;
const MAX_CODING_BYTES: usize = 32;

/// A body coding this module undoes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    /// `gzip` or `x-gzip`.
    Gzip,
    Deflate,
    Br,
    Zstd,
}

impl Coding {
    /// The coding's name for the OPEN meta `coding` list.
    pub fn name(self) -> &'static str {
        match self {
            Coding::Gzip => "gzip",
            Coding::Deflate => "deflate",
            Coding::Br => "br",
            Coding::Zstd => "zstd",
        }
    }
}

/// A body whose codings cannot be undone: GAP `unsupported_encoding`, no body byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// The tokens as sent (GAP 3 tail `codings`): at most 4, each cut to 32 bytes.
    pub codings: Vec<String>,
}

/// The codings named by `values`, in order of application: the `content-encoding` field values,
/// then the transfer codings other than `chunked`. `identity` is skipped. Any other unknown token
/// makes the whole body unsupported.
pub fn parse_codings(values: &[&str]) -> Result<Vec<Coding>, Unsupported> {
    let tokens = values
        .iter()
        .flat_map(|v| v.split(','))
        .map(|t| t.trim_matches([' ', '\t']))
        .filter(|t| !t.is_empty());
    let mut codings = Vec::new();
    for token in tokens.clone() {
        let coding = match token.to_ascii_lowercase().as_str() {
            "identity" => continue,
            "gzip" | "x-gzip" => Coding::Gzip,
            "deflate" => Coding::Deflate,
            "br" => Coding::Br,
            "zstd" => Coding::Zstd,
            _ => {
                let codings = tokens.take(MAX_CODINGS).map(cut).collect();
                return Err(Unsupported { codings });
            }
        };
        codings.push(coding);
    }
    Ok(codings)
}

/// `token` cut to at most [`MAX_CODING_BYTES`], on a character boundary.
fn cut(token: &str) -> String {
    let mut end = token.len().min(MAX_CODING_BYTES);
    while !token.is_char_boundary(end) {
        end -= 1;
    }
    token[..end].to_owned()
}

/// What the caller records after a [`BodyDecoder`] call. Once a call answered `Fault` or
/// `NotDecoded`, every later call answers the same again; the caller writes the GAP once, when
/// the body ends, so its `wire` tail counts the whole body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing to record beyond the pieces already handed out.
    Ok,
    /// The coding broke part way, or the body ended inside it. The decoded prefix already went
    /// out; the caller writes GAP `decode_failed` and ends the stream `decode_error`.
    Fault,
    /// The `decoder_window` guard: the window did not fit `Recorder::room()`, so the body is not
    /// decoded and no piece went out. The caller writes GAP `memory_guard` with `lost` 0 and tail
    /// `{"guard":"decoder_window","limit":limit,"wire":wire()}`, and relays on.
    NotDecoded { limit: u64 },
}

/// One body's decoders, one stage per coding, chained in decoding order.
pub struct BodyDecoder {
    stages: Vec<Stage>,
    guard: Guard,
    wire: u64,
    /// Set once a call answered `Fault` or `NotDecoded`; later calls only count wire bytes.
    stop: Option<Step>,
}

impl BodyDecoder {
    /// A decoder for `codings` in order of application, as [`parse_codings`] gives them. With
    /// none, the body passes through in pieces.
    pub fn new(codings: Vec<Coding>, rec: &Recorder) -> BodyDecoder {
        BodyDecoder {
            stages: codings.into_iter().rev().map(|c| Stage::new(c, rec)).collect(),
            guard: Guard {
                rec: rec.clone(),
                held: Vec::new(),
                emitted: false,
            },
            wire: 0,
            stop: None,
        }
    }

    /// Encoded bytes seen so far (END tail `wire`, and the GAP tails).
    pub fn wire(&self) -> u64 {
        self.wire
    }

    /// Decodes the next wire bytes of the body; each decoded piece goes to `out` as it is made.
    pub fn push(&mut self, input: &[u8], out: &mut dyn FnMut(Bytes)) -> Step {
        self.wire += input.len() as u64;
        if let Some(step) = self.stop {
            return step;
        }
        let result = run(&mut self.stages, input, &mut self.guard, out);
        self.settle(result)
    }

    /// The body ended on the wire: `Fault` if a coding is cut short. Every decoder hands its
    /// output out as it goes, so nothing is left to hand out here.
    pub fn finish(mut self) -> Step {
        if let Some(step) = self.stop {
            return step;
        }
        let complete = self.stages.iter().all(Stage::complete);
        self.settle(if complete { Ok(()) } else { Err(Halt::Invalid) })
    }

    fn settle(&mut self, result: Result<(), Halt>) -> Step {
        let step = match result {
            Ok(()) => return Step::Ok,
            Err(Halt::Invalid) => Step::Fault,
            Err(Halt::Window(limit)) => Step::NotDecoded { limit },
        };
        // Decoding is over: the windows go back to the guard now, not when the body ends.
        self.guard.held.clear();
        self.stages.clear();
        self.stop = Some(step);
        step
    }
}

enum Halt {
    /// Not a valid stream of its coding (or incomplete at the end of the body).
    Invalid,
    /// A window did not fit `Recorder::room()` before the first piece went out; the room at that
    /// moment, this body's own earlier windows still held.
    Window(u64),
}

/// The `decoder_window` guard of one body: the windows it holds, and whether a piece went out.
struct Guard {
    rec: Recorder,
    held: Vec<Reserved>,
    emitted: bool,
}

impl Guard {
    /// Counts a stage's `window` in the recorder's guard. Before the first piece went out, a
    /// window that does not fit `room()` stops decoding. After it, the body is part recorded and
    /// the window is only counted (a later zstd frame), so PRD-02's guard sees it.
    fn admit(&mut self, window: u64) -> Result<(), Halt> {
        let room = self.rec.room();
        if !self.emitted && room < window {
            return Err(Halt::Window(room));
        }
        self.held.push(self.rec.reserve(window));
        Ok(())
    }
}

/// Each decoded piece of a stage, handed to the next stage or to the caller.
type Emit<'a> = dyn FnMut(&[u8], &mut Guard) -> Result<(), Halt> + 'a;

/// Feeds `input` to the first stage, its output to the next, and the last output to `out`.
fn run(
    stages: &mut [Stage],
    input: &[u8],
    g: &mut Guard,
    out: &mut dyn FnMut(Bytes),
) -> Result<(), Halt> {
    match stages.split_first_mut() {
        None => {
            for piece in input.chunks(PIECE) {
                g.emitted = true;
                out(Bytes::copy_from_slice(piece));
            }
            Ok(())
        }
        Some((stage, rest)) => stage.feed(input, g, &mut |piece, g| run(rest, piece, g, out)),
    }
}

struct Stage {
    kind: Kind,
    fed: bool,
}

enum Kind {
    Gzip(Box<Gzip>),
    Deflate(Box<Deflate>),
    Brotli(Box<Brotli>),
    Zstd(Box<Zstd>),
}

impl Stage {
    fn new(coding: Coding, rec: &Recorder) -> Stage {
        let kind = match coding {
            Coding::Gzip => Kind::Gzip(Box::default()),
            Coding::Deflate => Kind::Deflate(Box::default()),
            Coding::Br => Kind::Brotli(Box::new(Brotli::new(rec))),
            Coding::Zstd => Kind::Zstd(Box::default()),
        };
        Stage { kind, fed: false }
    }

    fn feed(&mut self, input: &[u8], g: &mut Guard, emit: &mut Emit<'_>) -> Result<(), Halt> {
        if input.is_empty() {
            return Ok(());
        }
        if !self.fed {
            self.fed = true;
            // Brotli counts its allocations, zstd each frame's window, in their own code.
            if matches!(self.kind, Kind::Gzip(_) | Kind::Deflate(_)) {
                g.admit(DEFLATE_WINDOW)?;
            }
        }
        match &mut self.kind {
            Kind::Gzip(gzip) => gzip.feed(input, g, emit),
            Kind::Deflate(deflate) => deflate.feed(input, g, emit),
            Kind::Brotli(brotli) => brotli.feed(input, g, emit),
            Kind::Zstd(zstd) => zstd.feed(input, g, emit),
        }
    }

    /// Did the coded stream end exactly? A stage that got no input needs no decoding.
    fn complete(&self) -> bool {
        !self.fed
            || match &self.kind {
                Kind::Gzip(gzip) => gzip.complete(),
                Kind::Deflate(deflate) => deflate.ended,
                Kind::Brotli(brotli) => brotli.ended,
                Kind::Zstd(zstd) => zstd.frame_done && zstd.head.is_empty(),
            }
    }
}

/// Inflates from `input` until it is used up or the stream ends (`true`), emitting at most
/// [`PIECE`] at a time.
fn inflate(
    z: &mut Decompress,
    out: &mut [u8],
    input: &mut &[u8],
    mut crc: Option<&mut Crc>,
    g: &mut Guard,
    emit: &mut Emit<'_>,
) -> Result<bool, Halt> {
    loop {
        let (in_before, out_before) = (z.total_in(), z.total_out());
        let result = z.decompress(input, out, FlushDecompress::None);
        // Counted before the result is looked at: a call that meets corruption may still have
        // decoded valid output first, and that prefix is kept.
        let consumed = (z.total_in() - in_before) as usize;
        let produced = (z.total_out() - out_before) as usize;
        *input = &input[consumed..];
        if produced > 0 {
            if let Some(crc) = crc.as_deref_mut() {
                crc.update(&out[..produced]);
            }
            emit(&out[..produced], g)?;
        }
        if result.map_err(|_| Halt::Invalid)? == Status::StreamEnd {
            return Ok(true);
        }
        if consumed == 0 && produced == 0 {
            return if input.is_empty() {
                Ok(false)
            } else {
                Err(Halt::Invalid)
            };
        }
    }
}

/// RFC 1952 members, one after another: header, raw deflate, CRC-32 and size trailer.
struct Gzip {
    state: GzipState,
    z: Decompress,
    crc: Crc,
    out: Vec<u8>,
    members: u64,
}

enum GzipState {
    Header(GzHead),
    Body,
    Trailer(Vec<u8>),
}

/// Where a member header stands (RFC 1952 section 2.3). It holds none of the header's bytes,
/// only counters, so a header of any length is read: the 10 fixed bytes, then each optional
/// field its flags name, cleared from `flags` once read.
#[derive(Default)]
struct GzHead {
    /// Fixed bytes seen, 0 to 10.
    fixed: u8,
    flags: u8,
    /// FEXTRA length bytes seen, and the length read so far.
    xlen_seen: u8,
    xlen: usize,
    /// Bytes of FEXTRA data or FHCRC still to skip.
    skip: usize,
}

const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;

impl GzHead {
    /// Takes header bytes from `input`; `true` once the header is complete. Reserved flags and a
    /// wrong magic or method are refused.
    fn take(&mut self, input: &mut &[u8]) -> Result<bool, Halt> {
        loop {
            let n = self.skip.min(input.len());
            *input = &input[n..];
            self.skip -= n;
            if self.skip > 0 {
                return Ok(false);
            }
            if self.fixed < 10 || (self.flags & FEXTRA != 0 && self.xlen_seen < 2) {
                let Some((&b, rest)) = input.split_first() else {
                    return Ok(false);
                };
                *input = rest;
                match self.fixed {
                    0..=2 if b != [0x1f, 0x8b, 8][usize::from(self.fixed)] => {
                        return Err(Halt::Invalid);
                    }
                    3 if b & 0xe0 != 0 => return Err(Halt::Invalid),
                    3 => self.flags = b,
                    10 => {
                        self.xlen |= usize::from(b) << (8 * self.xlen_seen);
                        self.xlen_seen += 1;
                    }
                    _ => {}
                }
                self.fixed = (self.fixed + 1).min(10);
            } else if self.flags & FEXTRA != 0 {
                self.flags &= !FEXTRA;
                self.skip = self.xlen;
            } else if let Some(flag) = [FNAME, FCOMMENT].into_iter().find(|f| self.flags & *f != 0)
            {
                let Some(zero) = input.iter().position(|&b| b == 0) else {
                    *input = &[];
                    return Ok(false);
                };
                *input = &input[zero + 1..];
                self.flags &= !flag;
            } else if self.flags & FHCRC != 0 {
                self.flags &= !FHCRC;
                self.skip = 2;
            } else {
                return Ok(true);
            }
        }
    }
}

impl Default for Gzip {
    fn default() -> Self {
        Gzip {
            state: GzipState::Header(GzHead::default()),
            z: Decompress::new(false),
            crc: Crc::new(),
            out: vec![0; PIECE],
            members: 0,
        }
    }
}

impl Gzip {
    fn feed(&mut self, mut input: &[u8], g: &mut Guard, emit: &mut Emit<'_>) -> Result<(), Halt> {
        while !input.is_empty() {
            match &mut self.state {
                GzipState::Header(header) => {
                    if header.take(&mut input)? {
                        self.z.reset(false);
                        self.crc.reset();
                        self.state = GzipState::Body;
                    }
                }
                GzipState::Body => {
                    let crc = Some(&mut self.crc);
                    if inflate(&mut self.z, &mut self.out, &mut input, crc, g, emit)? {
                        self.state = GzipState::Trailer(Vec::with_capacity(8));
                    }
                }
                GzipState::Trailer(trailer) => {
                    let take = (8 - trailer.len()).min(input.len());
                    trailer.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    if trailer.len() == 8 {
                        let crc =
                            u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
                        let size =
                            u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
                        if crc != self.crc.sum() || size != self.crc.amount() {
                            return Err(Halt::Invalid);
                        }
                        self.members += 1;
                        self.state = GzipState::Header(GzHead::default());
                    }
                }
            }
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.members > 0 && matches!(&self.state, GzipState::Header(h) if h.fixed == 0)
    }
}

/// HTTP `deflate`: zlib (RFC 1950) as specified, or the raw deflate some servers send, told
/// apart by the zlib header check on the first two bytes. The first byte may arrive alone.
struct Deflate {
    z: Decompress,
    /// The first two bytes were seen and `z` is set for zlib or raw.
    told: bool,
    first: Option<u8>,
    ended: bool,
    out: Vec<u8>,
}

impl Default for Deflate {
    fn default() -> Self {
        Deflate {
            z: Decompress::new(true),
            told: false,
            first: None,
            ended: false,
            out: vec![0; PIECE],
        }
    }
}

impl Deflate {
    /// `input` is never empty (`Stage::feed`).
    fn feed(&mut self, input: &[u8], g: &mut Guard, emit: &mut Emit<'_>) -> Result<(), Halt> {
        if !self.told {
            let (b0, b1, held) = match self.first.take() {
                Some(b0) => (b0, input[0], true),
                None if input.len() == 1 => {
                    self.first = Some(input[0]);
                    return Ok(());
                }
                None => (input[0], input[1], false),
            };
            let zlib =
                b0 & 0x0f == 8 && b0 >> 4 <= 7 && (u16::from(b0) << 8 | u16::from(b1)) % 31 == 0;
            self.z.reset(zlib);
            self.told = true;
            if held {
                self.inflate(&[b0], g, emit)?;
            }
        }
        self.inflate(input, g, emit)
    }

    /// `input` is never empty.
    fn inflate(
        &mut self,
        mut input: &[u8],
        g: &mut Guard,
        emit: &mut Emit<'_>,
    ) -> Result<(), Halt> {
        if self.ended {
            return Err(Halt::Invalid);
        }
        if inflate(&mut self.z, &mut self.out, &mut input, None, g, emit)? {
            self.ended = true;
            if !input.is_empty() {
                return Err(Halt::Invalid);
            }
        }
        Ok(())
    }
}

/// HTTP `br` (RFC 7932), strict: no large window, and bytes after the stream's end are invalid.
/// Its memory is counted as its decoder allocates it ([`BrotliHeap`]).
struct Brotli {
    state: BrotliState<BrotliHeap, BrotliHeap, BrotliHeap>,
    ended: bool,
    out: Vec<u8>,
}

/// The brotli decoder's memory: zeroed boxed slices from the global allocator, each counted in
/// PRD-02's guard (`Recorder::reserve`) until it is freed or dropped. The count is the
/// allocation itself, not the output so far and not resident pages: brotli-decompressor 5.0.3
/// allocates the ring buffer, the whole declared window, at the first meta-block, and shrinks it
/// only when that meta-block is also the last. A flushed stream (SSE) is many meta-blocks, so it
/// holds its full window, up to 16 MiB, from the first push. Untouched calloc pages that look
/// small in RSS are still this memory once the allocator hands out reused heap.
///
/// While `strict` (no piece of the body went out yet), an allocation larger than `room()` sets
/// `over` to that room. The decoder cannot be refused memory without failing, so the allocation
/// is made; [`Brotli::feed`] then answers `Halt::Window` before any output goes out, and the
/// state, with its memory, is dropped at once.
struct BrotliHeap {
    rec: Recorder,
    strict: bool,
    over: Option<u64>,
}

struct BrotliCells<T>(Box<[T]>, Option<Reserved>);

impl<T> Default for BrotliCells<T> {
    fn default() -> Self {
        BrotliCells(Box::default(), None)
    }
}

impl<T> SliceWrapper<T> for BrotliCells<T> {
    fn slice(&self) -> &[T] {
        &self.0
    }
}

impl<T> SliceWrapperMut<T> for BrotliCells<T> {
    fn slice_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T: Clone + Default> Allocator<T> for BrotliHeap {
    type AllocatedMemory = BrotliCells<T>;

    fn alloc_cell(&mut self, len: usize) -> BrotliCells<T> {
        let bytes = (len * std::mem::size_of::<T>()) as u64;
        let room = self.rec.room();
        if self.strict && room < bytes {
            self.over.get_or_insert(room);
        }
        let cells = vec![T::default(); len].into_boxed_slice();
        BrotliCells(cells, Some(self.rec.reserve(bytes)))
    }

    fn free_cell(&mut self, _cells: BrotliCells<T>) {}
}

impl Brotli {
    fn new(rec: &Recorder) -> Brotli {
        let heap = || BrotliHeap {
            rec: rec.clone(),
            strict: true,
            over: None,
        };
        Brotli {
            state: BrotliState::new_strict(heap(), heap(), heap()),
            ended: false,
            out: vec![0; PIECE],
        }
    }

    /// Sets each heap's `strict`; answers the room an allocation did not fit while strict.
    fn heaps(&mut self, strict: bool) -> Option<u64> {
        let s = &mut self.state;
        [&mut s.alloc_u8, &mut s.alloc_u32, &mut s.alloc_hc]
            .into_iter()
            .fold(None, |over, heap| {
                heap.strict = strict;
                over.or(heap.over)
            })
    }

    fn feed(&mut self, input: &[u8], g: &mut Guard, emit: &mut Emit<'_>) -> Result<(), Halt> {
        if self.ended {
            return Err(Halt::Invalid);
        }
        let (mut available_in, mut input_offset) = (input.len(), 0);
        loop {
            self.heaps(!g.emitted);
            let (mut available_out, mut output_offset, mut total_out) = (self.out.len(), 0, 0);
            let result = BrotliDecompressStream(
                &mut available_in,
                &mut input_offset,
                input,
                &mut available_out,
                &mut output_offset,
                &mut self.out,
                &mut total_out,
                &mut self.state,
            );
            // Set only while no piece went out: refused before this call's output is emitted.
            if let Some(room) = self.heaps(!g.emitted) {
                return Err(Halt::Window(room));
            }
            // Emitted before the result is looked at: output decoded before corruption is kept.
            if output_offset > 0 {
                emit(&self.out[..output_offset], g)?;
            }
            match result {
                BrotliResult::NeedsMoreOutput => {}
                BrotliResult::NeedsMoreInput => return Ok(()),
                BrotliResult::ResultSuccess => {
                    self.ended = true;
                    return if available_in == 0 {
                        Ok(())
                    } else {
                        Err(Halt::Invalid)
                    };
                }
                BrotliResult::ResultFailure => return Err(Halt::Invalid),
            }
        }
    }
}

/// HTTP `zstd` (RFC 8878 frames, RFC 9659 window limit), one frame or several in a row. The body
/// is complete when its last frame is. Each frame's window is read from its header before the
/// decoder sees it, and counted whole: libzstd allocates its window buffer at the frame's window
/// size when the frame starts, whatever the output so far. An RSS reading that looks smaller only
/// shows pages not yet touched; do not trim this count from it (nor brotli's, [`BrotliHeap`]).
struct Zstd {
    /// Built at the first frame, once its window is counted.
    z: Option<zstd::stream::raw::Decoder<'static>>,
    /// The start of a frame whose window is not read yet.
    head: Vec<u8>,
    at_frame_start: bool,
    frame_done: bool,
    /// The largest window counted so far; the decoder keeps its buffer between frames.
    counted: u64,
    out: Vec<u8>,
}

impl Default for Zstd {
    fn default() -> Self {
        Zstd {
            z: None,
            head: Vec::new(),
            at_frame_start: true,
            frame_done: false,
            counted: 0,
            out: vec![0; PIECE],
        }
    }
}

impl Zstd {
    fn feed(&mut self, mut input: &[u8], g: &mut Guard, emit: &mut Emit<'_>) -> Result<(), Halt> {
        while !input.is_empty() {
            if !self.at_frame_start {
                input = self.decode(input, g, emit)?;
                continue;
            }
            let take = input.len().min(ZSTD_HEAD_MAX - self.head.len());
            self.head.extend_from_slice(&input[..take]);
            input = &input[take..];
            if let Some(window) = zstd_window(&self.head)? {
                let window = window.min(1 << ZSTD_WINDOW_LOG_MAX);
                if window > self.counted {
                    g.admit(window - self.counted)?;
                    self.counted = window;
                }
                self.at_frame_start = false;
                let head = std::mem::take(&mut self.head);
                self.feed(&head, g, emit)?;
            }
        }
        Ok(())
    }

    /// Decodes inside one frame; answers the input left after the frame ended, if any.
    fn decode<'a>(
        &mut self,
        input: &'a [u8],
        g: &mut Guard,
        emit: &mut Emit<'_>,
    ) -> Result<&'a [u8], Halt> {
        use zstd::stream::raw::{DParameter, Decoder, InBuffer, Operation, OutBuffer};
        let z = match &mut self.z {
            Some(z) => z,
            slot => {
                let mut z = Decoder::new().map_err(|_| Halt::Invalid)?;
                z.set_parameter(DParameter::WindowLogMax(ZSTD_WINDOW_LOG_MAX))
                    .map_err(|_| Halt::Invalid)?;
                slot.insert(z)
            }
        };
        let mut source = InBuffer::around(input);
        loop {
            let before = source.pos();
            let mut sink = OutBuffer::around(&mut self.out[..]);
            let result = z.run(&mut source, &mut sink);
            let produced = sink.pos();
            if produced > 0 {
                emit(&self.out[..produced], g)?;
            }
            // Zero: the frame ended and everything it decoded was flushed.
            self.frame_done = result.map_err(|_| Halt::Invalid)? == 0;
            let rest = &input[source.pos()..];
            if self.frame_done && !rest.is_empty() {
                self.at_frame_start = true;
                return Ok(rest);
            }
            // A frame that ended with no input left holds nothing back, even when its output
            // filled `out` exactly: running zstd again would only ask for the next frame.
            if rest.is_empty() && (self.frame_done || produced < self.out.len()) {
                return Ok(rest);
            }
            if !rest.is_empty() && produced == 0 && source.pos() == before {
                return Err(Halt::Invalid);
            }
        }
    }
}

/// The window of the zstd frame starting at `h` (RFC 8878 section 3.1.1.1); `None` if more
/// bytes are needed. A skippable frame has none.
fn zstd_window(h: &[u8]) -> Result<Option<u64>, Halt> {
    let [m0, m1, m2, m3, ..] = *h else {
        return Ok(None);
    };
    let magic = u32::from_le_bytes([m0, m1, m2, m3]);
    if magic & 0xffff_fff0 == 0x184d_2a50 {
        return Ok(Some(0));
    }
    if magic != 0xfd2f_b528 {
        return Err(Halt::Invalid);
    }
    let Some(&fhd) = h.get(4) else {
        return Ok(None);
    };
    if fhd & 0x08 != 0 {
        return Err(Halt::Invalid);
    }
    if fhd & 0x20 == 0 {
        let Some(&wd) = h.get(5) else {
            return Ok(None);
        };
        let base = 1u64 << (10 + u32::from(wd >> 3));
        return Ok(Some(base + base / 8 * u64::from(wd & 7)));
    }
    // Single segment: the window is the frame content size.
    let at = 5 + [0, 1, 2, 4][usize::from(fhd & 3)];
    let len = [1, 2, 4, 8][usize::from(fhd >> 6)];
    let Some(field) = h.get(at..at + len) else {
        return Ok(None);
    };
    let mut size = [0; 8];
    size[..len].copy_from_slice(field);
    let size = u64::from_le_bytes(size);
    Ok(Some(if len == 2 { size + 256 } else { size }))
}
