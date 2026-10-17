//! Bounded streaming redaction for protocol-v5 capture.
//!
//! [`StreamRedactor`] turns one producer's ordered [`CaptureFrame`]s into sanitized
//! [`CaptureRecord`]s. Raw bytes never leave it: a [`CaptureRecord::Segment`] carries only
//! redacted bytes (base64 is applied after redaction), and its audit names the replaced
//! ranges in **absolute source offsets of that stream**, never base64 or JSON offsets.
//!
//! # The policy is the shared one
//!
//! What counts as a secret, the span of each rule, first-rule-wins overlap resolution and
//! the `<redacted:KIND>` marker come from [`super::patterns`], [`super::candidates`],
//! [`super::select`] and [`super::marker`] — the code [`super::Redactor::apply`] runs.
//! Every span is found by those regexes, except the fail-closed redactions below. This
//! module adds the answer to "may these bytes leave yet?": a small bounded recognizer per
//! shared literal (`SHAPES`, each pinned to its literal's exact source) reports which trailing
//! bytes could still become, or still extend, a match once more bytes arrive. Those bytes
//! are held. Everything before the earliest such start — moved back to the start of any
//! whole match that crosses it — is a clean cut: it is redacted with the shared regexes
//! and released.
//!
//! The PEM block is the one rule that does not simply lose to an earlier one: `select` lets
//! it absorb the claims inside it and merge with the claims across its edge. The hold already
//! covers that. A block's `BEGIN` is a live candidate from its first byte until its `END`
//! completes it, so nothing inside it is released, and a complete match of any rule that
//! straddles a cut moves the cut back to its start, so a claim across an edge of the block
//! is held with it. A block whose `END` does not start within `super::PEM_BODY_MAX` bytes of its
//! `BEGIN` line is those bytes (to the end of a character they cut, and fewer at the stream's
//! end): its candidate ends there, below the hold, and the shared regexes take the same span.
//! The block's claims are the string interiors of that span (`super::string_pieces`); each
//! carries the whole match, so a cut never falls between them.
//!
//! # Connector `extra_patterns` (D39)
//!
//! [`StreamRedactor::with_extra_patterns`] adds a connector's `[redaction].extra_patterns`
//! after the shared literals, in declaration order — the rule set and priorities
//! `Redactor::for_patterns` applies to a whole record — compiled by the same compiler
//! (`super::compile_extra`). A connector rule is a typed form, not a regex
//! (`goalcup_protocol::harness::RedactionRule`): a JSON field with an object, string or scalar
//! value, or a header prefix. The protocol crate derives each form's regex, and this module
//! has one bounded recognizer per form, derived by hand and reviewed at the hold bound like
//! the shared ones, parameterised only by the field name or prefix bytes. Those names are
//! plain ASCII without a quote, a backslash or a line end (the protocol's grammar), which is
//! what lets a recognizer compare them byte for byte. A form with no recognizer here is
//! refused at construction rather than streamed with a guessed hold. From there an extra rule
//! is one more literal: held, released, followed through a fail-closed redaction and replaced
//! at a discontinuity exactly as a shared one is.
//!
//! # A run's exact secret values (D60)
//!
//! [`StreamRedactor::for_run`] adds, after the connector's rules, one literal rule per form of
//! each of the run's secret values (`super::RunSecrets`): fixed printable-ASCII bytes with at
//! most one line break (LF, CR LF, or either as JSON text) between any two of them, held from
//! the first byte until the last completes the match, and a block in `super::select`, so the
//! claims they overlap join them. The longest wrapped form fits [`MAX_HELD_BYTES`]. Any held
//! byte of one is replaced at a discontinuity.
//!
//! Consequences, each pinned by `tests/it/capture_redaction.rs`,
//! `tests/it/capture_redaction_extra.rs`, `tests/it/capture_redaction_run_secrets.rs` and the unit
//! tests below:
//!
//! * While no stream's held bytes reach [`MAX_HELD_BYTES`], the retained bytes and audit
//!   of a completed stream are exactly `Redactor::apply(whole stream)` — `Redactor::new()`,
//!   or `Redactor::for_patterns` with the same extra patterns — however the stream was
//!   chunked: a secret split across pushes is one secret.
//! * Output never depends on chunking. Segment boundaries and observation intervals may
//!   differ; retained bytes and audit spans may not.
//!
//! # The bound: fail closed
//!
//! A stream holds at most [`MAX_HELD_BYTES`] undecided raw bytes plus the up-to-three bytes
//! of an incomplete UTF-8 sequence. A bounded hold cannot mirror an unbounded regex, so
//! when holding one more unit would exceed that (or a stream has more than
//! `MAX_CANDIDATES` live candidates) the stream fails closed at that content-determined
//! point:
//!
//! * the bytes before the earliest undecided byte — moved back over any match crossing
//!   it — are released with every match of every shared rule over them replaced;
//! * from there every byte is replaced by one marker (named by the earliest undecided
//!   candidate's rule) and one audit span, without holding the bytes. The redaction ends
//!   before a record terminator `\n` that no candidate of any rule that began inside the
//!   range still reaches; otherwise it runs to the stream's `End` or `Gap`. The terminator
//!   and what follows start a fresh window.
//!
//! The stream may therefore replace more than the whole-record policy would — a whole
//! line, or the rest of a single-line body — but never less: no byte that any rule's
//! partial or complete match could cover is released raw. Below the bound nothing changes.
//!
//! A discontinuity inside a stream (a producer `Gap`, or `End` other than `Complete`)
//! cannot complete a held candidate. Held bytes that already look like secret material — a
//! complete vendor prefix (`sk-ant-`, `ghp_`, …), `eyJ`, a `bearer` value, a PEM body, a
//! secret-key JSON value, a shell `token=` value — are replaced by that rule's marker to the
//! cut; other held bytes are decided as end of input. Bytes after a mid-stream gap are
//! scanned without the context before it. [`StreamRedactor::interrupt`] instead discards
//! every held byte, and any redaction in progress, and reports a gap, for a producer the
//! entry owner has lost.
//!
//! This is best-effort body redaction for the built-in rules and the connector rules, not a
//! guarantee that no secret-like content survives.
//!
//! # Ownership
//!
//! One instance per producer. The entry owner drives [`StreamRedactor::flush`] at least
//! every 250 ms, enforces the per-entry stream limit across instances and, when it admits a
//! `Recovery` that names an earlier producer, calls [`StreamRedactor::interrupt`] on that
//! producer's instance with its own clock before appending the new producer's records. No
//! method performs I/O, spawns work, allocates a sequence number or invents an observation
//! time; a segment carries the producer's own observation times.

use super::{Claim, Pattern, candidates, compile_extra, marker, patterns, select};
use crate::DaemonError;
use base64::{Engine, engine::general_purpose::STANDARD};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureRecord, CaptureTime, EndReason, FrameEvent, GapReason,
    MAX_ENVELOPE_BYTES, MAX_METADATA_STRING_BYTES, MAX_OPEN_STREAMS, MAX_SEGMENT_BYTES, StreamKey,
};
use goalcup_protocol::harness::{self, ExtraPattern, RedactionRule};
use goalcup_protocol::redaction::{RedactionAudit, RedactionAuditSpan};
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, OnceLock};

/// The most undecided raw bytes one stream holds. A quarter of the 64 KiB source-chunk
/// bound: long enough for every realistic secret of the built-in rules (an RSA-8192 PEM
/// block is about 6.4 KiB), small enough that 128 open streams hold at most 2 MiB.
pub const MAX_HELD_BYTES: usize = MAX_SEGMENT_BYTES / 4;

// A run secret's form is held from its first byte to its last, line breaks included (D60): the
// longest wrapped form fits the hold, so it never fails closed on its length alone.
const _: () = assert!(goalcup_protocol::run_secret::MAX_WRAPPED_FORM_BYTES <= MAX_HELD_BYTES);

/// Live recognizer states per stream. Equivalent states are merged as they arise, so real
/// input stays in single digits; reaching this fails closed exactly like a full hold.
const MAX_CANDIDATES: usize = 64;
/// Frame observations remembered per stream to time held bytes. Merging keeps the earlier
/// time, so a segment's interval can only widen, never exclude a contributing observation.
const MAX_TIME_MARKS: usize = 8;
/// Room left in [`MAX_ENVELOPE_BYTES`] for the envelope around a segment record (entry id,
/// source, timestamps, link hashes, JSON framing and the trailing newline).
const ENVELOPE_RESERVE_BYTES: usize = 4 * 1024;
const SEGMENT_RECORD_BUDGET: usize = MAX_ENVELOPE_BYTES - ENVELOPE_RESERVE_BYTES;
/// The longest key the shared JSON literal can match, in units: up to 1,000 on each side of
/// an eight-unit key word. A longer `[A-Za-z0-9_\-]` run after a quote is no key, so its
/// candidate ends there instead of holding the run.
const JSON_KEY_MAX_UNITS: u16 = 2008;
/// The most backslashes the generic-secret literals take before a quote: JSON escaped up to
/// three levels deep (`\"`, `\\\"`, seven before the quote).
const MAX_QUOTE_ESCAPES: u8 = 7;
/// The longest local part the shared `email` literal matches, in units (all ASCII).
const EMAIL_LOCAL_MAX: u16 = 1024;

/// Positions of the two `generic_secret_assignment` literals in the shared list, counting
/// `masked_key` at index 2. A rule inserted before them shifts both, which `policy_ready`
/// catches by refusing to stream.
const JSON_LITERAL: usize = 10;
const SHELL_LITERAL: usize = 11;

// ── units ──────────────────────────────────────────────────────────────────────────────

/// One decoding step: a Unicode scalar value. A byte that is not valid UTF-8 is one unit, read
/// as `super::INVALID_BYTE_AS`, because the shared regexes match on `super::match_view`, which
/// reads it so. The regexes run in Unicode mode, so case-insensitive rules also match `ſ`
/// (U+017F) for `s` and the Kelvin sign (U+212A) for `k`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Char(char),
}

/// The unit an invalid byte decodes to: what `super::match_view` reads it as.
const INVALID: Unit = Unit::Char(super::INVALID_BYTE_AS as char);

enum Decoded {
    Unit(Unit, usize),
    Incomplete,
}

/// Decode the unit at the start of `bytes`, or report a valid but incomplete prefix.
fn decode(bytes: &[u8]) -> Decoded {
    let Some(&b0) = bytes.first() else {
        return Decoded::Incomplete;
    };
    let (need, lo, hi): (usize, u8, u8) = match b0 {
        0x00..=0x7F => return Decoded::Unit(Unit::Char(char::from(b0)), 1),
        0xC2..=0xDF => (1, 0x80, 0xBF),
        0xE0 => (2, 0xA0, 0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => (2, 0x80, 0xBF),
        0xED => (2, 0x80, 0x9F),
        0xF0 => (3, 0x90, 0xBF),
        0xF1..=0xF3 => (3, 0x80, 0xBF),
        0xF4 => (3, 0x80, 0x8F),
        _ => return Decoded::Unit(INVALID, 1),
    };
    for i in 1..=need {
        let Some(&b) = bytes.get(i) else {
            return Decoded::Incomplete;
        };
        let (l, h) = if i == 1 { (lo, hi) } else { (0x80, 0xBF) };
        if b < l || b > h {
            return Decoded::Unit(INVALID, 1);
        }
    }
    match bytes
        .get(..=need)
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| s.chars().next())
    {
        Some(c) => Decoded::Unit(Unit::Char(c), need + 1),
        None => Decoded::Unit(INVALID, 1),
    }
}

const LONG_S: char = '\u{017F}';
const KELVIN: char = '\u{212A}';

fn ascii(u: Unit) -> Option<u8> {
    match u {
        Unit::Char(c) if c.is_ascii() => Some(c as u8),
        _ => None,
    }
}

/// The ASCII letter a case-insensitive shared rule compares this unit as.
fn folded(u: Unit) -> Option<u8> {
    match u {
        Unit::Char(LONG_S) => Some(b's'),
        Unit::Char(KELVIN) => Some(b'k'),
        _ => ascii(u).map(|b| b.to_ascii_lowercase()),
    }
}

fn is(u: Unit, b: u8) -> bool {
    ascii(u) == Some(b)
}

fn ascii_where(u: Unit, f: impl Fn(u8) -> bool) -> bool {
    ascii(u).is_some_and(f)
}

fn fold_extra(u: Unit) -> bool {
    matches!(u, Unit::Char(LONG_S | KELVIN))
}

/// `[A-Za-z0-9]`
fn word(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric())
}
/// `[A-Za-z0-9_]`
fn word_underscore(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric() || b == b'_')
}
/// `[A-Za-z0-9_\-]`
fn token(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
/// `[A-Za-z0-9]` under `(?i)`: also `ſ` and the Kelvin sign, which fold to `s` and `k`.
fn word_folded(u: Unit) -> bool {
    word(u) || fold_extra(u)
}
/// `[A-Za-z0-9_\-]` under `(?i)`
fn token_folded(u: Unit) -> bool {
    token(u) || fold_extra(u)
}
/// `[A-Za-z0-9._\-]` under `(?i)`
fn bearer_value(u: Unit) -> bool {
    ascii_where(u, |b| {
        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
    }) || fold_extra(u)
}
/// `[A-Za-z0-9\-]`
fn slack_value(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric() || b == b'-')
}
/// `[0-9A-Z]`
fn aws_value(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_digit() || b.is_ascii_uppercase())
}
/// `[a-f0-9]`
fn hex_lower(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// `[A-Za-z0-9+/]`, the standard base64 alphabet without its pad.
fn base64_char(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}
/// `[A-Za-z0-9._%+\-]`
fn local_part(u: Unit) -> bool {
    ascii_where(u, |b| {
        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
    })
}
/// `[A-Za-z0-9.\-]`
fn domain(u: Unit) -> bool {
    ascii_where(u, |b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}
/// `\s` in Unicode mode: exactly the White_Space property.
fn space(u: Unit) -> bool {
    matches!(u, Unit::Char(c) if c.is_whitespace())
}
/// `[A-Za-z0-9_\-]` under `(?i)`, the characters of a JSON secret key.
fn json_key(u: Unit) -> bool {
    token_folded(u)
}
/// `[^{}]` in Unicode mode: any scalar value but a brace.
fn object_char(u: Unit) -> bool {
    matches!(u, Unit::Char(c) if c != '{' && c != '}')
}
/// `[^"\\]` in Unicode mode: any scalar value but a double quote or a backslash.
fn string_char(u: Unit) -> bool {
    matches!(u, Unit::Char(c) if c != '"' && c != '\\')
}
/// `[^,}\s]` in Unicode mode: any scalar value but a comma, a closing brace or White_Space.
fn credit_char(u: Unit) -> bool {
    matches!(u, Unit::Char(c) if c != ',' && c != '}' && !c.is_whitespace())
}
/// `[a-z0-9-]`
fn header_name(u: Unit) -> bool {
    ascii_where(u, |b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
    })
}
/// `[^\r\n"]` in Unicode mode: any scalar value but CR, LF or a double quote.
fn header_value(u: Unit) -> bool {
    matches!(u, Unit::Char(c) if !matches!(c, '\r' | '\n' | '"'))
}

// ── the streaming form of each shared literal ─────────────────────────────────────────

/// A literal prefix of byte alternatives, then a class run of at least `min` (and, when
/// `max` is set, exactly `max`) units.
struct Prefixed {
    prefix: &'static [&'static [u8]],
    class: fn(Unit) -> bool,
    min: u8,
    max: Option<u8>,
}

enum Form {
    Prefixed(Prefixed),
    /// `(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}`
    MaskedKey,
    Bearer,
    Jwt,
    PrivateKey,
    JsonAssignment,
    ShellAssignment,
    Email,
    /// A connector rule on one JSON field: `KEY\s*:\s*VALUE`, `KEY` the quoted field name
    /// (`harness::RedactionRule::JsonField`).
    Field {
        key: Box<[u8]>,
        value: FieldValue,
    },
    /// A connector rule on one header: `PREFIX[a-z0-9-]+:\s*[^\r\n"]+`
    /// (`harness::RedactionRule::HeaderPrefix`).
    HeaderPrefix {
        prefix: Box<[u8]>,
    },
    /// One form of a run's exact secret value (`super::RunSecrets`, D60): these bytes, all
    /// printable ASCII, with at most one line break in each gap between two of them
    /// (`goalcup_protocol::run_secret::Progress`).
    Literal {
        bytes: Box<[u8]>,
    },
}

