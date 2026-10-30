//! The goalcup record format, GCR1 (PRD-01).
//!
//! One entry (one `goalcup start` or `goalcup continue`) is one record stream: light binary
//! records (`kind`, `len`, body) holding raw, masked bytes, split into zstd part files
//! (`rec-<raw offset>.gcr.zst`) with a local flush index (`.idx`). Every loss is a GAP with a
//! start and an end; a LINK once per flush chains everything before it.
//!
//! - [`codec`]: varint, zigzag, encode and decode of every record.
//! - [`link`]: the link hash.
//! - [`part`]: [`PartWriter`]: compression, flush at LINK, index lines, rotation, fsync,
//!   crash recovery.
//! - [`read`]: streaming reader over part files or a fetched concatenation, [`read::read_index`].
//! - [`dump`]: one JSON line per record (`goalcup-verify dump`).

pub mod codec;
pub mod dump;
pub mod link;
pub mod part;
pub mod read;

pub use codec::Record;
pub use part::{Durable, Now, PartOptions, PartWriter, Recover, Recovered};
pub use read::{Item, Reader, Tail};

use std::fmt;
use std::io;

/// The 4 magic bytes at the start of every PART body.
pub const MAGIC: &[u8; 4] = b"GCR1";
/// The format version in PART.
pub const VERSION: u8 = 1;
/// The domain prefix of every link hash (section 10).
pub const LINK_DOMAIN: &[u8] = b"GCR1-LINK";
/// The largest record body a reader accepts, so a corrupt file cannot make it allocate wildly.
pub const MAX_BODY: u64 = 16 << 20;
/// The largest DATA payload a writer writes.
pub const MAX_DATA: usize = 256 << 10;
/// The largest OPEN metadata or EVENT body a writer writes.
pub const MAX_JSON: usize = 1 << 20;
/// A zstd block header with Last_Block 1, type raw, size 0 (RFC 8878 section 3.1.1.2): it closes
/// an open frame that ends on a block boundary (section 12, recovery step 2).
pub const ZSTD_END_BLOCK: [u8; 3] = [0x01, 0x00, 0x00];
/// The zstd window: 2^27, the largest `zstd -d` accepts with no flag (ledger F2, locked).
pub const WINDOW_LOG: u32 = 27;
/// zstd level 3 (PRD-06 may change it with a measured reason).
pub const LEVEL: i32 = 3;
/// DATA `flags` bit 0: end of WebSocket message.
pub const FLAG_FIN: u8 = 0x01;

/// Record kinds (section 3). 0x00 is never valid; an unknown kind is skipped by its length.
pub mod kind {
    pub const PART: u8 = 0x01;
    pub const OPEN: u8 = 0x02;
    pub const DATA: u8 = 0x03;
    pub const END: u8 = 0x04;
    pub const GAP: u8 = 0x05;
    pub const MASK: u8 = 0x06;
    pub const EVENT: u8 = 0x07;
    pub const LINK: u8 = 0x08;
}

/// A numbered enum with names, where a number this build does not know is kept as `Other(n)`
/// and shown as `other(n)`.
macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($v:ident = $n:literal => $s:literal,)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($v,)* Other(u8) }
        impl $name {
            pub fn from_u8(n: u8) -> Self { match n { $($n => Self::$v,)* n => Self::Other(n) } }
            pub fn to_u8(self) -> u8 { match self { $(Self::$v => $n,)* Self::Other(n) => n } }
            /// The name readers show; an unknown number is `other(n)`.
            pub fn name(self) -> String {
                match self { $(Self::$v => $s.to_string(),)* Self::Other(n) => format!("other({n})") }
            }
            pub fn from_name(s: &str) -> Option<Self> { match s { $($s => Some(Self::$v),)* _ => None } }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.name()) }
        }
    };
}

named!(
    /// `skind` in OPEN (section 4).
    StreamKind {
        Conn = 1 => "conn",
        HttpReq = 2 => "http_req",
        HttpResp = 3 => "http_resp",
        WsUp = 4 => "ws_up",
        WsDown = 5 => "ws_down",
        Native = 6 => "native",
        Snapshot = 7 => "snapshot",
    }
);

named!(
    /// END `reason` (section 3).
    EndReason {
        Complete = 1 => "complete",
        Interrupted = 2 => "interrupted",
        DecodeError = 3 => "decode_error",
        CutAtSeal = 4 => "cut_at_seal",
        SourceChanged = 5 => "source_changed",
    }
);

named!(
    /// GAP `reason` (section 3). 8 and 9 are reserved and never written by 1.0.
    GapReason {
        MemoryGuard = 1 => "memory_guard",
        RecorderDown = 2 => "recorder_down",
        UnsupportedEncoding = 3 => "unsupported_encoding",
        DecodeFailed = 4 => "decode_failed",
        SourceChanged = 5 => "source_changed",
        TlsRefused = 6 => "tls_refused",
        Pinned = 7 => "pinned",
        CutAtSeal = 10 => "cut_at_seal",
        H2Unsupported = 11 => "h2_unsupported",
        MaskFailed = 12 => "mask_failed",
        UpstreamUntrusted = 13 => "upstream_untrusted",
        ProtocolOther = 14 => "protocol_other",
        AfterStop = 15 => "after_stop",
    }
);

