//! Minimal, bounded HTTP/1.1 and WebSocket (RFC 6455) framing shared by the controlled
//! endpoint and the traffic generator — the two *peers* of the recorder under test. Nothing here is on
//! the recorder's path: the recording proxy under test relays and captures with its own production
//! parsers, and these helpers only produce and consume the traffic it relays.

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

pub const MAX_HEAD_BYTES: usize = 16 * 1024;
const READ_CHUNK: usize = 16 * 1024;

pub const OP_CONTINUATION: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// A parsed request or response head: the start line and lowercase header names.
#[derive(Debug, Clone)]
pub struct Head {
    pub start: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Whether a comma-separated header carries `token` (case-insensitive).
    pub fn has_token(&self, name: &str, token: &str) -> bool {
        self.headers
            .iter()
            .filter(|(k, _)| k == name)
            .flat_map(|(_, v)| v.split(','))
            .any(|t| t.trim().eq_ignore_ascii_case(token))
    }

    pub fn content_length(&self) -> io::Result<Option<u64>> {
        match self.header("content-length") {
            None => Ok(None),
            Some(v) => v
                .trim()
                .parse()
                .map(Some)
                .map_err(|_| invalid("bad content-length")),
        }
    }

    pub fn is_chunked(&self) -> bool {
        self.has_token("transfer-encoding", "chunked")
    }
}