/// The value half of a [`Form::Field`] rule; group 1 of its regex is the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldValue {
    /// `(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})`: an object nested at most three deep.
    Object,
    /// `"([^"\\]+)"`: a non-empty string without escapes; the group is its interior.
    String,
    /// `("[^"\\]*"|[^,}\s]+)`: a string without escapes, else a bare token.
    Scalar,
}

struct Shape {
    kind: &'static str,
    /// The exact shared regex source this recognizer was derived from. `policy_ready`
    /// refuses streaming when the compiled shared literal differs, so a rule edit cannot
    /// silently outrun its recognizer.
    source: &'static str,
    form: Form,
}

const fn prefixed(
    kind: &'static str,
    source: &'static str,
    prefix: &'static [&'static [u8]],
    class: fn(Unit) -> bool,
    min: u8,
    max: Option<u8>,
) -> Shape {
    Shape {
        kind,
        source,
        form: Form::Prefixed(Prefixed {
            prefix,
            class,
            min,
            max,
        }),
    }
}

/// The streaming recognizer of each shared literal, in `super::SHARED` order. A
/// recognizer never decides a span below the bound — the regex does — it says how long a
/// candidate must be held, and how long a fail-closed redaction must run. `policy_ready`
/// refuses to run if this list and the compiled shared list ever disagree.
static SHAPES: [Shape; 23] = [
    prefixed(
        "anthropic_key",
        r"sk-ant-[A-Za-z0-9_\-]{20,}",
        &[b"s", b"k", b"-", b"a", b"n", b"t", b"-"],
        token,
        20,
        None,
    ),
    // `sk-(?:proj-)?C{20,}`: `proj-` is itself five class units, so `sk-` then the class.
    prefixed(
        "openai_key",
        r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}",
        &[b"s", b"k", b"-"],
        token,
        20,
        None,
    ),
    // A provider key echoed masked (`key[:8]…key[-4:]` in a request dump, OpenAI's 401 echo
    // with 39 stars, Hermes' `sk-0...cdef`). At most 3 + 8 + 64 + 4 = 79 units (and one more
    // while a three character tail may still take its fourth), so a candidate is held for
    // about a hundred bytes. No assertion, so the recognizer reads nothing outside the match,
    // and it is SOFT: `super::compile` marks the shared literal of kind `super::SOFT_KIND`,
    // and `super::select` lets it lose every overlap and keeps only the parts no hard rule
    // claimed. It never shadows another rule, so it needs no left context either. Its place
    // in this list (index 2) is only a place: a soft rule's priority number decides nothing.
    Shape {
        kind: "masked_key",
        source: r"(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}",
        form: Form::MaskedKey,
    },
    prefixed(
        "github_token",
        r"github_pat_[A-Za-z0-9_]{22,}",
        &[
            b"g", b"i", b"t", b"h", b"u", b"b", b"_", b"p", b"a", b"t", b"_",
        ],
        word_underscore,
        22,
        None,
    ),
    prefixed(
        "github_token",
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        &[b"g", b"h", b"pousr", b"_"],
        word,
        20,
        None,
    ),
    prefixed(
        "aws_access_key",
        r"AKIA[0-9A-Z]{16}",
        &[b"A", b"K", b"I", b"A"],
        aws_value,
        16,
        Some(16),
    ),
    Shape {
        kind: "bearer",
        source: r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
        form: Form::Bearer,
    },
    Shape {
        kind: "jwt",
        source: r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        form: Form::Jwt,
    },
    Shape {
        kind: "private_key",
        source: r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:(?-u:[\x00-\xFF]){0,15872}?-----END [A-Z ]*PRIVATE KEY-----|(?-u:[\x00-\xFF]){0,15872})",
        form: Form::PrivateKey,
    },
    prefixed(
        "slack_token",
        r"xox[baprs]-[A-Za-z0-9\-]{10,}",
        &[b"x", b"o", b"x", b"baprs", b"-"],
        slack_value,
        10,
        None,
    ),
    Shape {
        kind: "generic_secret_assignment",
        source: r#"(?i)\\{0,7}"(?:[A-Za-z0-9_\-]{0,999}[_\-])?(?:api[_-]?key|secret|token|password)(?:[_\-][A-Za-z0-9_\-]{0,999})?\\{0,7}"\s*:\s*\\{0,7}"([^"\\]{16,})\\{0,7}""#,
        form: Form::JsonAssignment,
    },
    Shape {
        kind: "generic_secret_assignment",
        source: r#"(?i)(?:api[_-]?key|secret|token|password)\s*[=:]\s*(?:\\{0,7}['"])?(?:[A-Za-z0-9_\-]{16,})"#,
        form: Form::ShellAssignment,
    },
    Shape {
        kind: "email",
        source: r"[A-Za-z0-9._%+\-]{1,1024}@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
        form: Form::Email,
    },
    prefixed(
        "google_key",
        r"AIza[A-Za-z0-9_\-]{30,}",
        &[b"A", b"I", b"z", b"a"],
        token,
        30,
        None,
    ),
    prefixed(
        "huggingface_token",
        r"hf_[A-Za-z0-9]{20,}",
        &[b"h", b"f", b"_"],
        word,
        20,
        None,
    ),
    prefixed(
        "npm_token",
        r"npm_[A-Za-z0-9]{20,}",
        &[b"n", b"p", b"m", b"_"],
        word,
        20,
        None,
    ),
    prefixed(
        "xai_key",
        r"xai-[A-Za-z0-9]{20,}",
        &[b"x", b"a", b"i", b"-"],
        word,
        20,
        None,
    ),
    prefixed(
        "stripe_key",
        r"[sr]k_live_[A-Za-z0-9]{16,}",
        &[b"sr", b"k", b"_", b"l", b"i", b"v", b"e", b"_"],
        word,
        16,
        None,
    ),
    prefixed(
        "gitlab_token",
        r"glpat-[A-Za-z0-9_\-]{20,}",
        &[b"g", b"l", b"p", b"a", b"t", b"-"],
        token,
        20,
        None,
    ),
    prefixed(
        "digitalocean_token",
        r"dop_v1_[a-f0-9]{32,}",
        &[b"d", b"o", b"p", b"_", b"v", b"1", b"_"],
        hex_lower,
        32,
        None,
    ),
    // The tracer's proxy credential: fixed-length forms, like `aws_access_key`, so a
    // candidate is held for at most 54 units and ends the unit after its last.
    prefixed(
        "proxy_credential",
        r"goalcup:[a-f0-9]{32}",
        &[b"g", b"o", b"a", b"l", b"c", b"u", b"p", b":"],
        hex_lower,
        32,
        Some(32),
    ),
    prefixed(
        "proxy_credential",
        r"goalcup%3[Aa][a-f0-9]{32}",
        &[b"g", b"o", b"a", b"l", b"c", b"u", b"p", b"%", b"3", b"Aa"],
        hex_lower,
        32,
        Some(32),
    ),
    prefixed(
        "proxy_credential",
        r"Z29hbGN1cD[op][A-Za-z0-9+/]{43}",
        &[
            b"Z", b"2", b"9", b"h", b"b", b"G", b"N", b"1", b"c", b"D", b"op",
        ],
        base64_char,
        43,
        Some(43),
    ),
];

/// One connector rule: compiled by the V1 compiler, streamed by the recognizer of its form.
///
/// Each form's character predicates are checked against their regex classes over every
/// scalar value by `extra_predicates_equal_their_regex_classes`. What each recognizer relies
/// on, whatever field name or prefix it is given, each checked against the regex by
/// `extra_recognizers_hold_every_prefix_of_a_match` over the shipped rules and other names:
/// * no rule can match the empty string or start a match with `\n`;
/// * a span group's replacement quoting (`super::span_is_a_json_string_interior`) reads only
///   bytes inside the whole match: group 1 of a JSON-field rule starts after `:` or a space
///   (never a `"`), or is a string interior whose quotes the match contains;
/// * no rule has an assertion, so a match does not depend on bytes outside it.
struct Extra {
    pattern: Pattern,
    form: Form,
}

/// A run secret's form is wiped with its rule (D60); the compiled regex keeps its own copy,
/// which is hygiene and not a boundary (D37).
impl Drop for Extra {
    fn drop(&mut self) {
        if let Form::Literal { bytes } = &mut self.form {
            goalcup_protocol::run_secret::wipe(bytes);
        }
    }
}

/// The rule set one redactor streams: the shared literals (`lit` below [`SHAPES`]'s length,
/// in `super::SHARED` order), then the connector's rules in declaration order. A `lit` is a
/// rule's position in this list, which is also its priority in `super::select`.
#[derive(Default)]
struct Policy {
    extra: Vec<Extra>,
}

impl Policy {
    fn len(&self) -> usize {
        SHAPES.len() + self.extra.len()
    }

    /// The compiled rules in priority order: the order `Redactor::apply` tries them.
    fn rules(&self) -> impl Iterator<Item = &Pattern> {
        patterns()
            .iter()
            .chain(self.extra.iter().map(|e| &e.pattern))
    }

    fn form(&self, lit: usize) -> Option<&Form> {
        match SHAPES.get(lit) {
            Some(shape) => Some(&shape.form),
            None => self.extra.get(lit - SHAPES.len()).map(|e| &e.form),
        }
    }

    /// The audit kind of rule `lit`. Every shared `lit` indexes [`SHAPES`], which
    /// [`policy_ready`] has checked against the compiled list before any frame is admitted.
    fn kind(&self, lit: usize) -> &str {
        match SHAPES.get(lit) {
            Some(shape) => shape.kind,
            None => self
                .extra
                .get(lit - SHAPES.len())
                .map_or("", |e| e.pattern.kind.as_str()),
        }
    }

    /// Compile a connector's `extra_patterns` for streaming, or say why one cannot be.
    ///
    /// Each rule's form maps onto its recognizer, and the regex the form derives must have
    /// the span group that recognizer models. A form this module has no recognizer for (one
    /// the protocol crate added later) is refused, never streamed with a guessed hold.
    fn with_extra(extra: &[ExtraPattern]) -> Result<Policy, DaemonError> {
        let refuse = |p: &ExtraPattern, why: &str| {
            DaemonError::Config(format!("capture redaction pattern {:?}: {why}", p.kind()))
        };
        let mut rules = Vec::with_capacity(extra.len());
        for p in extra {
            let pattern = compile_extra(p).map_err(|reason| refuse(p, &reason))?;
            let (form, span_group) = match p.rule() {
                RedactionRule::JsonField { field, value } => {
                    let value = match value {
                        harness::FieldValue::Object => FieldValue::Object,
                        harness::FieldValue::String => FieldValue::String,
                        harness::FieldValue::Scalar => FieldValue::Scalar,
                        _ => return Err(refuse(p, "no streaming recognizer for this value form")),
                    };
                    let key = format!("\"{field}\"").into_bytes().into_boxed_slice();
                    (Form::Field { key, value }, 1)
                }
                RedactionRule::HeaderPrefix(prefix) => (
                    Form::HeaderPrefix {
                        prefix: prefix.as_bytes().into(),
                    },
                    0,
                ),
                _ => return Err(refuse(p, "no streaming recognizer for this rule form")),
            };
            if pattern.span_group != span_group {
                return Err(refuse(
                    p,
                    "the form's regex and its streaming recognizer disagree on the span",
                ));
            }
            rules.push(Extra { pattern, form });
        }
        Ok(Policy { extra: rules })
    }

    /// The run's exact values after every other rule, one literal rule per form (D60).
    fn with_secrets(mut self, secrets: &super::RunSecrets) -> Result<Policy, DaemonError> {
        for (pattern, bytes) in secrets.patterns().into_iter().zip(secrets.forms()) {
            let bytes = bytes.into();
            self.extra.push(Extra {
                pattern,
                form: Form::Literal { bytes },
            });
        }
        Ok(self)
    }
}

/// True when [`SHAPES`] describes the compiled shared list literal for literal: same
/// count, same kinds and the same regex sources in the same order, and the JSON literal
/// is the one with a group.
fn policy_ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        let compiled = patterns();
        compiled.len() == SHAPES.len()
            && compiled
                .iter()
                .zip(SHAPES.iter())
                .all(|(p, s)| p.kind == s.kind && p.re.as_str() == s.source)
            && compiled
                .get(JSON_LITERAL)
                .is_some_and(|p| p.span_group == 1)
            && compiled
                .get(SHELL_LITERAL)
                .is_some_and(|p| p.span_group == 0)
    })
}

/// Whether a quoted JSON key makes `"KEY": "…"` a secret assignment, asked of the shared
/// JSON literal itself with a fixed sixteen-character value.
fn json_key_is_secret(key: &[u8]) -> bool {
    let Some(p) = patterns().get(JSON_LITERAL) else {
        return true;
    };
    let mut probe = Vec::with_capacity(key.len() + 24);
    probe.push(b'"');
    probe.extend_from_slice(key);
    probe.extend_from_slice(b"\":\"0123456789abcdef\"");
    p.re.find(&probe).is_some_and(|m| m.start() == 0)
}

/// The last up-to-eleven header units of a PEM `BEGIN`/`END` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HeaderTail {
    buf: [u8; 11],
    len: u8,
}

impl HeaderTail {
    fn push(&mut self, b: u8) {
        if (self.len as usize) < self.buf.len() {
            self.buf[self.len as usize] = b;
            self.len += 1;
        } else {
            self.buf.rotate_left(1);
            self.buf[10] = b;
        }
    }
    fn is_private_key(&self) -> bool {
        self.len == 11 && &self.buf == b"PRIVATE KEY"
    }
}

fn pem_header(u: Unit) -> Option<u8> {
    ascii(u).filter(|b| b.is_ascii_uppercase() || *b == b' ')
}

/// Finds the first `-----END [A-Z ]*PRIVATE KEY-----` in a PEM body, one unit at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndLine {
    /// Consecutive dashes seen, capped at five.
    Scan(u8),
    /// Matched this many bytes of `END `.
    Lit(u8),
    Header(HeaderTail),
    /// Dashes after `PRIVATE KEY`.
    Tail(u8),
}

impl EndLine {
    /// True on the unit that completes the END line.
    fn step(&mut self, u: Unit) -> bool {
        let dash = is(u, b'-');
        let restart = if dash {
            EndLine::Scan(1)
        } else {
            EndLine::Scan(0)
        };
        let (next, complete) = match *self {
            EndLine::Scan(d) if dash => (EndLine::Scan((d + 1).min(5)), false),
            EndLine::Scan(d) if d >= 5 && is(u, b'E') => (EndLine::Lit(1), false),
            EndLine::Scan(_) => (EndLine::Scan(0), false),
            EndLine::Lit(p) if b"END ".get(p as usize).is_some_and(|&b| is(u, b)) => {
                if p + 1 == 4 {
                    (EndLine::Header(HeaderTail::default()), false)
                } else {
                    (EndLine::Lit(p + 1), false)
                }
            }
            EndLine::Lit(_) => (restart, false),
            EndLine::Header(mut t) => match pem_header(u) {
                Some(b) => {
                    t.push(b);
                    (EndLine::Header(t), false)
                }
                None if dash && t.is_private_key() => (EndLine::Tail(1), false),
                None => (restart, false),
            },
            EndLine::Tail(n) if dash && n + 1 == 5 => (EndLine::Scan(0), true),
            EndLine::Tail(n) if dash => (EndLine::Tail(n + 1), false),
            EndLine::Tail(_) => (EndLine::Scan(0), false),
        };
        *self = next;
        complete
    }
}

/// What one unit does to a candidate. `Last` keeps a fail-closed redaction from ending
/// on a unit that completes a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// A match that started here can still include this unit and extend past it.
    Live,
    /// This unit completes a match that cannot extend (a PEM END line, a fixed-length
    /// key): the unit belongs to the match.
    Last,
    /// No match that started here includes this unit.
    Dead,
}