/// WebSocket opcode names for DATA `flags` bits 4 to 7.
pub fn ws_opcode_name(op: u8) -> Option<&'static str> {
    match op {
        0x1 => Some("text"),
        0x2 => Some("binary"),
        0x8 => Some("close"),
        0x9 => Some("ping"),
        0xA => Some("pong"),
        _ => None,
    }
}

/// Memory guard names, the `guard` key of a `memory_guard` GAP tail (section 3).
pub const GUARDS: &[&str] = &[
    "held_bytes",
    "client_hello",
    "http_head",
    "upgrade_copy",
    "hpack_table",
    "decoder_window",
];

/// `conn` meta `reason` for a tunnel (section 4).
pub const TUNNEL_REASONS: &[&str] = &[
    "not_listed",
    "sign_in",
    "tls_refused",
    "pinned",
    "upstream_untrusted",
    "alpn_other",
    "not_tls",
    "hello_too_big",
];

/// The GAP reason a tunnel to a listed host writes at close, by its `conn` meta `reason`
/// (section 4). `not_listed` and `sign_in` are not a loss and get none.
pub fn tunnel_gap(reason: &str) -> Option<GapReason> {
    match reason {
        "tls_refused" => Some(GapReason::TlsRefused),
        "pinned" => Some(GapReason::Pinned),
        "upstream_untrusted" => Some(GapReason::UpstreamUntrusted),
        "alpn_other" | "not_tls" => Some(GapReason::ProtocolOther),
        "hello_too_big" => Some(GapReason::MemoryGuard),
        _ => None,
    }
}

/// Event names, the `ev` key (section 11).
pub const EVENTS: &[&str] = &[
    "start",
    "prev_boot",
    "launch",
    "exit",
    "count",
    "clock",
    "suspend",
    "note",
    "seal",
];

/// The `what` of a `note` event (section 11). Readers show an unknown `what` as is.
pub const NOTES: &[&str] = &[
    "long_tunnel",
    "ca_refused",
    "informational_response",
    "connect_refused",
    "parse_stopped",
    "fd_limit",
    "writer_slow",
    "outlived",
    "native_unreadable",
    "native_symlink",
    "native_missing",
    "native_other_device",
    "snapshot_unstable",
    "final_snapshot_live",
    "local_model",
    "mask_values",
];

/// The facts of the notes other PRDs read (section 11, PRD-00 R3-4), besides `what`.
pub const NOTE_FACTS: &[(&str, &[&str])] = &[
    ("long_tunnel", &["host", "why", "conns", "up", "down", "secs"]),
    ("parse_stopped", &["conn", "reason"]),
    ("connect_refused", &["host", "status"]),
    ("outlived", &["count", "programs"]),
    ("native_missing", &["row"]),
    ("native_other_device", &["row", "path"]),
];

/// `long_tunnel` `why`.
pub const LONG_TUNNEL_WHY: &[&str] = &["stream", "upload"];

/// Seal `reason` (section 11).
pub const SEAL_REASONS: &[&str] = &[
    "harness_exit",
    "deadline",
    "server_stop",
    "seal_command",
    "signal",
    "recovered",
];

/// The keys of each `tunnel_hosts` value in COUNT (section 9). `listed` is the GAP reason name of
/// a listed host's tunnels, null for an unlisted host (PRD-00 R3-4).
pub const TUNNEL_HOST_KEYS: &[&str] = &["conns", "up", "down", "listed"];

/// One line of a part file's flush index (section 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexLine {
    pub kind: IndexKind,
    /// The LINK's `seq`.
    pub seq: u64,
    /// The raw offset just past the LINK record.
    pub raw_end: u64,
    /// The byte length of the part file at that point.
    pub c_end: u64,
    /// The LINK's hash.
    pub hash: [u8; 32],
}

/// `L` a flush point inside an open frame; `E` the frame and the part file are closed; `R` a
/// recovery frame was appended and the part file is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    L,
    E,
    R,
}

impl fmt::Display for IndexLine {
    /// The line without its newline: `L <seq> <raw_end> <c_end> <hash hex>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let k = match self.kind {
            IndexKind::L => 'L',
            IndexKind::E => 'E',
            IndexKind::R => 'R',
        };
        write!(f, "{k} {} {} {} {}", self.seq, self.raw_end, self.c_end, hex(&self.hash))
    }
}

impl IndexLine {
    /// Parses one line (without its newline).
    pub fn parse(line: &str) -> io::Result<Self> {
        let bad = || invalid(format!("bad index line: {line:?}"));
        let mut it = line.split(' ');
        let kind = match it.next() {
            Some("L") => IndexKind::L,
            Some("E") => IndexKind::E,
            Some("R") => IndexKind::R,
            _ => return Err(bad()),
        };
        let mut num = || it.next().and_then(|s| s.parse::<u64>().ok()).ok_or_else(bad);
        let (seq, raw_end, c_end) = (num()?, num()?, num()?);
        let hash = it.next().and_then(unhex32).ok_or_else(bad)?;
        if it.next().is_some() {
            return Err(bad());
        }
        Ok(Self { kind, seq, raw_end, c_end, hash })
    }
}

/// The file name of the part file that starts at `raw_offset`.
pub fn part_name(raw_offset: u64) -> String {
    format!("rec-{raw_offset:020}.gcr.zst")
}

/// The file name of that part file's index.
pub fn index_name(raw_offset: u64) -> String {
    format!("rec-{raw_offset:020}.idx")
}

/// Lowercase hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

pub(crate) fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