pub fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// A byte stream with a small read-ahead buffer.
pub struct Conn<S> {
    pub io: S,
    buf: Vec<u8>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Conn<S> {
    pub fn new(io: S) -> Self {
        Self {
            io,
            buf: Vec::new(),
        }
    }

    async fn fill(&mut self) -> io::Result<usize> {
        let mut chunk = [0u8; READ_CHUNK];
        let n = self.io.read(&mut chunk).await?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// Read one head. `Ok(None)` is a clean EOF before any byte of a new head.
    pub async fn read_head(&mut self) -> io::Result<Option<Head>> {
        loop {
            if let Some(end) = find(&self.buf, b"\r\n\r\n") {
                let raw: Vec<u8> = self.buf.drain(..end + 4).collect();
                return parse_head(&raw[..end]).map(Some);
            }
            if self.buf.len() > MAX_HEAD_BYTES {
                return Err(invalid("head too large"));
            }
            if self.fill().await? == 0 {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
        }
    }

    /// Read exactly `n` bytes, handing each piece to `sink` as it arrives.
    pub async fn read_body(&mut self, mut n: u64, mut sink: impl FnMut(&[u8])) -> io::Result<()> {
        while n > 0 {
            if self.buf.is_empty() && self.fill().await? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let take = (self.buf.len() as u64).min(n) as usize;
            sink(&self.buf[..take]);
            self.buf.drain(..take);
            n -= take as u64;
        }
        Ok(())
    }

    async fn read_line(&mut self, max: usize) -> io::Result<Vec<u8>> {
        loop {
            if let Some(end) = find(&self.buf, b"\r\n") {
                let line: Vec<u8> = self.buf.drain(..end + 2).take(end).collect();
                return Ok(line);
            }
            if self.buf.len() > max {
                return Err(invalid("line too long"));
            }
            if self.fill().await? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }

    /// Read a chunked body to its terminal chunk (trailers skipped), handing each data piece
    /// to `sink` as it arrives.
    pub async fn read_chunked(&mut self, mut sink: impl FnMut(&[u8])) -> io::Result<()> {
        loop {
            let line = self.read_line(1024).await?;
            let text = std::str::from_utf8(&line).map_err(|_| invalid("chunk size"))?;
            let size_text = text.split(';').next().unwrap_or("").trim();
            let size = u64::from_str_radix(size_text, 16).map_err(|_| invalid("chunk size"))?;
            if size == 0 {
                // Trailer section up to the blank line.
                loop {
                    if self.read_line(MAX_HEAD_BYTES).await?.is_empty() {
                        return Ok(());
                    }
                }
            }
            self.read_body(size, &mut sink).await?;
            if !self.read_line(2).await?.is_empty() {
                return Err(invalid("chunk data not followed by CRLF"));
            }
        }
    }

    pub async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.io.write_all(bytes).await?;
        self.io.flush().await
    }

    /// Read one WebSocket frame, unmasking a masked payload. `Ok(None)` is EOF at a frame
    /// boundary.
    pub async fn read_frame(&mut self, max_payload: usize) -> io::Result<Option<Frame>> {
        while self.buf.len() < 2 {
            if self.fill().await? == 0 {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
        }
        let b0 = self.buf[0];
        let b1 = self.buf[1];
        let masked = b1 & 0x80 != 0;
        let (len_bytes, short) = match b1 & 0x7f {
            126 => (2, None),
            127 => (8, None),
            n => (0, Some(n as u64)),
        };
        let header = 2 + len_bytes + if masked { 4 } else { 0 };
        while self.buf.len() < header {
            if self.fill().await? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        let len = match short {
            Some(n) => n,
            None => self.buf[2..2 + len_bytes]
                .iter()
                .fold(0u64, |acc, b| (acc << 8) | u64::from(*b)),
        };
        if len > max_payload as u64 {
            return Err(invalid("frame too large"));
        }
        let mask = masked.then(|| {
            let at = 2 + len_bytes;
            [
                self.buf[at],
                self.buf[at + 1],
                self.buf[at + 2],
                self.buf[at + 3],
            ]
        });
        self.buf.drain(..header);
        let mut payload = Vec::with_capacity(len as usize);
        self.read_body(len, |piece| payload.extend_from_slice(piece))
            .await?;
        if let Some(mask) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }
        Ok(Some(Frame {
            fin: b0 & 0x80 != 0,
            opcode: b0 & 0x0f,
            masked,
            payload,
        }))
    }
}

/// One WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub masked: bool,
    pub payload: Vec<u8>,
}

/// Encode one WebSocket frame; `mask` is required from a client and absent from a server.
pub fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(if fin { 0x80 } else { 0 } | opcode);
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(mask) => {
            out.extend_from_slice(&mask);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// The frames of one message split into `fragment`-byte pieces (at least one frame).
pub fn fragment_message(
    opcode: u8,
    payload: &[u8],
    fragment: usize,
    mut mask: impl FnMut() -> Option<[u8; 4]>,
) -> Vec<Vec<u8>> {
    let fragment = fragment.max(1);
    let pieces: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(fragment).collect()
    };
    let last = pieces.len() - 1;
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, piece)| {
            let op = if i == 0 { opcode } else { OP_CONTINUATION };
            encode_frame(i == last, op, piece, mask())
        })
        .collect()
}

/// Encode one chunk of a chunked body. A zero-length `data` is the terminal chunk followed
/// by the empty trailer section: `0\r\n\r\n`.
pub fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_head(raw: &[u8]) -> io::Result<Head> {
    let text = std::str::from_utf8(raw).map_err(|_| invalid("head is not UTF-8"))?;
    let mut lines = text.split("\r\n");
    let start = lines.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(invalid("header line"))?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok(Head { start, headers })
}

/// A deterministic byte generator (xorshift64*), so both peers can predict content from a
/// shared test ID without exchanging it.
#[derive(Debug, Clone)]
pub struct Det(u64);

impl Det {
    pub fn new(seed: &str, salt: u64) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Self(h | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Exactly `n` bytes of lowercase words (2–9 letters) separated by single spaces. No
    /// built-in redaction rule matches it, and no word is long enough to stay an undecided
    /// candidate (an e-mail local part, a key) past the streaming redactor's 16 KiB hold, so
    /// it survives capture byte for byte.
    pub fn text(&mut self, n: usize) -> String {
        let mut out = String::with_capacity(n);
        let mut word = 0u64;
        let mut word_len = 2 + self.next_u64() % 8;
        while out.len() < n {
            if word == word_len && out.len() + 1 < n {
                out.push(' ');
                word = 0;
                word_len = 2 + self.next_u64() % 8;
            } else {
                out.push((b'a' + (self.next_u64() % 26) as u8) as char);
                word += 1;
            }
        }
        out
    }

    /// `n` bytes with the high bit set: never valid UTF-8 text, never a redaction match.
    pub fn high_bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| 0x80 | (self.next_u64() as u8)).collect()
    }
}