impl Step {
    fn of(live: bool) -> Step {
        if live { Step::Live } else { Step::Dead }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pem {
    Begin(u8),
    Header(HeaderTail),
    Dashes(u8),
    Body {
        /// Bytes after the BEGIN line, this unit's included.
        n: u32,
        end: EndLine,
        /// Units of the END line in progress, from its first dash; zero when none is.
        since: u32,
    },
}

/// The most bytes the shared `private_key` literal takes after its BEGIN line
/// (`super::PEM_BODY_MAX`, which says why 15.5 KiB): an END line must start within them, and
/// without one the block is them. A block this long, with a BEGIN and an END line, fits the
/// hold, so a lone BEGIN is decided below it.
///
/// The 512 bytes left are room for a BEGIN line and an END line with real labels (the longest,
/// `-----BEGIN ENCRYPTED PRIVATE KEY-----`, is 37 bytes). The label is `[A-Z ]*`, unbounded: a
/// BEGIN line longer than about 450 bytes holds its block past the hold, and the stream fails
/// closed there. That redacts more than the whole-record policy, never less.
const PEM_BODY_MAX: u32 = super::PEM_BODY_MAX as u32;
const _: () = assert!(super::PEM_BODY_MAX + 512 <= MAX_HELD_BYTES);

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPhase {
    /// Before the key's opening quote.
    Open,
    /// Backslashes (one to seven) before the key's opening quote.
    OpenQuote(u8),
    Key,
    /// Backslashes before the key's closing quote.
    KeyQuote(u8),
    AfterKey,
    AfterColon,
    /// Backslashes before the value's opening quote.
    ValueQuote(u8),
    Value,
    /// Backslashes after the value: only a quote completes the match.
    ValueEnd(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Json {
    phase: JsonPhase,
    /// The key so far; empty after the key.
    key: Vec<u8>,
    /// Units of the key so far, at most [`JSON_KEY_MAX_UNITS`].
    key_units: u16,
    value_start: u64,
    /// Value characters, capped at sixteen.
    n: u8,
}

const SHELL_WORDS: [&[u8]; 4] = [b"secret", b"token", b"password", b"api"];
const API: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Shell {
    Word {
        which: u8,
        pos: u8,
    },
    ApiSep,
    ApiKey(u8),
    BeforeSep,
    AfterSep,
    /// Backslashes (one to seven) before the optional quote.
    Escape(u8),
    Quoted,
    /// Value characters, capped at sixteen.
    Value(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Email {
    /// Local-part units, at most [`EMAIL_LOCAL_MAX`]: past that the window slides.
    Local(u16),
    Domain {
        /// At least one domain unit seen (capped).
        some: bool,
        /// Letters since a dot that had a domain unit before it, capped at two.
        after_dot: Option<u8>,
        tld: bool,
    },
}

/// The recognizer of the shared `masked_key` literal:
/// `(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}`.
///
/// The four separators start with four different units (`.`, `…`, `\`, `*`, none a run or tail unit), so the
/// walk needs one state at a time. A match is `sk-`, one to eight run units, a separator of
/// one to sixty-four units and three or four tail units: 79 units at most, which is the
/// hold's bound for this rule. A third tail unit leaves the match complete AND extendable,
/// so the candidate stays live until the next unit decides it (a fourth tail unit is
/// taken, anything else ends the match before it, as for every open-ended rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Masked {
    /// This many units of `sk-` matched.
    Prefix(u8),
    /// Run units (`[A-Za-z0-9-]`) after `sk-`, one to eight; the separator may start from one.
    Run(u8),
    /// Of the three dots.
    Dots(u8),
    /// Of the six units of `\u2026`, one to five.
    Escape(u8),
    /// Stars, one to sixty-four; the tail may start from three.
    Stars(u8),
    /// Of the four tail units, zero right after a separator.
    Tail(u8),
}

const MASK_ESCAPE: &[u8; 6] = b"\\u2026";

impl Masked {
    fn step(&mut self, u: Unit) -> Step {
        let next = match *self {
            Masked::Prefix(p) => {
                let wanted = match p {
                    0 => folded(u) == Some(b's'),
                    1 => folded(u) == Some(b'k'),
                    _ => is(u, b'-'),
                };
                if !wanted {
                    return Step::Dead;
                }
                if p == 2 {
                    Masked::Run(0)
                } else {
                    Masked::Prefix(p + 1)
                }
            }
            Masked::Run(n) if word_folded(u) || is(u, b'-') => {
                if n == 8 {
                    return Step::Dead;
                }
                Masked::Run(n + 1)
            }
            Masked::Run(n) if n >= 1 => match u {
                Unit::Char('.') => Masked::Dots(1),
                Unit::Char('\u{2026}') => Masked::Tail(0),
                Unit::Char('\\') => Masked::Escape(1),
                Unit::Char('*') => Masked::Stars(1),
                _ => return Step::Dead,
            },
            Masked::Run(_) => return Step::Dead,
            Masked::Dots(k) if is(u, b'.') && k < 3 => Masked::Dots(k + 1),
            Masked::Dots(3) if word_folded(u) => Masked::Tail(1),
            Masked::Dots(_) => return Step::Dead,
            Masked::Escape(p) => {
                let wanted = MASK_ESCAPE.get(p as usize).map(u8::to_ascii_lowercase);
                if folded(u) != wanted {
                    return Step::Dead;
                }
                if p as usize + 1 == MASK_ESCAPE.len() {
                    Masked::Tail(0)
                } else {
                    Masked::Escape(p + 1)
                }
            }
            Masked::Stars(k) if is(u, b'*') && k < 64 => Masked::Stars(k + 1),
            Masked::Stars(k) if k >= 3 && word_folded(u) => Masked::Tail(1),
            Masked::Stars(_) => return Step::Dead,
            Masked::Tail(m) if word_folded(u) => {
                // The fourth tail unit completes a match that cannot extend.
                if m == 3 {
                    return Step::Last;
                }
                Masked::Tail(m + 1)
            }
            // Past three tail units a non-tail unit ends a complete match before it; before
            // that it ends nothing.
            Masked::Tail(_) => return Step::Dead,
        };
        *self = next;
        Step::Live
    }

    /// The separator is complete: held bytes from the start already look like a masked key.
    fn past_separator(&self) -> bool {
        matches!(
            self,
            Masked::Dots(3) | Masked::Stars(3..=64) | Masked::Tail(_)
        )
    }
}

/// A recognizer's position. Counts are capped at the largest threshold that matters, so
/// two candidates of one literal in equal states have identical futures and the later
/// one can be dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Walk {
    Prefixed {
        pos: u8,
        count: u8,
    },
    Masked(Masked),
    Bearer {
        pos: u8,
        spaced: bool,
        count: u8,
    },
    Jwt {
        pos: u8,
        seg: u8,
        count: u8,
    },
    Pem(Pem),
    Json(Json),
    Shell(Shell),
    Email(Email),
    Field(Field),
    Header(Header),
    /// How far a [`Form::Literal`] matched, a line break in progress included.
    Literal(goalcup_protocol::run_secret::Progress),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldPhase {
    /// This many bytes of the quoted key matched.
    Key(u8),
    /// The first `\s*`.
    BeforeColon,
    /// The second `\s*`.
    AfterColon,
    /// Inside the object at this depth, one to three.
    Object(u8),
    /// Inside the string; whether it has a character yet.
    String(bool),
    /// `"[^"\\]*"` still open, and `[^,}\s]+` still running: the regex's two alternatives,
    /// followed together until the first one completes (leftmost-first then takes it).
    Scalar { quoted: bool, bare: bool },
}

/// A [`Form::Field`] candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    phase: FieldPhase,
    /// Where the rule's span (the value, capture group 1) starts, once the value has.
    value_start: u64,
}

impl Field {
    /// `KEY\s*:\s*VALUE`; `off..off + len` is this unit.
    fn step(&mut self, key: &[u8], value: FieldValue, u: Unit, off: u64, len: usize) -> Step {
        let next = match self.phase {
            FieldPhase::Key(p) => {
                if !key.get(p as usize).is_some_and(|&b| is(u, b)) {
                    return Step::Dead;
                }
                if p as usize + 1 == key.len() {
                    FieldPhase::BeforeColon
                } else {
                    FieldPhase::Key(p + 1)
                }
            }
            FieldPhase::BeforeColon if space(u) => FieldPhase::BeforeColon,
            FieldPhase::BeforeColon if is(u, b':') => FieldPhase::AfterColon,
            FieldPhase::BeforeColon => return Step::Dead,
            FieldPhase::AfterColon if space(u) => FieldPhase::AfterColon,
            FieldPhase::AfterColon => match value {
                FieldValue::Object if is(u, b'{') => {
                    self.value_start = off;
                    FieldPhase::Object(1)
                }
                FieldValue::String if is(u, b'"') => {
                    self.value_start = off + len as u64;
                    FieldPhase::String(false)
                }
                // `"` opens the first alternative and is also a character of the second.
                FieldValue::Scalar if credit_char(u) => {
                    self.value_start = off;
                    FieldPhase::Scalar {
                        quoted: is(u, b'"'),
                        bare: true,
                    }
                }
                _ => return Step::Dead,
            },
            // `[^{}]` matches every scalar value but a brace, `\n` included, and never an
            // invalid byte.
            FieldPhase::Object(depth) => match u {
                Unit::Char('{') if depth < 3 => FieldPhase::Object(depth + 1),
                Unit::Char('}') if depth == 1 => return Step::Last,
                Unit::Char('}') => FieldPhase::Object(depth - 1),
                _ if object_char(u) => FieldPhase::Object(depth),
                _ => return Step::Dead,
            },
            FieldPhase::String(some) => match u {
                Unit::Char('"') if some => return Step::Last,
                _ if string_char(u) => FieldPhase::String(true),
                _ => return Step::Dead,
            },
            FieldPhase::Scalar { quoted, bare } => {
                // The first alternative completes: leftmost-first takes it, whatever the
                // second could still have matched.
                if quoted && is(u, b'"') {
                    return Step::Last;
                }
                let quoted = quoted && string_char(u);
                let bare = bare && credit_char(u);
                if !quoted && !bare {
                    return Step::Dead;
                }
                FieldPhase::Scalar { quoted, bare }
            }
        };
        self.phase = next;
        Step::Live
    }
}

/// A [`Form::HeaderPrefix`] candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    /// This many bytes of the prefix matched.
    Lit(u8),
    /// In `[a-z0-9-]+`; whether it has a character yet.
    Name(bool),
    /// After the colon: whether `\s*` can still be running, and whether `[^\r\n"]+` has
    /// started. A space other than CR or LF can be either, so both can hold at once.
    Value { spaces: bool, value: bool },
}

impl Header {
    /// `PREFIX[a-z0-9-]+:\s*[^\r\n"]+`
    fn step(&mut self, prefix: &[u8], u: Unit) -> Step {
        let next = match *self {
            Header::Lit(p) => {
                if !prefix.get(p as usize).is_some_and(|&b| is(u, b)) {
                    return Step::Dead;
                }
                if p as usize + 1 == prefix.len() {
                    Header::Name(false)
                } else {
                    Header::Lit(p + 1)
                }
            }
            Header::Name(_) if header_name(u) => Header::Name(true),
            Header::Name(true) if is(u, b':') => Header::Value {
                spaces: true,
                value: false,
            },
            Header::Name(_) => return Step::Dead,
            Header::Value { spaces, value } => {
                let value = (spaces || value) && header_value(u);
                let spaces = spaces && space(u);
                if !spaces && !value {
                    return Step::Dead;
                }
                Header::Value { spaces, value }
            }
        };
        *self = next;
        Step::Live
    }
}

impl Prefixed {
    fn step(&self, pos: &mut u8, count: &mut u8, u: Unit) -> bool {
        if let Some(set) = self.prefix.get(*pos as usize) {
            *pos += 1;
            return ascii(u).is_some_and(|b| set.contains(&b));
        }
        if !(self.class)(u) {
            return false;
        }
        match self.max {
            // A fixed-length match cannot extend once complete.
            Some(max) => {
                *count += 1;
                *count < max
            }
            None => {
                *count = (*count + 1).min(self.min);
                true
            }
        }
    }
}

/// `(?i)bearer\s+[A-Za-z0-9._\-]{16,}`
fn step_bearer(pos: &mut u8, spaced: &mut bool, count: &mut u8, u: Unit) -> bool {
    if let Some(&b) = b"bearer".get(*pos as usize) {
        *pos += 1;
        return folded(u) == Some(b);
    }
    if *count == 0 && space(u) {
        *spaced = true;
        return true;
    }
    if (*count > 0 || *spaced) && bearer_value(u) {
        *count = (*count + 1).min(16);
        return true;
    }
    false
}

impl Pem {
    /// `-----BEGIN [A-Z ]*PRIVATE KEY-----(?:(?-u:[\x00-\xFF]){0,15872}?-----END [A-Z ]*PRIVATE
    /// KEY-----|(?-u:[\x00-\xFF]){0,15872})`: to an END line that starts within
    /// [`PEM_BODY_MAX`] bytes, else those bytes and the rest of a character they cut (fewer at
    /// the end of the stream, which is decided there with every other held byte by the shared
    /// regexes). `len` is this unit's length in bytes.
    fn step(&mut self, u: Unit, len: usize) -> Step {
        let next = match self {
            Pem::Begin(p) => {
                if !b"-----BEGIN ".get(*p as usize).is_some_and(|&b| is(u, b)) {
                    return Step::Dead;
                }
                if *p + 1 == 11 {
                    Pem::Header(HeaderTail::default())
                } else {
                    Pem::Begin(*p + 1)
                }
            }
            Pem::Header(t) => match pem_header(u) {
                Some(b) => {
                    t.push(b);
                    return Step::Live;
                }
                None if is(u, b'-') && t.is_private_key() => Pem::Dashes(1),
                None => return Step::Dead,
            },
            Pem::Dashes(n) => {
                if !is(u, b'-') {
                    return Step::Dead;
                }
                if *n + 1 == 5 {
                    Pem::Body {
                        n: 0,
                        end: EndLine::Scan(0),
                        since: 0,
                    }
                } else {
                    Pem::Dashes(*n + 1)
                }
            }
            // The body is any bytes; a completed END line finishes the lazy arm with this unit.
            Pem::Body { n, end, since } => {
                if end.step(u) {
                    return Step::Last;
                }
                let before = *n;
                *n += len as u32;
                *since = match end {
                    EndLine::Scan(d) => u32::from(*d),
                    EndLine::Lit(p) => 5 + u32::from(*p),
                    EndLine::Header(_) | EndLine::Tail(_) => *since + 1,
                };
                // Live while an END line may still start within the bound (the next unit starts
                // at byte `n`; an END line is ASCII, so `since` is bytes too), or one that did is
                // still open. A unit the bound cuts ends the block with it; otherwise the bounded
                // arm ended before this unit.
                return if *n <= PEM_BODY_MAX || (*since > 0 && *n - *since <= PEM_BODY_MAX) {
                    Step::Live
                } else if before < PEM_BODY_MAX {
                    Step::Last
                } else {
                    Step::Dead
                };
            }
        };
        *self = next;
        Step::Live
    }
}

impl Json {
    /// `(?i)\\{0,7}"KEY\\{0,7}"\s*:\s*\\{0,7}"([^"\\]{16,})\\{0,7}"`; `after` is the offset
    /// after this unit.
    fn step(&mut self, u: Unit, after: u64) -> bool {
        // One more backslash before a quote, while at most seven have been seen.
        let more = |k: u8| (is(u, b'\\') && k < MAX_QUOTE_ESCAPES).then_some(k + 1);
        match self.phase {
            JsonPhase::Open if is(u, b'\\') => {
                self.phase = JsonPhase::OpenQuote(1);
                true
            }
            JsonPhase::OpenQuote(k) if is(u, b'\\') => {
                self.phase = JsonPhase::OpenQuote(k + 1);
                more(k).is_some()
            }
            JsonPhase::Open | JsonPhase::OpenQuote(_) => {
                self.phase = JsonPhase::Key;
                is(u, b'"')
            }
            JsonPhase::Key if json_key(u) => {
                if self.key_units == JSON_KEY_MAX_UNITS {
                    return false;
                }
                let Unit::Char(c) = u;
                let mut buf = [0u8; 4];
                self.key
                    .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                self.key_units += 1;
                true
            }
            JsonPhase::Key if is(u, b'\\') => {
                self.phase = JsonPhase::KeyQuote(1);
                !self.key.is_empty()
            }
            JsonPhase::KeyQuote(k) if is(u, b'\\') => {
                self.phase = JsonPhase::KeyQuote(k + 1);
                more(k).is_some()
            }
            JsonPhase::Key | JsonPhase::KeyQuote(_) => {
                if !is(u, b'"') {
                    return false;
                }
                let secret = !self.key.is_empty() && json_key_is_secret(&self.key);
                self.key = Vec::new();
                self.key_units = 0;
                self.phase = JsonPhase::AfterKey;
                secret
            }
            JsonPhase::AfterKey if is(u, b':') => {
                self.phase = JsonPhase::AfterColon;
                true
            }
            JsonPhase::AfterColon if is(u, b'\\') => {
                self.phase = JsonPhase::ValueQuote(1);
                true
            }
            JsonPhase::ValueQuote(k) if is(u, b'\\') => {
                self.phase = JsonPhase::ValueQuote(k + 1);
                more(k).is_some()
            }
            JsonPhase::AfterColon | JsonPhase::ValueQuote(_) if is(u, b'"') => {
                self.phase = JsonPhase::Value;
                self.value_start = after;
                true
            }
            JsonPhase::AfterKey | JsonPhase::AfterColon => space(u),
            JsonPhase::ValueQuote(_) => false,
            JsonPhase::Value => match u {
                // The closing quote may be escaped: hold the backslash until its quote.
                Unit::Char('\\') => {
                    self.phase = JsonPhase::ValueEnd(1);
                    true
                }
                Unit::Char('"') => false,
                Unit::Char(_) => {
                    self.n = (self.n + 1).min(16);
                    true
                }
            },
            // The quote completes the match, which the regex then sees; anything else but one
            // more backslash ends it.
            JsonPhase::ValueEnd(k) => match more(k) {
                Some(k) => {
                    self.phase = JsonPhase::ValueEnd(k);
                    true
                }
                None => false,
            },
        }
    }
}

impl Shell {
    /// `(?i)(?:api[_-]?key|secret|token|password)\s*[=:]\s*(?:\\{0,7}['"])?[A-Za-z0-9_\-]{16,}`
    fn step(&mut self, u: Unit) -> bool {
        let next = match *self {
            Shell::Word { which, pos } => {
                let Some(f) = folded(u) else { return false };
                let which = if pos == 0 {
                    match SHELL_WORDS.iter().position(|w| w.first() == Some(&f)) {
                        Some(i) => i as u8,
                        None => return false,
                    }
                } else if SHELL_WORDS
                    .get(which as usize)
                    .and_then(|w| w.get(pos as usize))
                    == Some(&f)
                {
                    which
                } else {
                    return false;
                };
                let len = SHELL_WORDS.get(which as usize).map_or(0, |w| w.len());
                if pos as usize + 1 < len {
                    Shell::Word {
                        which,
                        pos: pos + 1,
                    }
                } else if which == API {
                    Shell::ApiSep
                } else {
                    Shell::BeforeSep
                }
            }
            Shell::ApiSep => match folded(u) {
                Some(b'_' | b'-') => Shell::ApiKey(0),
                Some(b'k') => Shell::ApiKey(1),
                _ => return false,
            },
            Shell::ApiKey(p) => {
                if folded(u) != b"key".get(p as usize).copied() {
                    return false;
                }
                if p + 1 == 3 {
                    Shell::BeforeSep
                } else {
                    Shell::ApiKey(p + 1)
                }
            }
            Shell::BeforeSep if is(u, b'=') || is(u, b':') => Shell::AfterSep,
            Shell::BeforeSep | Shell::AfterSep if space(u) => return true,
            Shell::AfterSep if is(u, b'\\') => Shell::Escape(1),
            Shell::Escape(k) if is(u, b'\\') && k < MAX_QUOTE_ESCAPES => Shell::Escape(k + 1),
            Shell::AfterSep | Shell::Escape(_) if is(u, b'\'') || is(u, b'"') => Shell::Quoted,
            Shell::AfterSep | Shell::Quoted if token_folded(u) => Shell::Value(1),
            Shell::Value(n) if token_folded(u) => Shell::Value((n + 1).min(16)),
            _ => return false,
        };
        *self = next;
        true
    }
}

impl Email {
    /// `[A-Za-z0-9._%+\-]{1,1024}@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}`. The domain stays live
    /// while its run continues, since a later `.xx` can still extend the match.
    fn step(&mut self, u: Unit) -> bool {
        match self {
            Email::Local(_) if is(u, b'@') => {
                *self = Email::Domain {
                    some: false,
                    after_dot: None,
                    tld: false,
                };
                true
            }
            Email::Local(n) => {
                *n = (*n + 1).min(EMAIL_LOCAL_MAX);
                local_part(u)
            }
            Email::Domain {
                some,
                after_dot,
                tld,
            } => {
                if !domain(u) {
                    return false;
                }
                if is(u, b'.') {
                    *after_dot = if *some { Some(0) } else { None };
                } else if ascii_where(u, |b| b.is_ascii_alphabetic()) {
                    *after_dot = after_dot.map(|k| (k + 1).min(2));
                    *tld |= *after_dot == Some(2);
                } else {
                    *after_dot = None;
                }
                *some = true;
                true
            }
        }
    }
}

/// A JWT segment step: `[A-Za-z0-9_\-]` extends the segment; `.` starts the next one only
/// after ten units and only twice.
fn jwt_step(seg: &mut u8, count: &mut u8, u: Unit) -> bool {
    if token(u) {
        *count = (*count + 1).min(10);
        true
    } else if is(u, b'.') && *seg < 3 && *count >= 10 {
        *seg += 1;
        *count = 0;
        true
    } else {
        false
    }
}

/// One live candidate: a literal, where it started, and its recognizer.
#[derive(Debug, Clone)]
struct Candidate {
    lit: usize,
    start: u64,
    walk: Walk,
}

impl Candidate {
    /// The candidate a unit starts for literal `lit`, before that unit is stepped.
    fn spawn(policy: &Policy, lit: usize, u: Unit, prev_local: bool) -> Option<Walk> {
        let form = policy.form(lit)?;
        let starts = match form {
            Form::Prefixed(p) => {
                ascii(u).is_some_and(|b| p.prefix.first().is_some_and(|s| s.contains(&b)))
            }
            Form::MaskedKey => folded(u) == Some(b's'),
            Form::Bearer => folded(u) == Some(b'b'),
            Form::Jwt => is(u, b'e'),
            Form::PrivateKey => is(u, b'-'),
            Form::JsonAssignment => is(u, b'"') || is(u, b'\\'),
            Form::ShellAssignment => matches!(folded(u), Some(b's' | b't' | b'p' | b'a')),
            // The leftmost-first email match starts where its local-part run starts, or
            // 1,024 units before its `@` in a longer run: one candidate per run, whose
            // start slides with the run (see `Candidate::step`).
            Form::Email => local_part(u) && !prev_local,
            Form::Field { key, .. } => key.first().is_some_and(|&b| is(u, b)),
            Form::HeaderPrefix { prefix } => prefix.first().is_some_and(|&b| is(u, b)),
            Form::Literal { bytes } => bytes.first().is_some_and(|&b| is(u, b)),
        };
        if !starts {
            return None;
        }
        Some(match form {
            Form::Prefixed(_) => Walk::Prefixed { pos: 0, count: 0 },
            Form::MaskedKey => Walk::Masked(Masked::Prefix(0)),
            Form::Bearer => Walk::Bearer {
                pos: 0,
                spaced: false,
                count: 0,
            },
            Form::Jwt => Walk::Jwt {
                pos: 0,
                seg: 0,
                count: 0,
            },
            Form::PrivateKey => Walk::Pem(Pem::Begin(0)),
            Form::JsonAssignment => Walk::Json(Json {
                phase: JsonPhase::Open,
                key: Vec::new(),
                key_units: 0,
                value_start: 0,
                n: 0,
            }),
            Form::ShellAssignment => Walk::Shell(Shell::Word { which: 0, pos: 0 }),
            Form::Email => Walk::Email(Email::Local(0)),
            Form::Field { .. } => Walk::Field(Field {
                phase: FieldPhase::Key(0),
                value_start: 0,
            }),
            Form::HeaderPrefix { .. } => Walk::Header(Header::Lit(0)),
            Form::Literal { .. } => Walk::Literal(Default::default()),
        })
    }

    /// Feed one unit at `off..off + len`. Anything but [`Step::Live`] ends the candidate:
    /// the regex then sees its complete match, if any.
    fn step(&mut self, policy: &Policy, u: Unit, off: u64, len: usize) -> Step {
        let Some(form) = policy.form(self.lit) else {
            return Step::Dead;
        };
        Step::of(match (&mut self.walk, form) {
            (Walk::Prefixed { pos, count }, Form::Prefixed(p)) => {
                let live = p.step(pos, count, u);
                if !live && p.max.is_some_and(|max| *count == max) {
                    return Step::Last;
                }
                live
            }
            (Walk::Masked(masked), _) => return masked.step(u),
            (Walk::Bearer { pos, spaced, count }, _) => step_bearer(pos, spaced, count, u),
            (Walk::Jwt { pos, seg, count }, _) => {
                if let Some(&b) = b"eyJ".get(*pos as usize) {
                    *pos += 1;
                    if *pos == 3 {
                        *seg = 1;
                    }
                    return Step::of(is(u, b));
                }
                jwt_step(seg, count, u)
            }
            (Walk::Pem(pem), _) => return pem.step(u, len),
            (Walk::Json(json), _) => json.step(u, off + len as u64),
            (Walk::Shell(shell), _) => shell.step(u),
            (Walk::Email(email), _) => {
                // A full window that takes one more local-part unit drops its first: the
                // local part is ASCII, so the match can start no earlier than one byte on.
                if *email == Email::Local(EMAIL_LOCAL_MAX) && local_part(u) {
                    self.start += 1;
                }
                email.step(u)
            }
            (Walk::Field(field), Form::Field { key, value }) => {
                return field.step(key, *value, u, off, len);
            }
            (Walk::Header(header), Form::HeaderPrefix { prefix }) => {
                return header.step(prefix, u);
            }
            // A literal cannot extend once complete: its last byte ends it.
            (Walk::Literal(progress), Form::Literal { bytes }) => {
                use goalcup_protocol::run_secret::Advance;
                let Some(b) = ascii(u) else {
                    return Step::Dead;
                };
                return match progress.step(bytes, b) {
                    Advance::Live => Step::Live,
                    Advance::Complete => Step::Last,
                    Advance::Dead => Step::Dead,
                };
            }
            _ => false,
        })
    }

    /// Where held bytes of this candidate already look like secret material, for a
    /// discontinuity that can no longer complete it. `tail` is where the stream's
    /// undecoded bytes (an incomplete UTF-8 sequence) start, when it has any.
    fn secret_from(&self, policy: &Policy, tail: Option<u64>) -> Option<u64> {
        let form = policy.form(self.lit)?;
        match (&self.walk, form) {
            (Walk::Prefixed { pos, .. }, Form::Prefixed(p)) => {
                (*pos as usize >= p.prefix.len()).then_some(self.start)
            }
            (Walk::Masked(masked), _) => masked.past_separator().then_some(self.start),
            (Walk::Bearer { count, .. }, _) => (*count >= 1).then_some(self.start),
            (Walk::Jwt { pos, .. }, _) => (*pos == 3).then_some(self.start),
            (Walk::Pem(Pem::Body { .. }), _) => Some(self.start),
            (Walk::Json(json), _) => {
                matches!(json.phase, JsonPhase::Value | JsonPhase::ValueEnd(_))
                    .then_some(json.value_start)
            }
            (Walk::Shell(Shell::Value(_)), _) => Some(self.start),
            // A connector field's value has begun: from there it is the rule's span.
            (Walk::Field(field), Form::Field { value, .. }) => match field.phase {
                FieldPhase::Object(_) | FieldPhase::String(_) | FieldPhase::Scalar { .. } => {
                    Some(field.value_start)
                }
                // A bare scalar value can begin with a multi-byte character. Cut inside its
                // encoding, the undecoded bytes are the value's first bytes: V1 on the same
                // prefix keeps them, but they are part of a value that has begun.
                FieldPhase::AfterColon if *value == FieldValue::Scalar => tail,
                FieldPhase::Key(_) | FieldPhase::BeforeColon | FieldPhase::AfterColon => None,
            },
            // A header's name and colon are complete: the whole match is the span.
            (Walk::Header(Header::Value { .. }), _) => Some(self.start),
            // Any held byte of a run's secret value is part of it: none is released at a cut.
            (Walk::Literal(progress), _) => (progress.matched() >= 1).then_some(self.start),
            _ => None,
        }
    }
}

// ── one stream's undecided bytes ──────────────────────────────────────────────────────

/// Decided output, in source order, not yet placed in a segment.
enum Piece {
    Raw {
        offset: u64,
        bytes: Vec<u8>,
        first: CaptureTime,
    },
    /// Named by rule `lit` of the stream's [`Policy`].
    Mark {
        start: u64,
        end: u64,
        lit: usize,
        quote: bool,
        first: CaptureTime,
    },
}

/// A decided redaction in absolute source offsets. `priority` is the rule's `lit`, which
/// also names it.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
    priority: usize,
    quote: bool,
}

impl Span {
    fn of(claim: &Claim<'_>, base: u64) -> Span {
        Span {
            start: base + claim.start as u64,
            end: base + claim.end as u64,
            priority: claim.priority,
            quote: claim.quote,
        }
    }
}

/// Overlapping spans become one, named by their highest-priority part.
fn union(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|s| (s.start, s.priority));
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        if s.start >= s.end {
            continue;
        }
        match out.last_mut() {
            Some(last) if s.start < last.end => {
                last.end = last.end.max(s.end);
                last.quote = false;
                last.priority = last.priority.min(s.priority);
            }
            _ => out.push(s),
        }
    }
    out
}

/// Candidates are in start order; keep the earliest of each equivalent state.
fn dedup(candidates: &mut Vec<Candidate>) {
    let mut i = 1;
    while i < candidates.len() {
        let (earlier, rest) = candidates.split_at(i);
        let c = &rest[0];
        if earlier.iter().any(|k| k.lit == c.lit && k.walk == c.walk) {
            candidates.remove(i);
        } else {
            i += 1;
        }
    }
}

/// A fail-closed redaction in progress: every byte from `start` is replaced until a
/// record terminator (`\n`) arrives at which no candidate that began inside the range is
/// still live, or the stream ends. Only recognizer state is kept, never the bytes.
struct Redacting {
    start: u64,
    /// The literal named by the marker: the earliest undecided candidate's.
    lit: usize,
    /// Candidates of every literal that began inside the range and are still live.
    live: Vec<Candidate>,
    /// Too many live candidates to follow: then only the end of the stream ends it.
    saturated: bool,
    /// The end of a block released in part ([`OpenBlock`]): the redaction does not end before.
    until: u64,
    first: CaptureTime,
}

/// A lone BEGIN's window the stream released in part (H-20 re-review 2). Its BEGIN has left
/// the hold, so the regexes no longer find it; its rest, from `held_start` to `end`, is claimed
/// from the quote state there, as the whole-record policy claims it (`super::block_pieces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenBlock {
    /// Where its `BEGIN` was: the cut point is `origin + super::PEM_SLICE`.
    origin: u64,
    end: u64,
    /// The quote state at `held_start`.
    quotes: super::Quotes,
}

/// The shared literal of the PEM block rule.
fn pem_literal() -> Option<usize> {
    SHAPES
        .iter()
        .position(|shape| matches!(shape.form, Form::PrivateKey))
}

/// The bounded redaction state of one stream: held raw bytes (`held_start ..
/// decoded_end`), an incomplete UTF-8 tail after them and live candidates — or a
/// fail-closed redaction in progress (then nothing is held).
struct Scanner {
    /// The producer's rule set, shared by every stream of the producer.
    policy: Arc<Policy>,
    held: Vec<u8>,
    held_start: u64,
    decoded_end: u64,
    tail: Vec<u8>,
    prev_local: bool,
    candidates: Vec<Candidate>,
    redacting: Option<Redacting>,
    open_block: Option<OpenBlock>,
    marks: VecDeque<(u64, CaptureTime)>,
}

impl Scanner {
    fn new(offset: u64, policy: Arc<Policy>) -> Scanner {
        Scanner {
            policy,
            held: Vec::new(),
            held_start: offset,
            decoded_end: offset,
            tail: Vec::new(),
            prev_local: false,
            candidates: Vec::new(),
            redacting: None,
            open_block: None,
            marks: VecDeque::new(),
        }
    }

    /// The source offset of the next byte this scanner expects.
    fn end_offset(&self) -> u64 {
        self.decoded_end + self.tail.len() as u64
    }

    fn mark(&mut self, offset: u64, time: &CaptureTime) {
        self.marks.push_back((offset, time.clone()));
        if self.marks.len() > MAX_TIME_MARKS {
            // Merge the second-oldest into the oldest: its bytes take the earlier time.
            self.marks.remove(1);
        }
    }

    /// The observation of the frame that delivered the byte at `offset`, or an earlier one.
    fn time_at(&self, offset: u64, fallback: &CaptureTime) -> CaptureTime {
        self.marks
            .iter()
            .rev()
            .find(|(start, _)| *start <= offset)
            .or(self.marks.front())
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| fallback.clone())
    }

    fn trim_marks(&mut self, offset: u64) {
        while self.marks.len() >= 2 && self.marks.get(1).is_some_and(|(s, _)| *s <= offset) {
            self.marks.pop_front();
        }
    }

    fn feed(&mut self, data: &[u8], now: &CaptureTime, out: &mut Vec<Piece>) {
        let joined;
        let buf: &[u8] = if self.tail.is_empty() {
            data
        } else {
            joined = [self.tail.as_slice(), data].concat();
            &joined
        };
        let base = self.decoded_end;
        self.tail.clear();
        let policy = Arc::clone(&self.policy);
        let mut pos = 0usize;
        while pos < buf.len() {
            let (u, len) = match decode(&buf[pos..]) {
                Decoded::Incomplete => {
                    self.tail = buf[pos..].to_vec();
                    break;
                }
                Decoded::Unit(u, len) => (u, len),
            };
            let off = base + pos as u64;
            if self.redacting.is_some() {
                if self.redact_unit(&policy, u, off, len, out) {
                    pos += len;
                }
                // Otherwise the terminator is dispatched again, outside the redaction.
                continue;
            }
            if self.held.len() + len > MAX_HELD_BYTES {
                self.release(now, out);
                if self.held.len() + len > MAX_HELD_BYTES {
                    self.fail_closed(now, out);
                    continue;
                }
            }
            self.held.extend_from_slice(&buf[pos..pos + len]);
            self.decoded_end = off + len as u64;
            self.step_unit(&policy, u, off, len);
            pos += len;
            if self.candidates.len() > MAX_CANDIDATES {
                self.fail_closed(now, out);
            }
        }
        if self.redacting.is_none() {
            self.release(now, out);
        }
    }

    /// One unit during a fail-closed redaction. True when the unit is redacted; false
    /// when the redaction ended before it (a terminator no live candidate includes).
    fn redact_unit(
        &mut self,
        policy: &Policy,
        u: Unit,
        off: u64,
        len: usize,
        out: &mut Vec<Piece>,
    ) -> bool {
        let Some(red) = self.redacting.as_mut() else {
            return false;
        };
        let mut included = false;
        red.live.retain_mut(|c| {
            let step = c.step(policy, u, off, len);
            included |= step != Step::Dead;
            step == Step::Live
        });
        if is(u, b'\n') && !included && !red.saturated && off >= red.until {
            // No rule's match that began inside the range can include this terminator,
            // and none can begin with it: what follows starts a fresh window.
            self.end_redaction(off, out);
            return false;
        }
        for lit in 0..policy.len() {
            if let Some(walk) = Candidate::spawn(policy, lit, u, self.prev_local) {
                let mut c = Candidate {
                    lit,
                    start: off,
                    walk,
                };
                if c.step(policy, u, off, len) == Step::Live {
                    red.live.push(c);
                }
            }
        }
        dedup(&mut red.live);
        if red.live.len() > MAX_CANDIDATES {
            red.live.truncate(MAX_CANDIDATES);
            red.saturated = true;
        }
        self.prev_local = local_part(u);
        self.decoded_end = off + len as u64;
        true
    }

    fn step_unit(&mut self, policy: &Policy, u: Unit, off: u64, len: usize) {
        self.candidates
            .retain_mut(|c| c.step(policy, u, off, len) == Step::Live);
        for lit in 0..policy.len() {
            if let Some(walk) = Candidate::spawn(policy, lit, u, self.prev_local) {
                let mut c = Candidate {
                    lit,
                    start: off,
                    walk,
                };
                if c.step(policy, u, off, len) == Step::Live {
                    self.candidates.push(c);
                }
            }
        }
        self.prev_local = local_part(u);
        dedup(&mut self.candidates);
    }

    /// Release `held[..n]` with `spans` (absolute, overlapping allowed) replaced.
    fn emit(&mut self, n: usize, spans: Vec<Span>, now: &CaptureTime, out: &mut Vec<Piece>) {
        let base = self.held_start;
        let upto = base + n as u64;
        let mut at = base;
        for s in union(spans) {
            let (start, end) = (s.start.max(at), s.end.min(upto));
            if start >= end {
                continue;
            }
            if start > at {
                out.push(Piece::Raw {
                    offset: at,
                    bytes: self.held[(at - base) as usize..(start - base) as usize].to_vec(),
                    first: self.time_at(at, now),
                });
            }
            out.push(Piece::Mark {
                start,
                end,
                lit: s.priority,
                quote: s.quote,
                first: self.time_at(start, now),
            });
            at = end;
        }
        if upto > at {
            out.push(Piece::Raw {
                offset: at,
                bytes: self.held[(at - base) as usize..n].to_vec(),
                first: self.time_at(at, now),
            });
        }
        self.held.drain(..n);
        self.held_start = upto;
        self.trim_marks(upto);
    }

    /// Every match of every rule in `held[..n]`, and the pieces of a block released in part
    /// ([`OpenBlock`]) there; the index where those pieces begin.
    fn found<'p>(&self, policy: &'p Policy, n: usize) -> (Vec<Claim<'p>>, usize) {
        let mut found = candidates(policy.rules(), &self.held[..n]);
        let first = found.len();
        let (Some(ob), Some(lit)) = (self.open_block, pem_literal()) else {
            return (found, first);
        };
        let Some(kind) = policy.rules().nth(lit).map(|p| p.kind.as_str()) else {
            return (found, first);
        };
        let base = self.held_start;
        let view = super::match_view(&self.held[..n]);
        let whole = (self.open_end(ob) - base) as usize;
        let cut = (ob.origin + super::PEM_SLICE as u64)
            .checked_sub(base)
            .map(|c| c as usize);
        for (start, end) in super::block_pieces(&view, 0, whole.min(n), ob.quotes, cut) {
            found.push(Claim {
                priority: lit,
                start,
                end,
                whole_start: 0,
                whole_end: whole,
                kind,
                quote: false,
                soft: false,
                block: true,
                bounded: true,
            });
        }
        (found, first)
    }

    /// Where a block released in part ends: its own end, or a `BEGIN` held inside it, where the
    /// next block starts (`super::pem_matches`).
    fn open_end(&self, ob: OpenBlock) -> u64 {
        let base = self.held_start;
        let to = ((ob.end - base) as usize).min(self.held.len());
        super::first_begin(&super::match_view(&self.held), 0, to)
            .map_or(ob.end, |at| base + at as u64)
    }

    /// Release every byte before the earliest live candidate: a clean cut, where no match
    /// of any rule can cross whatever arrives next.
    ///
    /// A lone BEGIN's window that is decided (its walk has ended, or it is a block already
    /// released in part) does not hold the cut back to its start. The cut moves only to the
    /// start of the piece it falls in, so no claim is split, and the rest of the window is
    /// carried as an [`OpenBlock`]. Otherwise a run that starts late in the window and crosses
    /// its end would hold the whole window past the hold and fail the stream closed.
    fn release(&mut self, now: &CaptureTime, out: &mut Vec<Piece>) {
        if self.held.is_empty() {
            return;
        }
        let policy = Arc::clone(&self.policy);
        let end = self.decoded_end;
        let base = self.held_start;
        let mut cut = self.candidates.iter().map(|c| c.start).min().unwrap_or(end);
        if cut == base {
            return;
        }
        // A complete match that straddles the cut may still lose its claim to a
        // higher-priority candidate that completes later: hold it too. With no live candidate
        // and no open block, no match can straddle the end of the hold: skip the search.
        let (found, open_from) = if cut < end || self.open_block.is_some() {
            self.found(&policy, self.held.len())
        } else {
            (Vec::new(), 0)
        };
        let pem = pem_literal();
        let decided = |i: usize, m: &Claim<'_>| {
            m.block
                && m.bounded
                && (i >= open_from
                    || !self
                        .candidates
                        .iter()
                        .any(|c| Some(c.lit) == pem && c.start == base + m.whole_start as u64))
        };
        let mut carried: Option<usize>;
        loop {
            let mut moved = false;
            carried = None;
            for (i, m) in found.iter().enumerate() {
                let (s, t) = (base + m.whole_start as u64, base + m.whole_end as u64);
                if !(s < cut && cut < t) {
                    continue;
                }
                if decided(i, m) {
                    carried = Some(i);
                    let inside = found.iter().find(|p| {
                        p.whole_start == m.whole_start
                            && base + (p.start as u64) < cut
                            && cut < base + (p.end as u64)
                    });
                    if let Some(piece) = inside {
                        cut = base + piece.start as u64;
                        moved = true;
                    }
                } else {
                    cut = s;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        if cut == base {
            return;
        }
        let n = (cut - base) as usize;
        // The decided window the cut falls in goes on as the open block, with the quote state
        // at the cut. An open block the cut does not fall in has ended.
        let carry = carried.map(|i| (i, &found[i])).map(|(i, m)| {
            let view = super::match_view(&self.held[..n]);
            let (origin, quotes) = match self.open_block.filter(|_| i >= open_from) {
                Some(ob) => (ob.origin, ob.quotes.after(&view)),
                None => (
                    base + m.whole_start as u64,
                    super::Quotes::START.after(&view[m.whole_start..]),
                ),
            };
            OpenBlock {
                origin,
                end: base + m.whole_end as u64,
                quotes,
            }
        });
        let spans = select(self.found(&policy, n).0)
            .iter()
            .map(|m| Span::of(m, base))
            .collect();
        self.emit(n, spans, now, out);
        self.open_block = carry;
    }

    /// The hold is full and some candidate is still undecided: fail closed. The bytes
    /// before the earliest undecided byte leave with every match over them replaced; from
    /// there everything is redacted until a record terminator no candidate reaches.
    fn fail_closed(&mut self, now: &CaptureTime, out: &mut Vec<Piece>) {
        let base = self.held_start;
        let policy = Arc::clone(&self.policy);
        let until = self.open_block.map_or(0, |ob| self.open_end(ob));
        let Some((mut start, lit)) = self.candidates.iter().map(|c| (c.start, c.lit)).min() else {
            let n = self.held.len();
            let spans = select(self.found(&policy, n).0)
                .iter()
                .map(|m| Span::of(m, base))
                .collect();
            return self.emit(n, spans, now, out);
        };
        // Every match in the hold, not only this window's claims: a match the window
        // cannot see yet (a PEM block whose END is still to come) can change which ones the
        // whole-record policy keeps. A match crossing the start moves it back.
        let (raw, _) = self.found(&policy, self.held.len());
        loop {
            let mut moved = false;
            for m in &raw {
                let (s, w) = (base + m.start as u64, base + m.whole_end as u64);
                if s < start && start < w {
                    start = s;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        let before: Vec<Span> = raw
            .iter()
            .filter(|m| base + (m.end as u64) <= start)
            .map(|m| Span::of(m, base))
            .collect();
        self.emit((start - base) as usize, before, now, out);
        self.open_block = None;
        self.redacting = Some(Redacting {
            start,
            lit,
            live: std::mem::take(&mut self.candidates),
            saturated: false,
            until,
            first: self.time_at(start, now),
        });
        self.held.clear();
        self.held_start = self.decoded_end;
    }

    fn end_redaction(&mut self, end: u64, out: &mut Vec<Piece>) {
        if let Some(red) = self.redacting.take() {
            out.push(Piece::Mark {
                start: red.start,
                end,
                lit: red.lit,
                quote: false,
                first: red.first,
            });
        }
        self.held.clear();
        self.held_start = end;
        self.decoded_end = end;
        self.candidates.clear();
        self.open_block = None;
        self.prev_local = false;
        self.trim_marks(end);
    }

    /// The stream is complete: every held byte is decided as end of input.
    fn finish_complete(&mut self, now: &CaptureTime, out: &mut Vec<Piece>) {
        let end = self.end_offset();
        if self.redacting.is_some() {
            self.tail.clear();
            self.decoded_end = end;
            self.end_redaction(end, out);
            return;
        }
        self.held.append(&mut self.tail);
        self.decoded_end = end;
        let base = self.held_start;
        let policy = Arc::clone(&self.policy);
        let n = self.held.len();
        let spans = select(self.found(&policy, n).0)
            .iter()
            .map(|m| Span::of(m, base))
            .collect();
        self.emit(n, spans, now, out);
        self.candidates.clear();
        self.open_block = None;
    }

    /// A discontinuity: nothing after it can complete a held candidate. Held bytes that
    /// already look like secret material are replaced; the rest is decided as end of input.
    fn finish_cut(&mut self, now: &CaptureTime, out: &mut Vec<Piece>) {
        if self.redacting.is_some() {
            return self.finish_complete(now, out);
        }
        let end = self.end_offset();
        let tail = (!self.tail.is_empty()).then_some(self.decoded_end);
        self.held.append(&mut self.tail);
        self.decoded_end = end;
        let base = self.held_start;
        let policy = Arc::clone(&self.policy);
        let found = select(self.found(&policy, self.held.len()).0);
        self.open_block = None;
        let mut pick: Option<(u64, usize)> = None;
        for c in &self.candidates {
            if let Some(x) = c.secret_from(&policy, tail)
                && pick.is_none_or(|p| (x, c.lit) < p)
            {
                pick = Some((x, c.lit));
            }
        }
        self.candidates.clear();
        let mut spans: Vec<Span> = found.iter().map(|m| Span::of(m, base)).collect();
        if let Some((x, lit)) = pick {
            spans.push(Span {
                start: x,
                end,
                priority: lit,
                quote: false,
            });
        }
        let n = self.held.len();
        self.emit(n, spans, now, out);
    }

    /// The producer is gone: drop every undecided byte, and a redaction in progress with
    /// it, so a native resume from the durable end re-reads the secret from its start.
    /// Nothing is emitted.
    fn discard(&mut self) {
        self.redacting = None;
        self.open_block = None;
        self.held.clear();
        self.tail.clear();
        self.candidates.clear();
    }

    #[cfg(test)]
    fn held_bytes(&self) -> usize {
        self.held.len() + self.tail.len()
    }
}

// ── segments ─────────────────────────────────────────────────────────────────────────

fn json_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX / 4, |v| v.len())
}

fn b64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

/// A segment being filled. `fixed` is its serialized size with no data, no spans and no
/// `last` time; the estimate adds those exactly.
struct Segment {
    source_offset: u64,
    source_len: u64,
    data: Vec<u8>,
    spans: Vec<RedactionAuditSpan>,
    spans_json: usize,
    first: CaptureTime,
    last: CaptureTime,
    fixed: usize,
}

impl Segment {
    fn new(key: &StreamKey, part: u64, offset: u64, first: &CaptureTime) -> Segment {
        let skeleton = CaptureRecord::Segment {
            stream: key.clone(),
            part,
            source_offset: offset,
            source_len: u64::MAX,
            first: first.clone(),
            last: first.clone(),
            data_b64: String::new(),
            audit: RedactionAudit { spans: Vec::new() },
        };
        Segment {
            source_offset: offset,
            source_len: 0,
            data: Vec::new(),
            spans: Vec::new(),
            spans_json: 0,
            first: first.clone(),
            last: first.clone(),
            fixed: json_len(&skeleton).saturating_sub(json_len(first)),
        }
    }

    fn estimate(&self, data: usize, spans_json: usize, last: &CaptureTime) -> usize {
        self.fixed + json_len(last) + b64_len(data) + spans_json
    }

    fn fits(&self, data: usize, spans_json: usize, last: &CaptureTime) -> bool {
        data <= MAX_SEGMENT_BYTES && self.estimate(data, spans_json, last) <= SEGMENT_RECORD_BUDGET
    }

    /// Raw bytes that still fit with `last` as the segment's last time.
    fn raw_room(&self, last: &CaptureTime) -> usize {
        let used = self.fixed + json_len(last) + self.spans_json;
        let by_budget = SEGMENT_RECORD_BUDGET.saturating_sub(used) / 4 * 3;
        by_budget
            .min(MAX_SEGMENT_BYTES)
            .saturating_sub(self.data.len())
    }

    fn into_record(self, key: &StreamKey, part: u64) -> CaptureRecord {
        CaptureRecord::Segment {
            stream: key.clone(),
            part,
            source_offset: self.source_offset,
            source_len: self.source_len,
            first: self.first,
            last: self.last,
            data_b64: STANDARD.encode(&self.data),
            audit: RedactionAudit { spans: self.spans },
        }
    }
}

// ── the producer ─────────────────────────────────────────────────────────────────────

struct StreamState {
    key: StreamKey,
    /// Source offset the next `Bytes` must start at.
    next_offset: u64,
    /// After a gap of unknown size the next `Bytes` may start at any later offset.
    offset_unknown: bool,
    scanner: Scanner,
    segment: Option<Segment>,
    next_part: u64,
    /// The producer's latest observation on this stream: the `last` time of output the
    /// owner forces out with its own clock (`interrupt`), which never stamps a segment.
    last_seen: CaptureTime,
}

impl StreamState {
    fn close_segment(&mut self, out: &mut Vec<CaptureRecord>) {
        if let Some(seg) = self.segment.take() {
            out.push(seg.into_record(&self.key, self.next_part));
            self.next_part += 1;
        }
    }

    fn place(&mut self, pieces: Vec<Piece>, last: &CaptureTime, out: &mut Vec<CaptureRecord>) {
        for piece in pieces {
            match piece {
                Piece::Raw {
                    mut offset,
                    bytes,
                    first,
                } => {
                    let mut rest = bytes.as_slice();
                    while !rest.is_empty() {
                        let seg = match self.segment.as_mut() {
                            Some(seg) => seg,
                            None => {
                                self.segment =
                                    Some(Segment::new(&self.key, self.next_part, offset, &first));
                                continue;
                            }
                        };
                        let mut n = seg.raw_room(last).min(rest.len());
                        if n == 0 {
                            if !seg.data.is_empty() || !seg.spans.is_empty() {
                                self.close_segment(out);
                                continue;
                            }
                            // An empty segment always takes at least one byte.
                            n = 1;
                        }
                        seg.data.extend_from_slice(&rest[..n]);
                        seg.source_len += n as u64;
                        seg.last = last.clone();
                        rest = &rest[n..];
                        offset += n as u64;
                    }
                }
                Piece::Mark {
                    start,
                    end,
                    lit,
                    quote,
                    first,
                } => {
                    let policy = Arc::clone(&self.scanner.policy);
                    let kind = policy.kind(lit);
                    let text = marker(kind, quote);
                    let span = RedactionAuditSpan {
                        kind: kind.to_string(),
                        start,
                        end,
                    };
                    let span_json = json_len(&span) + 1;
                    let fits = self.segment.as_ref().is_some_and(|seg| {
                        seg.fits(
                            seg.data.len() + text.len(),
                            seg.spans_json + span_json,
                            last,
                        )
                    });
                    if !fits {
                        self.close_segment(out);
                        self.segment = Some(Segment::new(&self.key, self.next_part, start, &first));
                    }
                    if let Some(seg) = self.segment.as_mut() {
                        seg.data.extend_from_slice(text.as_bytes());
                        seg.spans.push(span);
                        seg.spans_json += span_json;
                        seg.source_len += end - start;
                        seg.last = last.clone();
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    Active,
    /// A frame was refused; decided output may still be flushed, nothing more is accepted.
    Failed,
    Interrupted,
}

/// The per-producer streaming redactor. See the module documentation.
pub struct StreamRedactor {
    policy: Arc<Policy>,
    producer: Option<String>,
    next_seq: u64,
    high_water: u64,
    streams: BTreeMap<u64, StreamState>,
    life: Life,
}

impl Default for StreamRedactor {
    fn default() -> Self {
        StreamRedactor::new()
    }
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// Redact one whitelisted metadata string, keeping it within the string bound.
fn scrub(policy: &Policy, value: &str) -> String {
    let claims = select(candidates(policy.rules(), value.as_bytes()));
    let mut out = String::with_capacity(value.len());
    let mut cut = 0usize;
    for claim in claims {
        out.push_str(value.get(cut..claim.start).unwrap_or(""));
        out.push_str(&marker(claim.kind, claim.quote));
        cut = claim.end;
    }
    out.push_str(value.get(cut..).unwrap_or(""));
    if out.len() > MAX_METADATA_STRING_BYTES {
        let mut at = MAX_METADATA_STRING_BYTES;
        while !out.is_char_boundary(at) {
            at -= 1;
        }
        out.truncate(at);
    }
    out
}

/// The retained form of a stream's metadata: the query and fragment never leave, and
/// every string goes through the producer's rules. Transport identity is structural and
/// kept.
fn sanitize(policy: &Policy, metadata: CaptureMeta) -> CaptureMeta {
    let clean = |value: &str| scrub(policy, value);
    let path = metadata.path.map(|p| {
        let end = p.find(['?', '#']).unwrap_or(p.len());
        clean(&p[..end])
    });
    CaptureMeta {
        wire: metadata.wire,
        label: clean(&metadata.label),
        protocol_hint: metadata.protocol_hint.as_deref().map(clean),
        method: metadata.method.as_deref().map(clean),
        host: metadata.host.as_deref().map(clean),
        path,
        status: metadata.status,
        content_type: metadata.content_type.as_deref().map(clean),
    }
}

impl StreamRedactor {
    /// The shared rules and nothing else.
    pub fn new() -> Self {
        StreamRedactor::with_policy(Policy::default())
    }

    /// The shared rules plus a connector's `[redaction].extra_patterns`, applied after them
    /// in declaration order: below the hold bound a completed stream's retained bytes and audit
    /// are exactly `Redactor::for_patterns(id, extra).apply(whole stream)`, and past it the
    /// connector's rules fail closed like the shared ones (see the module documentation).
    ///
    /// Each rule goes through the V1 compiler — a rule `Redactor::for_patterns` refuses is
    /// refused here for the same reason — and then its form must be one this module has a
    /// recognizer for. Anything else is a `DaemonError::Config` naming the kind, never a rule
    /// silently dropped or streamed with a guessed hold. The kind, field and prefix grammar is
    /// held by `ExtraPattern` itself. An empty list is [`Self::new`].
    pub fn with_extra_patterns(extra: &[ExtraPattern]) -> Result<Self, DaemonError> {
        Ok(StreamRedactor::with_policy(Policy::with_extra(extra)?))
    }

    /// [`Self::with_extra_patterns`] plus the run's exact secret values, applied last (D60):
    /// below the hold bound a completed stream's retained bytes and audit are exactly
    /// `Redactor::for_run(id, extra, secrets).apply(whole stream)`. A value's recognizer holds
    /// from its first byte until it completes, so a value split across pushes is one value, and
    /// a cut never releases a held byte of one.
    pub fn for_run(
        extra: &[ExtraPattern],
        secrets: &super::RunSecrets,
    ) -> Result<Self, DaemonError> {
        Ok(StreamRedactor::with_policy(
            Policy::with_extra(extra)?.with_secrets(secrets)?,
        ))
    }

    fn with_policy(policy: Policy) -> Self {
        StreamRedactor {
            policy: Arc::new(policy),
            producer: None,
            next_seq: 0,
            high_water: 0,
            streams: BTreeMap::new(),
            life: Life::Active,
        }
    }

    /// Admit the producer's next frame and return the records it decides.
    ///
    /// The frame must be this producer's (the first frame binds the instance), carry the
    /// next sequence number (from zero, controls included), satisfy
    /// [`CaptureFrame::validate`] and follow the stream lifecycle: `Open` with a stream ID
    /// above every earlier one and at most [`MAX_OPEN_STREAMS`] open; `Bytes` at the
    /// stream's next source offset; nothing after `End`; `Recovery` only as the first
    /// frame. A refused frame is an error that returns nothing and refuses every later
    /// frame; decided output stays available to [`Self::flush`] and
    /// [`Self::interrupt`], which the owner then calls to account the loss.
    ///
    /// `Bytes` output is coalesced per stream: a segment is returned when full, and the
    /// rest by the next [`Self::flush`], `End`, `Gap` or control frame. A control frame
    /// first returns every stream's decided segment, so retained order follows observation
    /// order for decided bytes.
    pub fn push(&mut self, frame: CaptureFrame) -> io::Result<Vec<CaptureRecord>> {
        match self.life {
            Life::Active => {}
            Life::Failed => return Err(invalid("capture producer already refused a frame")),
            Life::Interrupted => return Err(invalid("capture producer was interrupted")),
        }
        match self.admit(frame) {
            Ok(records) => Ok(records),
            Err(e) => {
                self.life = Life::Failed;
                Err(e)
            }
        }
    }

    fn admit(&mut self, frame: CaptureFrame) -> io::Result<Vec<CaptureRecord>> {
        if !policy_ready() {
            return Err(invalid(
                "streaming redaction policy does not match the shared rules",
            ));
        }
        if let Some(producer) = &self.producer
            && *producer != frame.stream.producer
        {
            return Err(invalid("frame belongs to a different producer"));
        }
        if frame.seq != self.next_seq {
            return Err(invalid("frame sequence is not the producer's next"));
        }
        let CaptureFrame {
            stream,
            seq,
            observed,
            event,
        } = frame;
        let event = match event {
            FrameEvent::Open { source, metadata } => FrameEvent::Open {
                source,
                metadata: sanitize(&self.policy, metadata),
            },
            other => other,
        };
        let frame = CaptureFrame {
            stream,
            seq,
            observed,
            event,
        };
        frame
            .validate()
            .map_err(|_| invalid("frame breaks a capture bound or representation rule"))?;
        let CaptureFrame {
            stream: key,
            observed,
            event,
            ..
        } = frame;
        let producer = key.producer.clone();

        let mut out = Vec::new();
        match event {
            FrameEvent::Connection { connection, event } => {
                self.flush_all(&mut out);
                out.push(CaptureRecord::Connection {
                    producer: key.producer.clone(),
                    connection,
                    observed,
                    event,
                });
            }
            FrameEvent::Loss { reason } => {
                // The producer's own observation of an uncaptured relay, on its own clock: decided
                // segments first (as for a connection event), then the stream-less gap. D3: the
                // daemon records it as observed and judges nothing.
                self.flush_all(&mut out);
                out.push(CaptureRecord::Gap {
                    stream: None,
                    observed,
                    reason,
                    lost_bytes: None,
                });
            }
            FrameEvent::Recovery {
                previous_producer,
                reason,
            } => {
                if seq != 0 {
                    return Err(invalid("recovery must be a producer's first frame"));
                }
                out.push(CaptureRecord::Recovery {
                    previous_producer,
                    producer: key.producer.clone(),
                    observed,
                    reason,
                });
            }
            FrameEvent::Open { source, metadata } => {
                if key.stream <= self.high_water {
                    return Err(invalid("stream ID is not above every earlier stream"));
                }
                if self.streams.len() >= MAX_OPEN_STREAMS {
                    return Err(invalid("too many open capture streams"));
                }
                self.high_water = key.stream;
                // A stream's first bytes may start at any offset (a native source can be
                // opened mid-file); after that they are contiguous.
                self.streams.insert(
                    key.stream,
                    StreamState {
                        key: key.clone(),
                        next_offset: 0,
                        offset_unknown: true,
                        scanner: Scanner::new(0, Arc::clone(&self.policy)),
                        segment: None,
                        next_part: 0,
                        last_seen: observed.clone(),
                    },
                );
                out.push(CaptureRecord::Open {
                    stream: key.clone(),
                    observed,
                    source,
                    metadata,
                });
            }
            FrameEvent::Bytes { offset, data } => {
                let state = self.stream_mut(key.stream)?;
                if state.offset_unknown {
                    if offset < state.next_offset {
                        return Err(invalid("bytes restart before the stream's last offset"));
                    }
                    state.scanner = Scanner::new(offset, Arc::clone(&state.scanner.policy));
                    state.offset_unknown = false;
                } else if offset != state.next_offset {
                    return Err(invalid("bytes do not start at the stream's next offset"));
                }
                state.next_offset = offset
                    .checked_add(data.len() as u64)
                    .ok_or_else(|| invalid("bytes overflow the stream offset"))?;
                state.last_seen = observed.clone();
                let mut pieces = Vec::new();
                state.scanner.mark(offset, &observed);
                state.scanner.feed(&data, &observed, &mut pieces);
                state.place(pieces, &observed, &mut out);
            }
            FrameEvent::End { reason } => {
                let mut state = self
                    .streams
                    .remove(&key.stream)
                    .ok_or_else(|| self.unknown(key.stream))?;
                let mut pieces = Vec::new();
                match reason {
                    EndReason::Complete => state.scanner.finish_complete(&observed, &mut pieces),
                    EndReason::Interrupted | EndReason::DecodeError => {
                        state.scanner.finish_cut(&observed, &mut pieces)
                    }
                }
                state.place(pieces, &observed, &mut out);
                state.close_segment(&mut out);
                out.push(CaptureRecord::End {
                    stream: key,
                    observed,
                    reason,
                });
            }
            FrameEvent::Gap { reason, lost_bytes } => {
                let state = self.stream_mut(key.stream)?;
                let resume = match lost_bytes {
                    Some(n) => Some(
                        state
                            .next_offset
                            .checked_add(n)
                            .ok_or_else(|| invalid("gap overflows the stream offset"))?,
                    ),
                    None => None,
                };
                let mut pieces = Vec::new();
                state.scanner.finish_cut(&observed, &mut pieces);
                state.place(pieces, &observed, &mut out);
                state.close_segment(&mut out);
                match resume {
                    Some(offset) => {
                        state.next_offset = offset;
                        state.scanner = Scanner::new(offset, Arc::clone(&state.scanner.policy));
                    }
                    None => state.offset_unknown = true,
                }
                out.push(CaptureRecord::Gap {
                    stream: Some(key),
                    observed,
                    reason,
                    lost_bytes,
                });
            }
        }
        // The first admitted frame binds the instance to its producer.
        self.producer.get_or_insert(producer);
        self.next_seq += 1;
        Ok(out)
    }

    fn unknown(&self, stream: u64) -> io::Error {
        if stream != 0 && stream <= self.high_water {
            invalid("stream has already ended")
        } else {
            invalid("stream was never opened")
        }
    }

    fn stream_mut(&mut self, stream: u64) -> io::Result<&mut StreamState> {
        if !self.streams.contains_key(&stream) {
            return Err(self.unknown(stream));
        }
        self.streams
            .get_mut(&stream)
            .ok_or_else(|| invalid("stream was never opened"))
    }

    fn flush_all(&mut self, out: &mut Vec<CaptureRecord>) {
        for state in self.streams.values_mut() {
            state.close_segment(out);
        }
    }

    /// Return every stream's decided bytes as segments now. Held undecided bytes stay
    /// held. The owner calls this at least every 250 ms; `now` is its tick instant. It is
    /// checked, but no record carries it: a segment keeps the observation interval of the
    /// frames that delivered and decided its bytes. No sequence number is consumed.
    pub fn flush(&mut self, now: CaptureTime) -> io::Result<Vec<CaptureRecord>> {
        now.validate().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "flush time is not a capture time",
            )
        })?;
        let mut out = Vec::new();
        if self.life != Life::Interrupted {
            self.flush_all(&mut out);
        }
        Ok(out)
    }

    /// The producer is gone (the owner admitted a `Recovery` naming it, or lost it). Every
    /// open stream returns its decided bytes, discards every undecided byte and gets a
    /// `Gap` with unknown loss, stamped with the owner's `observed` clock; with no stream
    /// open, one producer-level `Gap`. The instance then refuses every frame. A second
    /// call returns nothing.
    pub fn interrupt(&mut self, observed: CaptureTime, reason: GapReason) -> Vec<CaptureRecord> {
        if self.life == Life::Interrupted {
            return Vec::new();
        }
        self.life = Life::Interrupted;
        let mut out = Vec::new();
        let streams = std::mem::take(&mut self.streams);
        let any = !streams.is_empty();
        for (_, mut state) in streams {
            state.scanner.discard();
            state.close_segment(&mut out);
            out.push(CaptureRecord::Gap {
                stream: Some(state.key),
                observed: observed.clone(),
                reason,
                lost_bytes: None,
            });
        }
        if !any {
            out.push(CaptureRecord::Gap {
                stream: None,
                observed,
                reason,
                lost_bytes: None,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    const PIECES: &[&[u8]] = &[
        b"sk-ant-",
        b"sk-",
        b"proj-",
        b"ghp_",
        b"AKIA",
        b"bearer",
        b"Bearer",
        b"eyJ",
        b".",
        b"-----BEGIN ",
        b"RSA ",
        b"PRIVATE KEY",
        b"-----",
        b"-----END ",
        b"\"",
        b":",
        b"=",
        b"api_key",
        b"x-api-key",
        b"token",
        b"secret",
        b"password",
        b"@",
        b".com",
        b"xoxb-",
        b"AIza",
        b"hf_",
        b"npm_",
        b"xai-",
        b"sk_live_",
        b"glpat-",
        b"dop_v1_",
        b" ",
        b"\n",
        b"\t",
        b"'",
        b"\\",
        b"\xff",
        "\u{17f}".as_bytes(),
        "\u{212a}".as_bytes(),
        "\u{a0}".as_bytes(),
        b"abcdefghij",
        b"ABCDEFGHIJ",
        b"0123456789",
        b"deadbeef",
        b"_",
        b"-",
        b"%",
        b"+",
        b"\"api_key\":\"",
        // Escaped inside a string (I-4).
        b"\\\"api_key\\\": \\\"",
        b"\\\"",
        b"password: \\\"",
        b"\\\\\\\"token\\\\\\\":\\\\\\\"",
        b"\"token\" : \"",
        "\"Secret\":\u{a0}\"".as_bytes(),
        b"token = '",
        b"password: ",
        b"Bearer  ",
        b"abc def ghi jkl mno",
        b"x@y.co",
        b"eyJabcdefghij.",
        b"-----BEGIN RSA PRIVATE KEY-----\n",
        b"-----EN",
        b"END-",
        // Masked keys (S9): the prefix, both ends, and every separator and near miss.
        b"sk-",
        b"SK-ab",
        b"sk-abcde",
        b"***",
        b"********",
        b"...",
        b"..",
        "\u{2026}".as_bytes(),
        b"\\u2026",
        b"\\u202",
        b"wxyz",
        b"wxy",
        b"sk-0",
        b"sk-or-v1",
        b"-",
        b"_",
        b"a",
        "\u{e9}".as_bytes(),
        "\u{300}".as_bytes(),
        b"\xff",
        b"****************************************",
        // The proxy credential (H8): all three forms whole, cut short, and their prefixes.
        b"goalcup:",
        b"goalcup%3A",
        b"goalcup%3a",
        b"goalcup:0123456789abcdef0123456789abcdef",
        b"http://goalcup:0123456789abcdef0123456789abcde@127.0.0.1:4",
        b"goalcup%3Afedcba9876543210fedcba9876543210%40",
        b"0123456789abcdef",
        b"Z29hbGN1cDo",
        b"Z29hbGN1cDp",
        b"Z29hbGN1cDowMTIzNDU2Nzg5YWJjZGVmMDEyMzQ1Njc4OWFiY2RlZg==",
        b"MDEyMzQ1Njc4OWFi",
        b"+/",
    ];

    fn random_text(rng: &mut Rng) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..1 + rng.below(40) {
            if rng.below(4) == 0 {
                let class = b"aZ09_-.";
                for _ in 0..rng.below(24) {
                    out.push(class[rng.below(class.len())]);
                }
            } else {
                out.extend_from_slice(PIECES[rng.below(PIECES.len())]);
            }
        }
        out
    }

    /// Every recognizer is pinned to the exact source of the shared literal it models:
    /// editing a shared rule fails here, and refuses streaming at run time, until the
    /// recognizer is re-derived and its oracles re-run.
    #[test]
    fn shapes_follow_the_shared_list() {
        let compiled: Vec<(&str, &str)> = patterns()
            .iter()
            .map(|p| (p.kind.as_str(), p.re.as_str()))
            .collect();
        let shapes: Vec<(&str, &str)> = SHAPES.iter().map(|s| (s.kind, s.source)).collect();
        assert_eq!(compiled, shapes);
        assert!(policy_ready());
    }

    /// Soundness of the hold decision against the shared regexes themselves: for every
    /// match of every literal, every proper prefix of the match must keep a candidate of
    /// that literal starting at or before the match (or still be undecoded). Precision,
    /// for every literal but `email` (whose recognizer deliberately over-holds a domain
    /// run): once the unit after a match is decoded, its candidate is gone.
    #[test]
    fn recognizers_hold_every_prefix_of_a_match() {
        let now = CaptureTime {
            wall: "2026-09-27T10:00:00Z".into(),
            mono_ns: 0,
            clock_id: "t".into(),
        };
        let mut checked = 0usize;
        for seed in 1..=4000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let text = random_text(&mut rng);
            let mut matches = Vec::new();
            let view = super::super::match_view(&text);
            for (lit, p) in patterns().iter().enumerate() {
                for m in p.re.find_iter(&view) {
                    matches.push((lit, m.start() as u64, m.end() as u64));
                }
            }
            let mut scanner = Scanner::new(0, Arc::new(Policy::default()));
            scanner.mark(0, &now);
            let mut sink = Vec::new();
            for k in 1..=text.len() {
                scanner.feed(&text[k - 1..k], &now, &mut sink);
                let k = k as u64;
                for &(lit, s, t) in &matches {
                    if s < k && k < t {
                        checked += 1;
                        let held = scanner.decoded_end <= s
                            || scanner
                                .candidates
                                .iter()
                                .any(|c| c.lit == lit && c.start <= s);
                        assert!(
                            held,
                            "seed {seed}: {} match {s}..{t} not held after {k} bytes of {:?}",
                            SHAPES[lit].kind,
                            String::from_utf8_lossy(&text)
                        );
                        assert!(
                            scanner.held_start <= s,
                            "seed {seed}: bytes of a live match were released"
                        );
                    }
                    let email = SHAPES[lit].kind == "email";
                    if !email && k >= t + 4 && k < t + 5 {
                        checked += 1;
                        assert!(
                            !scanner
                                .candidates
                                .iter()
                                .any(|c| c.lit == lit && c.start == s),
                            "seed {seed}: {} match {s}..{t} still held at {k} in {:?}",
                            SHAPES[lit].kind,
                            String::from_utf8_lossy(&text)
                        );
                    }
                }
            }
        }
        assert!(
            checked > 10_000,
            "the generator reaches the rules: {checked}"
        );
    }

    /// The PEM END-line detector against the END half of the shared `private_key` literal:
    /// over generated bodies, its first completion is the end of the regex's first match.
    #[test]
    fn end_line_detector_matches_the_shared_literal() {
        let shared = patterns()
            .iter()
            .find(|p| p.kind == "private_key")
            .expect("private_key literal")
            .re
            .as_str();
        // The END line of the lazy arm, without the bounded arm: a block with no END line is
        // the walk's unit count, not a line this detector completes.
        let body = format!("(?-u:[\\x00-\\xFF]){{0,{PEM_BODY_MAX}}}");
        let end_half = shared
            .split(&format!("(?:{body}?"))
            .nth(1)
            .and_then(|h| h.strip_suffix(&format!("|{body})")))
            .expect("a bounded lazy body to an END line, or the bounded body, in the literal");
        let end_re = regex::bytes::Regex::new(end_half).expect("END half compiles");
        let pieces: &[&[u8]] = &[
            b"-",
            b"-----",
            b"------",
            b"E",
            b"EN",
            b"END",
            b"END ",
            b"-----END ",
            b"-----EN",
            b"RSA ",
            b" ",
            b"PRIVATE KEY",
            b"PRIVATE KEY-----",
            b"PRIVATE KEY----",
            b"KEY",
            b"x",
            b"\n",
            b"-END",
            b"PRIVATE",
            b"A",
        ];
        let mut seen = 0usize;
        for seed in 1..=20_000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut text = Vec::new();
            for _ in 0..1 + rng.below(14) {
                text.extend_from_slice(pieces[rng.below(pieces.len())]);
            }
            let mut detector = EndLine::Scan(0);
            let found = text
                .iter()
                .position(|&b| detector.step(Unit::Char(char::from(b))))
                .map(|i| i + 1);
            let want = end_re.find(&text).map(|m| m.end());
            seen += usize::from(want.is_some());
            assert_eq!(
                found,
                want,
                "seed {seed}: {:?}",
                String::from_utf8_lossy(&text)
            );
        }
        assert!(seen > 500, "the generator completes END lines: {seen}");
    }

    fn one_stream(redactor: &StreamRedactor) -> &StreamState {
        redactor.streams.values().next().expect("an open stream")
    }

    fn open_frame(producer: &str) -> CaptureFrame {
        CaptureFrame {
            stream: StreamKey {
                producer: producer.into(),
                stream: 1,
            },
            seq: 0,
            observed: CaptureTime {
                wall: "2026-09-27T10:00:00Z".into(),
                mono_ns: 0,
                clock_id: producer.into(),
            },
            event: FrameEvent::Open {
                source: goalcup_protocol::capture::CaptureSource::Native,
                metadata: CaptureMeta {
                    wire: None,
                    label: "store.jsonl#1".into(),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            },
        }
    }

    /// H-16: a stream-zero `Loss` returns the decided segments first, as a connection event
    /// does, and then one stream-less gap carrying the frame's own time and reason. It touches
    /// no stream state: the open stream keeps taking bytes from its next offset.
    #[test]
    fn a_loss_flushes_decided_bytes_then_records_a_streamless_gap() {
        let producer = "22222222222222222222222222222222";
        let observed = |seq: u64| CaptureTime {
            wall: "2026-09-27T10:00:00Z".into(),
            mono_ns: seq,
            clock_id: producer.into(),
        };
        let at = |stream: u64, seq: u64, event: FrameEvent| CaptureFrame {
            stream: StreamKey {
                producer: producer.into(),
                stream,
            },
            seq,
            observed: observed(seq),
            event,
        };
        let mut r = StreamRedactor::new();
        assert_eq!(r.push(open_frame(producer)).unwrap().len(), 1);
        let text = b"plain decided text\n";
        assert!(
            r.push(at(
                1,
                1,
                FrameEvent::Bytes {
                    offset: 0,
                    data: text.to_vec()
                }
            ))
            .unwrap()
            .is_empty(),
            "a short segment is coalesced until something flushes it"
        );
        let records = r
            .push(at(
                0,
                2,
                FrameEvent::Loss {
                    reason: GapReason::DecodeFailed,
                },
            ))
            .unwrap();
        assert_eq!(records.len(), 2, "{records:?}");
        assert!(
            matches!(
                &records[0],
                CaptureRecord::Segment {
                    source_offset: 0,
                    ..
                }
            ),
            "decided bytes come first: {:?}",
            records[0]
        );
        assert_eq!(
            records[1],
            CaptureRecord::Gap {
                stream: None,
                observed: observed(2),
                reason: GapReason::DecodeFailed,
                lost_bytes: None,
            },
            "the gap keeps the producer's own observation time"
        );
        // The stream is untouched: its next bytes continue at the next offset.
        assert!(
            r.push(at(
                1,
                3,
                FrameEvent::Bytes {
                    offset: text.len() as u64,
                    data: b"more\n".to_vec()
                }
            ))
            .is_ok()
        );
        assert_eq!(one_stream(&r).key.stream, 1);
    }

    /// `long_private_key_does_not_grow_pending_buffer`, measured from the inside: whatever
    /// a long candidate is — a key body, a local part, a JSON key, a whitespace run — the
    /// held bytes, candidates and unsent segment stay within their bounds after every push.
    #[test]
    fn held_state_stays_bounded() {
        let producer = "11111111111111111111111111111111";
        let bodies: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
            (
                "pem",
                b"-----BEGIN RSA PRIVATE KEY-----\n".to_vec(),
                b"MIIEowIBAAKCAQEA+/=\n".to_vec(),
            ),
            ("local-part", Vec::new(), b"abcdefghij.klmnop+".to_vec()),
            (
                "json-key",
                b"{\"".to_vec(),
                b"abcdefghijklmnopqrstuvwxyz".to_vec(),
            ),
            (
                "json-value",
                b"{\"api_key\":\"".to_vec(),
                b"abc/+= xyz".to_vec(),
            ),
            ("bearer-spaces", b"bearer".to_vec(), b" \t ".to_vec()),
            (
                "jwt-segments",
                b"eyJ".to_vec(),
                b"eyJeyJeyJ0123456789.".to_vec(),
            ),
            ("quotes", Vec::new(), b"\"a\"b\"token\":\"".to_vec()),
            (
                "pem-header",
                b"-----BEGIN ".to_vec(),
                b"RSA PRIVATE ".to_vec(),
            ),
        ];
        for (name, lead, unit) in bodies {
            let mut r = StreamRedactor::new();
            r.push(open_frame(producer)).unwrap();
            let mut offset = 0u64;
            let mut data = lead;
            for (seq, round) in (1u64..).zip(0..24) {
                while data.len() < MAX_SEGMENT_BYTES - unit.len() {
                    data.extend_from_slice(&unit);
                }
                let frame = CaptureFrame {
                    stream: StreamKey {
                        producer: producer.into(),
                        stream: 1,
                    },
                    seq,
                    observed: CaptureTime {
                        wall: "2026-09-27T10:00:00Z".into(),
                        mono_ns: seq,
                        clock_id: producer.into(),
                    },
                    event: FrameEvent::Bytes {
                        offset,
                        data: std::mem::take(&mut data),
                    },
                };
                offset += match &frame.event {
                    FrameEvent::Bytes { data, .. } => data.len() as u64,
                    _ => 0,
                };
                r.push(frame).unwrap();
                let state = one_stream(&r);
                assert!(
                    state.scanner.held_bytes() <= MAX_HELD_BYTES + 3,
                    "{name} round {round}: {} bytes held",
                    state.scanner.held_bytes()
                );
                assert!(
                    state.scanner.candidates.len() <= MAX_CANDIDATES,
                    "{name}: candidates"
                );
                assert!(
                    state
                        .segment
                        .as_ref()
                        .is_none_or(|s| s.data.len() <= MAX_SEGMENT_BYTES),
                    "{name}: unsent segment"
                );
            }
        }
    }

    /// Every connector `extra_patterns` entry the repository ships, in connector order, read
    /// from each `connectors/<id>/harness.toml`.
    fn shipped_extra_patterns() -> Vec<ExtraPattern> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
        let mut dirs: Vec<_> = std::fs::read_dir(&root)
            .expect("connectors directory")
            .map(|e| e.expect("entry").path())
            .filter(|p| p.join("harness.toml").is_file())
            .collect();
        dirs.sort();
        let mut out = Vec::new();
        for dir in dirs {
            let text = std::fs::read_to_string(dir.join("harness.toml")).expect("harness.toml");
            let manifest = goalcup_protocol::harness::HarnessManifest::from_toml_str(&text)
                .expect("a shipped harness.toml parses");
            out.extend(crate::redact::extra_patterns_of(&manifest));
        }
        out
    }

    /// One rule of every form under names no shipped connector uses: a one-byte field, a
    /// field with upper case, a digit and a hyphen, and a prefix that ends inside the header
    /// name. The recognizers are parameterised by these bytes, so the soundness tests run
    /// over these as well as over the shipped rules.
    fn synthetic_extra_patterns() -> Vec<ExtraPattern> {
        let field = |kind: &str, field: &str, value| {
            ExtraPattern::new(
                kind,
                RedactionRule::JsonField {
                    field: field.into(),
                    value,
                },
            )
            .expect("a valid rule")
        };
        vec![
            field("t_object", "k", harness::FieldValue::Object),
            field("t_string", "Tier-2", harness::FieldValue::String),
            field("t_scalar", "k", harness::FieldValue::Scalar),
            ExtraPattern::new("t_header", RedactionRule::HeaderPrefix("x-a".into()))
                .expect("a valid rule"),
        ]
    }

    fn every_extra_pattern() -> Vec<ExtraPattern> {
        let mut all = shipped_extra_patterns();
        all.extend(synthetic_extra_patterns());
        all
    }

    /// Every form maps onto its recognizer with the span its regex declares: a JSON field's
    /// value is group 1, a header line is the whole match. The shipped rules use every form.
    #[test]
    fn every_rule_form_streams_with_its_span() {
        let all = every_extra_pattern();
        let policy = Policy::with_extra(&all).expect("every form streams");
        assert_eq!(policy.len(), SHAPES.len() + all.len());
        for (lit, p) in (SHAPES.len()..).zip(&all) {
            let (form, group) = match policy.form(lit) {
                Some(Form::Field { key, .. }) => (String::from_utf8_lossy(key).into_owned(), 1),
                Some(Form::HeaderPrefix { prefix }) => {
                    (String::from_utf8_lossy(prefix).into_owned(), 0)
                }
                _ => panic!("{} has no connector recognizer", p.kind()),
            };
            assert!(p.regex().starts_with(&form), "{form} leads {}", p.regex());
            assert_eq!(policy.extra[lit - SHAPES.len()].pattern.span_group, group);
        }
        let shipped = shipped_extra_patterns();
        for value in ["object", "string", "scalar"] {
            assert!(
                shipped.iter().any(|p| matches!(p.rule(),
                    RedactionRule::JsonField { value: v, .. } if v.as_str() == value)),
                "no shipped rule uses the {value} form"
            );
        }
        assert!(
            shipped
                .iter()
                .any(|p| matches!(p.rule(), RedactionRule::HeaderPrefix(_))),
            "no shipped rule uses the header form"
        );
    }

    const EXTRA_PIECES: &[&[u8]] = &[
        b"\"rate_limits\"",
        b"\"plan_type\"",
        b"\"credits_remaining\"",
        b"\"k\"",
        b"\"k\": ",
        b"\"k\":{",
        b"\"Tier-2\"",
        b"\"Tier-2\": \"",
        b"x-a",
        b"x-anthropic-billing-",
        b"x-anthropic-billing-header",
        b"x-anthropic-billing",
        b"\"rate_limits\": {",
        b"\"rate_limits\":{\"primary\":{",
        b"{\"k\":{\"v\":1}}",
        b"\"plan_type\": \"",
        b"\"plan_type\":\"",
        b"\"credits_remaining\": ",
        b"\"credits_remaining\":\"",
        b"\"credits\": {",
        b"\"credits\":{\"balance\":",
        b"\"safety_identifier\": ",
        b"x-anthropic-billing-header: ",
        b"x-anthropic-billing-cc:",
        b"\"rate_",
        b"limits\"",
        b"header",
        b":",
        b": ",
        b" ",
        b"\t",
        b"\n",
        b"\r",
        b"\r\n",
        "\u{a0}".as_bytes(),
        "\u{2003}".as_bytes(),
        "\u{85}".as_bytes(),
        "\u{2028}".as_bytes(),
        "\u{3000}".as_bytes(),
        "\u{1680}".as_bytes(),
        b"\x0b",
        b"\x0c",
        "\u{e9}".as_bytes(),
        b"\xff",
        b"\xe2\x80",
        b"{",
        b"}",
        b"{}",
        b"\"",
        b"\\",
        b"\\\"",
        b",",
        b"[",
        b"]",
        b"412",
        b"17.5",
        b"plus",
        b"null",
        b"\"primary\"",
        b"\"used_percent\": 12.5",
        b"cc_version=2.1.283.abc;",
        b" cc_entrypoint=cli;",
        b"-",
        b"a",
        b"Z9",
        b"sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV",
        b"ops@example.com",
    ];

    /// Each JSON-field rule's quoted name followed by the start of a value, read from the
    /// rules themselves, so a shipped field reaches the generator without its name (which may
    /// carry a harness name) being written in core code.
    fn field_pieces(rules: &[ExtraPattern]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for p in rules {
            if let RedactionRule::JsonField { field, .. } = p.rule() {
                for tail in [": ", ":", ":\"", ": {"] {
                    out.push(format!("\"{field}\"{tail}").into_bytes());
                }
            }
        }
        out
    }

    fn random_extra_text(rng: &mut Rng, fields: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..1 + rng.below(40) {
            if !fields.is_empty() && rng.below(12) == 0 {
                out.extend_from_slice(&fields[rng.below(fields.len())]);
            } else if rng.below(5) == 0 {
                let class = b"az09_-.";
                for _ in 0..rng.below(12) {
                    out.push(class[rng.below(class.len())]);
                }
            } else {
                out.extend_from_slice(EXTRA_PIECES[rng.below(EXTRA_PIECES.len())]);
            }
        }
        out
    }

    /// Each hand-coded character predicate the connector recognizers step on is exactly
    /// its regex class, over every Unicode scalar value and every byte that cannot start
    /// one: a predicate that drops or adds a character (VT or FF read as a line end, say)
    /// fails here even when no generator happens to produce that character. Each class is
    /// taken from a form's regex, and compiled as the V1 compiler does (Unicode mode).
    #[test]
    fn extra_predicates_equal_their_regex_classes() {
        type Predicate = fn(Unit) -> bool;
        let sources: Vec<String> = every_extra_pattern().iter().map(|p| p.regex()).collect();
        let classes: [(&str, Predicate); 6] = [
            (r"\s", space),
            (r"[^{}]", object_char),
            (r#"[^"\\]"#, string_char),
            (r"[^,}\s]", credit_char),
            (r"[a-z0-9-]", header_name),
            (r#"[^\r\n"]"#, header_value),
        ];
        let mut buf = [0u8; 4];
        for (class, predicate) in classes {
            assert!(
                sources.iter().any(|s| s.contains(class)),
                "{class} is a class of a form's regex"
            );
            let re = regex::bytes::Regex::new(&format!("^{class}$")).expect("class compiles");
            for c in (0..=0x10_FFFF_u32).filter_map(char::from_u32) {
                let bytes = c.encode_utf8(&mut buf).as_bytes();
                assert_eq!(
                    predicate(Unit::Char(c)),
                    re.is_match(bytes),
                    "{class} disagrees with its predicate at U+{:04X}",
                    u32::from(c)
                );
            }
            // An invalid byte is the unit the rules match it as (H-20), for the stream too.
            for b in 0x80..=0xFF_u8 {
                assert_eq!(
                    predicate(INVALID),
                    re.is_match(&super::super::match_view(&[b])),
                    "{class} disagrees with its predicate at the lone byte {b:#x}"
                );
            }
        }
    }

    /// The two class predicates the `proxy_credential` recognizers step on are exactly the
    /// classes of their literals, over every Unicode scalar value and an invalid byte.
    #[test]
    fn proxy_credential_predicates_equal_their_regex_classes() {
        type Predicate = fn(Unit) -> bool;
        let classes: [(&str, Predicate); 2] =
            [("[a-f0-9]", hex_lower), ("[A-Za-z0-9+/]", base64_char)];
        let mut buf = [0u8; 4];
        for (class, predicate) in classes {
            assert!(
                SHAPES
                    .iter()
                    .any(|s| s.kind == "proxy_credential" && s.source.contains(class)),
                "{class} is a class of a proxy_credential literal"
            );
            let re = regex::bytes::Regex::new(&format!("^{class}$")).expect("class compiles");
            for c in (0..=0x10_FFFF_u32).filter_map(char::from_u32) {
                let bytes = c.encode_utf8(&mut buf).as_bytes();
                assert_eq!(
                    predicate(Unit::Char(c)),
                    re.is_match(bytes),
                    "{class} disagrees with its predicate at U+{:04X}",
                    u32::from(c)
                );
            }
            assert_eq!(
                predicate(INVALID),
                re.is_match(&super::super::match_view(b"\xff")),
                "{class} disagrees with its predicate at an invalid byte"
            );
        }
    }

    /// The two hand-coded predicates the `masked_key` recognizer steps on are exactly the
    /// regex classes they stand for, over every Unicode scalar value: `(?i)[A-Za-z0-9]`
    /// takes `ſ` and the Kelvin sign, and the case-folded prefix and escape letters do too.
    #[test]
    fn masked_key_predicates_equal_their_regex_classes() {
        type Predicate = fn(Unit) -> bool;
        let folded_is = |letter: u8| move |u: Unit| folded(u) == Some(letter);
        let letters: [(&str, u8); 3] = [("(?i)s", b's'), ("(?i)k", b'k'), ("(?i)u", b'u')];
        let mut buf = [0u8; 4];
        let classes: [(&str, Predicate); 1] = [("(?i)[A-Za-z0-9]", word_folded)];
        for (class, predicate) in classes {
            let re = regex::bytes::Regex::new(&format!("^{class}$")).expect("class compiles");
            for c in (0..=0x10_FFFF_u32).filter_map(char::from_u32) {
                let bytes = c.encode_utf8(&mut buf).as_bytes();
                assert_eq!(
                    predicate(Unit::Char(c)),
                    re.is_match(bytes),
                    "{class} disagrees with its predicate at U+{:04X}",
                    u32::from(c)
                );
            }
            assert_eq!(
                predicate(INVALID),
                re.is_match(&super::super::match_view(b"\xff")),
                "{class} disagrees with its predicate at an invalid byte"
            );
        }
        for (class, letter) in letters {
            let re = regex::bytes::Regex::new(&format!("^{class}$")).expect("class compiles");
            let predicate = folded_is(letter);
            for c in (0..=0x10_FFFF_u32).filter_map(char::from_u32) {
                let bytes = c.encode_utf8(&mut buf).as_bytes();
                assert_eq!(
                    predicate(Unit::Char(c)),
                    re.is_match(bytes),
                    "{class} disagrees with its predicate at U+{:04X}",
                    u32::from(c)
                );
            }
            assert_eq!(
                predicate(INVALID),
                re.is_match(&super::super::match_view(b"\xff")),
                "{class} disagrees with its predicate at an invalid byte"
            );
        }
    }

    /// Soundness of each connector recognizer against its own regex, as for the shared
    /// literals: for every match of an extra rule, every proper prefix keeps a candidate
    /// of that rule starting at or before the match (or is still undecoded), and no byte
    /// of it has been released. Precision for the two forms whose match cannot extend (an
    /// object value closes its object, a string value its string): once the unit after the
    /// match is decoded, its candidate is gone. The scalar and header recognizers
    /// deliberately over-hold (an open quote, or a whitespace run that a later value
    /// character could still claim). The rules are the shipped ones and one more of every
    /// form under other names ([`synthetic_extra_patterns`]).
    #[test]
    fn extra_recognizers_hold_every_prefix_of_a_match() {
        let now = CaptureTime {
            wall: "2026-09-28T10:00:00Z".into(),
            mono_ns: 0,
            clock_id: "t".into(),
        };
        let policy = Arc::new(Policy::with_extra(&every_extra_pattern()).expect("policy"));
        let fields = field_pieces(&every_extra_pattern());
        let mut checked = 0usize;
        let mut per_rule = vec![0usize; policy.len()];
        for seed in 1..=6000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let raw = random_extra_text(&mut rng, &fields);
            // The rules match on the view; the scanner is fed the raw bytes.
            let text = super::super::match_view(&raw).into_owned();
            let mut matches = Vec::new();
            for (lit, p) in policy.rules().enumerate().skip(SHAPES.len()) {
                for c in p.re.captures_iter(&text) {
                    let Some(m) = c.get(0) else { continue };
                    assert!(
                        m.start() < m.end(),
                        "an extra rule matched the empty string"
                    );
                    assert_ne!(
                        text[m.start()],
                        b'\n',
                        "an extra match starts with a newline"
                    );
                    // The replacement's quoting is decided by bytes inside the whole match.
                    if let Some(g) = c.get(p.span_group).filter(|_| p.span_group > 0) {
                        let inside = &text[m.start()..m.end()];
                        assert_eq!(
                            super::super::span_is_a_json_string_interior(&text, g.start(), g.end()),
                            super::super::span_is_a_json_string_interior(
                                inside,
                                g.start() - m.start(),
                                g.end() - m.start()
                            ),
                            "seed {seed}: {} quoting reads outside its match",
                            policy.kind(lit)
                        );
                    }
                    matches.push((lit, m.start() as u64, m.end() as u64));
                    per_rule[lit] += 1;
                }
            }
            let mut scanner = Scanner::new(0, Arc::clone(&policy));
            scanner.mark(0, &now);
            let mut sink = Vec::new();
            for k in 1..=text.len() {
                scanner.feed(&raw[k - 1..k], &now, &mut sink);
                let k = k as u64;
                for &(lit, s, t) in &matches {
                    if s < k && k < t {
                        checked += 1;
                        let held = scanner.decoded_end <= s
                            || scanner
                                .candidates
                                .iter()
                                .any(|c| c.lit == lit && c.start <= s);
                        assert!(
                            held,
                            "seed {seed}: {} match {s}..{t} not held after {k} bytes of {:?}",
                            policy.kind(lit),
                            String::from_utf8_lossy(&text)
                        );
                        assert!(
                            scanner.held_start <= s,
                            "seed {seed}: bytes of a live match were released"
                        );
                    }
                    let exact = matches!(
                        policy.form(lit),
                        Some(Form::Field {
                            value: FieldValue::Object | FieldValue::String,
                            ..
                        })
                    );
                    if exact && k >= t + 4 && k < t + 5 {
                        checked += 1;
                        assert!(
                            !scanner
                                .candidates
                                .iter()
                                .any(|c| c.lit == lit && c.start == s),
                            "seed {seed}: {} match {s}..{t} still held at {k} in {:?}",
                            policy.kind(lit),
                            String::from_utf8_lossy(&text)
                        );
                    }
                }
            }
        }
        assert!(
            checked > 10_000,
            "the generator reaches the rules: {checked}"
        );
        for (lit, &count) in per_rule.iter().enumerate().skip(SHAPES.len()) {
            assert!(
                count > 100,
                "the generator matches {}: {count}",
                policy.kind(lit)
            );
        }
    }

    /// The hold bound for the connector rules: however long an object, a string, a bare
    /// value, a header name or value, or the whitespace around a colon runs, the held
    /// bytes, candidates and unsent segment stay within their bounds after every push.
    #[test]
    fn extra_held_state_stays_bounded() {
        let producer = "11111111111111111111111111111111";
        let bodies: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
            (
                "object",
                b"{\"rate_limits\": {\"primary\": {".to_vec(),
                b"\"used_percent\": 1.5,\n".to_vec(),
            ),
            (
                "string",
                b"{\"plan_type\": \"".to_vec(),
                b"plus pro team\n".to_vec(),
            ),
            (
                "bare",
                b"{\"credits_remaining\": ".to_vec(),
                b"4120".to_vec(),
            ),
            (
                "quoted",
                b"{\"credits_remaining\": \"".to_vec(),
                b"41 20\n".to_vec(),
            ),
            (
                "key-spaces",
                b"{\"rate_limits\"".to_vec(),
                b" \n\t".to_vec(),
            ),
            (
                "colon-spaces",
                b"{\"plan_type\":".to_vec(),
                b" \r\n".to_vec(),
            ),
            (
                "header-name",
                b"x-anthropic-billing-".to_vec(),
                b"header-".to_vec(),
            ),
            (
                "header-value",
                b"x-anthropic-billing-header: ".to_vec(),
                b"cc_version=2.1.283; ".to_vec(),
            ),
            (
                "header-spaces",
                b"x-anthropic-billing-header:".to_vec(),
                b" \r\n".to_vec(),
            ),
        ];
        for (name, lead, unit) in bodies {
            let mut r = StreamRedactor::with_extra_patterns(&shipped_extra_patterns())
                .expect("shipped patterns");
            r.push(open_frame(producer)).unwrap();
            let mut offset = 0u64;
            let mut data = lead;
            for (seq, round) in (1u64..).zip(0..24) {
                while data.len() < MAX_SEGMENT_BYTES - unit.len() {
                    data.extend_from_slice(&unit);
                }
                let len = data.len() as u64;
                let frame = CaptureFrame {
                    stream: StreamKey {
                        producer: producer.into(),
                        stream: 1,
                    },
                    seq,
                    observed: CaptureTime {
                        wall: "2026-09-28T10:00:00Z".into(),
                        mono_ns: seq,
                        clock_id: producer.into(),
                    },
                    event: FrameEvent::Bytes {
                        offset,
                        data: std::mem::take(&mut data),
                    },
                };
                offset += len;
                r.push(frame).unwrap();
                let state = one_stream(&r);
                assert!(
                    state.scanner.held_bytes() <= MAX_HELD_BYTES + 3,
                    "{name} round {round}: {} bytes held",
                    state.scanner.held_bytes()
                );
                assert!(
                    state.scanner.candidates.len() <= MAX_CANDIDATES,
                    "{name}: candidates"
                );
                assert!(
                    state
                        .scanner
                        .redacting
                        .as_ref()
                        .is_none_or(|red| red.live.len() <= MAX_CANDIDATES),
                    "{name}: followed candidates"
                );
                assert!(
                    state
                        .segment
                        .as_ref()
                        .is_none_or(|s| s.data.len() <= MAX_SEGMENT_BYTES),
                    "{name}: unsent segment"
                );
            }
        }
    }
}
