//! Streaming masking (PRD-05 section 6): one [`Scanner`] per recorded stream turns pushed raw
//! bytes into released, masked bytes, holding only what is still undecided.
//!
//! Copied from the 0.x `goalcup-daemon` streaming redactor (`redact/stream.rs` lines 1 to
//! 2361), without frames, segments, time marks, `email`, the `object` form and
//! `proxy_credential`, and with a recognizer for `url_password` in slot 12.
//!
//! # The policy is the whole-record one
//!
//! What counts as a secret, the span of each rule, first-wins selection and the fill come from
//! [`super::rules`], the code `Masker::mask_whole` runs. This module adds the answer to "may
//! these bytes leave yet?": a small bounded recognizer per rule (`SHAPES`, each pinned to its
//! literal's exact source) reports which trailing bytes could still become, or still extend, a
//! match once more bytes arrive. Those bytes are held. Everything before the earliest such
//! start, moved back to the start of any whole match that crosses it and to before any
//! backslash run, is a clean cut: it is masked with the whole-record rules and released.
//!
//! While no hold reaches [`MAX_HELD_BYTES`], the bytes and spans a stream returns, joined,
//! equal `mask_whole` of the whole stream however it was split (for a WebSocket direction, of
//! each message, since `boundary()` decides the hold as end of input). A secret split across
//! two pushes is one secret.
//!
//! # The bound: fail closed
//!
//! A stream holds at most [`MAX_HELD_BYTES`] undecided bytes plus the up-to-three bytes of an
//! incomplete UTF-8 sequence, and at most `MAX_CANDIDATES` live candidates. Past that it fails
//! closed: the bytes before the earliest undecided byte leave masked; from there every byte is
//! starred as `fail_closed` until the first of: a `\n` that no live candidate reaches, a message
//! end (`boundary()`), a gap or the end. So it may mask more than the whole-record rules, never
//! less: no byte any rule's match could cover is released raw.
//!
//! At a gap, held bytes that already look like secret material (a complete vendor prefix,
//! `eyJ`, a `bearer` value, a PEM body, a secret JSON value, a shell `token=` value, a URL
//! password) are starred to the cut; other held bytes are decided as end of input.

use super::rules::{self, Claim, GUARD_BYTES, Pattern, Run, candidates, select};
use super::values::{Advance, Progress, RunSecrets};
use super::{MaskError, Masked, SecretValue, Span, ValueCounts};
use crate::connector::{FieldValue, MaskPattern, MaskRule};
use std::sync::Arc;

/// The most undecided raw bytes one stream holds: a stated memory guard, not a cap.
pub(crate) const MAX_HELD_BYTES: usize = 16 * 1024;

/// Live recognizer states per stream. Equivalent states are merged as they arise, so real
/// input stays in single digits; reaching this fails closed exactly like a full hold.
const MAX_CANDIDATES: usize = 64;
/// The longest key the shared JSON literal can match, in units.
const JSON_KEY_MAX_UNITS: u16 = 2008;
/// The most backslashes the generic-secret literals take before a quote.
const MAX_QUOTE_ESCAPES: u8 = 7;
/// The `url_password` bounds: user and password units.
const URL_USER_MAX: u16 = 256;
const URL_PASSWORD_MAX: u16 = 1024;

/// Positions of the JSON `generic_secret_assignment` literal and of `url_password` in the shared
/// list, the two with a capture group. They do not move: `url_password` took the slot `email`
/// left.
const JSON_LITERAL: usize = 10;
const URL_LITERAL: usize = 12;

const _: () = assert!(rules::PEM_BODY_MAX + 512 <= MAX_HELD_BYTES);

// ── units ──────────────────────────────────────────────────────────────────────────────

/// One decoding step: a Unicode scalar value. A byte that is not valid UTF-8 is one unit, read
/// as `rules::INVALID_BYTE_AS`, because the rules match on `rules::match_view`.
type Unit = char;

/// The unit an invalid byte decodes to.
const INVALID: Unit = rules::INVALID_BYTE_AS as char;

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
        0x00..=0x7F => return Decoded::Unit(char::from(b0), 1),
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
        Some(c) => Decoded::Unit(c, need + 1),
        None => Decoded::Unit(INVALID, 1),
    }
}

const LONG_S: char = '\u{017F}';
const KELVIN: char = '\u{212A}';

fn ascii(u: Unit) -> Option<u8> {
    u.is_ascii().then_some(u as u8)
}

/// The ASCII letter a case-insensitive shared rule compares this unit as.
fn folded(u: Unit) -> Option<u8> {
    match u {
        LONG_S => Some(b's'),
        KELVIN => Some(b'k'),
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
    matches!(u, LONG_S | KELVIN)
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
/// `\s` in Unicode mode: exactly the White_Space property.
fn space(u: Unit) -> bool {
    u.is_whitespace()
}
/// `[A-Za-z0-9_\-]` under `(?i)`, the characters of a JSON secret key.
fn json_key(u: Unit) -> bool {
    token_folded(u)
}

/// One unit of the shell literal's value after a `=` with no space, as its two alternatives read
/// it. `token_only` and `run` track the first alternative, `[A-Za-z0-9_\-]{16,}`: it is decided
/// once it has sixteen units and a unit outside its class arrives, so the candidate ends there.
/// The second is the form-encoded value, a run of `%XX` escapes and units of the class
/// `[A-Za-z0-9_.~+/=\-]`, which needs sixteen units and a `%XX` escape or the dotted shape (see
/// `redact.rs`). Its match ends at the end of that run, so this keeps every unit of the class
/// live past the first alternative, and the regex then decides the exact span. `None` ends the
/// candidate.
fn encoded_step(run: u8, token_only: bool, hex: u8, u: Unit) -> Option<Shell> {
    if hex > 0 {
        return ascii_where(u, |b| b.is_ascii_hexdigit()).then_some(Shell::Encoded {
            run,
            token_only: false,
            hex: hex - 1,
        });
    }
    if token_folded(u) {
        return Some(Shell::Encoded {
            run: (run + 1).min(16),
            token_only,
            hex: 0,
        });
    }
    if token_only && run >= 16 {
        return None;
    }
    match u {
        '%' => Some(Shell::Encoded {
            run,
            token_only: false,
            hex: 2,
        }),
        '.' | '~' | '+' | '/' | '=' => Some(Shell::Encoded {
            run,
            token_only: false,
            hex: 0,
        }),
        _ => None,
    }
}
/// `[^"\\]` in Unicode mode: any scalar value but a double quote or a backslash.
fn string_char(u: Unit) -> bool {
    u != '"' && u != '\\'
}
/// `[^,}\s]` in Unicode mode, a bare scalar value's class: any scalar value but a comma, a
/// closing brace or White_Space.
fn scalar_char(u: Unit) -> bool {
    u != ',' && u != '}' && !u.is_whitespace()
}
/// `[a-z0-9-]`
fn header_name(u: Unit) -> bool {
    ascii_where(u, |b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
    })
}
/// `[^\r\n"]` in Unicode mode: any scalar value but CR, LF or a double quote.
fn header_value(u: Unit) -> bool {
    !matches!(u, '\r' | '\n' | '"')
}
/// `[^\s/?#@:"'<>\\]`, the user of `url_password`.
fn url_user(u: Unit) -> bool {
    url_password_char(u) && !is(u, b':')
}
/// `[^\s/?#@"'<>\\]`, its password.
fn url_password_char(u: Unit) -> bool {
    !u.is_whitespace() && !matches!(u, '/' | '?' | '#' | '@' | '"' | '\'' | '<' | '>' | '\\')
}

// ── the streaming form of each rule ────────────────────────────────────────────────────

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
    /// `(?:\\{0,3}/){2}[^\s/?#@:"'<>\\]{0,256}:([^\s/?#@"'<>\\]{1,1024})@`
    UrlPassword,
    /// A connector rule on one JSON field: `KEY\s*:\s*VALUE`, `KEY` the quoted field name.
    Field {
        key: Box<[u8]>,
        scalar: bool,
    },
    /// A connector rule on one header: `PREFIX[a-z0-9-]+:\s*[^\r\n"]+`.
    HeaderPrefix {
        prefix: Box<[u8]>,
    },
    /// Form `i` of the exact values ([`RunSecrets`]): printable ASCII, with at most one line
    /// break in each gap between two of its bytes ([`Progress`]).
    Literal {
        form: usize,
    },
}

struct Shape {
    kind: &'static str,
    /// The exact source of the literal this recognizer was derived from. `Policy::new` refuses
    /// a `shapes.tsv` line that differs, so a rule edit cannot silently outrun its recognizer.
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

/// The streaming recognizer of each shared literal, in `shapes.tsv` order. A recognizer never
/// decides a span below the bound (the regex does); it says how long a candidate must be held,
/// and how long a fail-closed window must run.
static SHAPES: [Shape; 20] = [
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
    // A provider key echoed masked. At most 79 units, so a candidate is held for about a
    // hundred bytes. Soft: it loses every overlap, so it needs no left context either.
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
        source: r#"(?i)\\{0,7}"[A-Za-z0-9_\-]{0,1000}(?:api[_-]?key|secret|token|password)(?:[_\-][A-Za-z0-9_\-]{0,999})?\\{0,7}"\s*:\s*\\{0,7}"((?:[^"\\]|\\{1,7}[^"\\]){16,})\\{0,7}""#,
        form: Form::JsonAssignment,
    },
    Shape {
        kind: "generic_secret_assignment",
        source: r#"(?i)(?:api[_-]?key|secret|token|password)(?:\s*[=:]\s*(?:\\{0,7}['"])?[A-Za-z0-9_\-]{16,}|=(?:%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){15,}|[A-Za-z0-9_.~+/=\-]{1}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){14,}|[A-Za-z0-9_.~+/=\-]{2}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){13,}|[A-Za-z0-9_.~+/=\-]{3}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){12,}|[A-Za-z0-9_.~+/=\-]{4}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){11,}|[A-Za-z0-9_.~+/=\-]{5}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){10,}|[A-Za-z0-9_.~+/=\-]{6}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){9,}|[A-Za-z0-9_.~+/=\-]{7}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){8,}|[A-Za-z0-9_.~+/=\-]{8}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){7,}|[A-Za-z0-9_.~+/=\-]{9}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){6,}|[A-Za-z0-9_.~+/=\-]{10}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){5,}|[A-Za-z0-9_.~+/=\-]{11}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){4,}|[A-Za-z0-9_.~+/=\-]{12}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){3,}|[A-Za-z0-9_.~+/=\-]{13}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){2,}|[A-Za-z0-9_.~+/=\-]{14}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){1,}|[A-Za-z0-9_.~+/=\-]{15,}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2})*|[0-9]\.[A-Za-z0-9_.~+/=\-]{14,}))"#,
        form: Form::ShellAssignment,
    },
    Shape {
        kind: "url_password",
        source: r#"(?:\\{0,3}/){2}[^\s/?#@:"'<>\\]{0,256}:([^\s/?#@"'<>\\]{1,1024})@"#,
        form: Form::UrlPassword,
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
];

/// The kind of the exact-value rule.
pub(crate) const RUN_SECRET_KIND: &str = "run_secret";
/// The kind of bytes starred because the hold overflowed.
pub(crate) const FAIL_CLOSED_KIND: &str = "fail_closed";

/// One rule after the shared literals: a connector rule or an exact-value form.
struct Extra {
    pattern: Pattern,
    form: Form,
}

/// The rule set every stream of one masker shares: the shared literals (`lit` below
/// [`SHAPES`]'s length), then the connector rules in their order, then the exact-value forms.
/// A `lit` is a rule's position in this list, which is also its priority in [`select`].
pub(crate) struct Policy {
    shared: Vec<Pattern>,
    extra: Vec<Extra>,
    secrets: RunSecrets,
    /// The first-unit dispatch table (PRD-05 section 11 step 3): for each [`dispatch_slot`], the
    /// rules whose candidate that unit can start, in `lit` order.
    starts: Vec<Vec<usize>>,
    /// `lit` to its index in [`Self::names`].
    rule_of: Vec<usize>,
    /// `mask_rules()`: one name per kind, `fail_closed` last.
    names: Vec<String>,
    pub(crate) counts: ValueCounts,
    #[cfg(feature = "test-hooks")]
    pub(crate) clear: bool,
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Policy")
            .field("rules", &self.names)
            .field("secrets", &self.secrets)
            .finish_non_exhaustive()
    }
}

impl Policy {
    /// The policy of one masker. A `shapes.tsv` line that differs from its recognizer's literal,
    /// or does not compile, is `MaskError::Shape(i)`; a connector rule that does not compile,
    /// or that takes the name `run_secret` or `fail_closed`, is `MaskError::Rule(i)`. An exact
    /// value that cannot be used is skipped and counted.
    pub(crate) fn new(conn: &[MaskRule], values: &[SecretValue]) -> Result<Policy, MaskError> {
        let mut shared = Vec::with_capacity(SHAPES.len());
        let mut names: Vec<String> = Vec::new();
        let mut rule_of = Vec::new();
        let rows: Vec<Option<rules::Row>> = rules::rows().collect();
        if rows.len() != SHAPES.len() {
            return Err(MaskError::Shape(rows.len().min(SHAPES.len())));
        }
        for (i, (row, shape)) in rows.iter().zip(SHAPES.iter()).enumerate() {
            let row = row
                .filter(|r| r.kind == shape.kind && r.source == shape.source)
                .ok_or(MaskError::Shape(i))?;
            let pattern = Pattern::shared(&row).ok_or(MaskError::Shape(i))?;
            let group_ok = match i {
                JSON_LITERAL | URL_LITERAL => pattern.span_group == 1,
                _ => pattern.span_group == 0,
            };
            if !group_ok {
                return Err(MaskError::Shape(i));
            }
            rule_of.push(name_index(&mut names, row.kind));
            shared.push(pattern);
        }
        let mut extra = Vec::with_capacity(conn.len());
        for (i, rule) in conn.iter().enumerate() {
            if [RUN_SECRET_KIND, FAIL_CLOSED_KIND].contains(&rule.kind.as_str()) {
                return Err(MaskError::Rule(i));
            }
            let pattern = rules::connector(rule).ok_or(MaskError::Rule(i))?;
            let form = match &rule.pattern {
                MaskPattern::JsonField { field, value } => Form::Field {
                    key: format!("\"{field}\"").into_bytes().into_boxed_slice(),
                    scalar: *value == FieldValue::Scalar,
                },
                MaskPattern::HeaderPrefix(prefix) => Form::HeaderPrefix {
                    prefix: prefix.as_bytes().into(),
                },
            };
            rule_of.push(name_index(&mut names, &rule.kind));
            extra.push(Extra { pattern, form });
        }
        let (secrets, counts) = RunSecrets::new(values);
        let secret_rule = name_index(&mut names, RUN_SECRET_KIND);
        for i in 0..secrets.len() {
            if let Some(re) = secrets.regex(i) {
                extra.push(Extra {
                    pattern: Pattern::exact(re.clone()),
                    form: Form::Literal { form: i },
                });
                rule_of.push(secret_rule);
            }
        }
        names.push(FAIL_CLOSED_KIND.to_string());
        let mut policy = Policy {
            shared,
            extra,
            secrets,
            starts: Vec::new(),
            rule_of,
            names,
            counts,
            #[cfg(feature = "test-hooks")]
            clear: false,
        };
        policy.starts = (0..DISPATCH_SLOTS)
            .map(|slot| {
                let u = slot_unit(slot);
                (0..policy.len())
                    .filter(|&lit| Candidate::spawn(&policy, lit, u).is_some())
                    .collect()
            })
            .collect();
        Ok(policy)
    }

    /// A policy with no rules whose streams pass bytes through (test hooks only).
    #[cfg(feature = "test-hooks")]
    pub(crate) fn clear() -> Policy {
        Policy {
            shared: Vec::new(),
            extra: Vec::new(),
            secrets: RunSecrets::default(),
            starts: Vec::new(),
            rule_of: Vec::new(),
            names: vec![FAIL_CLOSED_KIND.to_string()],
            counts: ValueCounts::default(),
            clear: true,
        }
    }

    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// The index of `fail_closed` in [`Self::names`]: the last.
    pub(crate) fn fail_closed(&self) -> usize {
        self.names.len().saturating_sub(1)
    }

    /// The `mask_rules()` index of rule `lit`.
    pub(crate) fn rule(&self, lit: usize) -> usize {
        self.rule_of.get(lit).copied().unwrap_or(self.fail_closed())
    }

    /// Whether the masker passes bytes through unchanged (test hooks only).
    pub(crate) fn is_clear(&self) -> bool {
        #[cfg(feature = "test-hooks")]
        return self.clear;
        #[cfg(not(feature = "test-hooks"))]
        false
    }

    fn len(&self) -> usize {
        self.shared.len() + self.extra.len()
    }

    /// The rules whose candidate `u` can start, in `lit` order.
    fn starts(&self, u: Unit) -> &[usize] {
        dispatch_slot(u)
            .and_then(|slot| self.starts.get(slot))
            .map_or(&[], Vec::as_slice)
    }

    /// The compiled rules in priority order.
    pub(crate) fn rules(&self) -> impl Iterator<Item = &Pattern> {
        self.shared
            .iter()
            .chain(self.extra.iter().map(|e| &e.pattern))
    }

    fn form(&self, lit: usize) -> Option<&Form> {
        match SHAPES.get(lit) {
            Some(shape) => Some(&shape.form),
            None => self.extra.get(lit - SHAPES.len()).map(|e| &e.form),
        }
    }

    fn guarded(&self, lit: usize) -> bool {
        self.shared.get(lit).is_some_and(|p| p.guard)
    }

    /// Whether a quoted JSON key makes `"KEY": "…"` a secret assignment, asked of the shared
    /// JSON literal itself with a fixed sixteen-character value.
    fn json_key_is_secret(&self, key: &[u8]) -> bool {
        let Some(p) = self.shared.get(JSON_LITERAL) else {
            return true;
        };
        let mut probe = Vec::with_capacity(key.len() + 24);
        probe.push(b'"');
        probe.extend_from_slice(key);
        probe.extend_from_slice(b"\":\"0123456789abcdef\"");
        p.re.find(&probe).is_some_and(|m| m.start() == 0)
    }
}

/// Slots of the dispatch table: the 128 ASCII units, then `ſ` and the Kelvin sign, the two
/// non-ASCII units a `(?i)` rule folds to ASCII. No rule starts on any other unit.
const DISPATCH_SLOTS: usize = 130;

fn dispatch_slot(u: Unit) -> Option<usize> {
    match u {
        LONG_S => Some(128),
        KELVIN => Some(129),
        _ => ascii(u).map(usize::from),
    }
}

/// The unit a dispatch slot stands for.
fn slot_unit(slot: usize) -> Unit {
    match slot {
        128 => LONG_S,
        129 => KELVIN,
        _ => u8::try_from(slot).map_or(INVALID, char::from),
    }
}

/// The index of `kind` in `names`, added at the end when new.
fn name_index(names: &mut Vec<String>, kind: &str) -> usize {
    match names.iter().position(|n| n == kind) {
        Some(i) => i,
        None => {
            names.push(kind.to_string());
            names.len() - 1
        }
    }
}

/// The shared literal of the PEM block rule.
fn pem_literal() -> Option<usize> {
    SHAPES
        .iter()
        .position(|shape| matches!(shape.form, Form::PrivateKey))
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
/// (`rules::PEM_BODY_MAX`, which says why 15.5 KiB): an END line must start within them, and
/// without one the block is them. A block this long, with a BEGIN and an END line, fits the
/// hold, so a lone BEGIN is decided below it.
///
/// The 512 bytes left are room for a BEGIN line and an END line with real labels (the longest,
/// `-----BEGIN ENCRYPTED PRIVATE KEY-----`, is 37 bytes). The label is `[A-Z ]*`, unbounded: a
/// BEGIN line longer than about 450 bytes holds its block past the hold, and the stream fails
/// closed there. That redacts more than the whole-record policy, never less.
const PEM_BODY_MAX: u32 = rules::PEM_BODY_MAX as u32;

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
    /// Just after `=` with no space: the form-encoded literal's value may start here.
    Direct,
    /// Inside the form-encoded literal's value (see [`encoded_step`]).
    Encoded {
        /// Token-class units so far, capped at sixteen.
        run: u8,
        /// True while every unit so far is a token-class unit.
        token_only: bool,
        /// Hex digits still owed by a `%XX` escape.
        hex: u8,
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
enum MaskWalk {
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

impl MaskWalk {
    fn step(&mut self, u: Unit) -> Step {
        let next = match *self {
            MaskWalk::Prefix(p) => {
                let wanted = match p {
                    0 => folded(u) == Some(b's'),
                    1 => folded(u) == Some(b'k'),
                    _ => is(u, b'-'),
                };
                if !wanted {
                    return Step::Dead;
                }
                if p == 2 {
                    MaskWalk::Run(0)
                } else {
                    MaskWalk::Prefix(p + 1)
                }
            }
            MaskWalk::Run(n) if word_folded(u) || is(u, b'-') => {
                if n == 8 {
                    return Step::Dead;
                }
                MaskWalk::Run(n + 1)
            }
            MaskWalk::Run(n) if n >= 1 => match u {
                '.' => MaskWalk::Dots(1),
                '\u{2026}' => MaskWalk::Tail(0),
                '\\' => MaskWalk::Escape(1),
                '*' => MaskWalk::Stars(1),
                _ => return Step::Dead,
            },
            MaskWalk::Run(_) => return Step::Dead,
            MaskWalk::Dots(k) if is(u, b'.') && k < 3 => MaskWalk::Dots(k + 1),
            MaskWalk::Dots(3) if word_folded(u) => MaskWalk::Tail(1),
            MaskWalk::Dots(_) => return Step::Dead,
            MaskWalk::Escape(p) => {
                let wanted = MASK_ESCAPE.get(p as usize).map(u8::to_ascii_lowercase);
                if folded(u) != wanted {
                    return Step::Dead;
                }
                if p as usize + 1 == MASK_ESCAPE.len() {
                    MaskWalk::Tail(0)
                } else {
                    MaskWalk::Escape(p + 1)
                }
            }
            MaskWalk::Stars(k) if is(u, b'*') && k < 64 => MaskWalk::Stars(k + 1),
            MaskWalk::Stars(k) if k >= 3 && word_folded(u) => MaskWalk::Tail(1),
            MaskWalk::Stars(_) => return Step::Dead,
            MaskWalk::Tail(m) if word_folded(u) => {
                // The fourth tail unit completes a match that cannot extend.
                if m == 3 {
                    return Step::Last;
                }
                MaskWalk::Tail(m + 1)
            }
            // Past three tail units a non-tail unit ends a complete match before it; before
            // that it ends nothing.
            MaskWalk::Tail(_) => return Step::Dead,
        };
        *self = next;
        Step::Live
    }

    /// The separator is complete: held bytes from the start already look like a masked key.
    fn past_separator(&self) -> bool {
        matches!(
            self,
            MaskWalk::Dots(3) | MaskWalk::Stars(3..=64) | MaskWalk::Tail(_)
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
    Masked(MaskWalk),
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
    Url(Url),
    Field(Field),
    Header(Header),
    /// How far a [`Form::Literal`] matched, a line break in progress included.
    Literal(Progress),
}

/// A [`Form::UrlPassword`] candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Url {
    /// In `(?:\\{0,3}/){2}`: slashes done (0 or 1), backslashes before the next slash.
    Slashes { done: u8, backslashes: u8 },
    /// User units so far.
    User(u16),
    /// Password units so far, and where the password (capture group 1) starts.
    Password { n: u16, from: u64 },
}

impl Url {
    /// `after` is the offset after this unit.
    fn step(&mut self, u: Unit, after: u64) -> Step {
        let next = match *self {
            Url::Slashes { done, backslashes } if is(u, b'\\') && backslashes < 3 => Url::Slashes {
                done,
                backslashes: backslashes + 1,
            },
            Url::Slashes { done: 0, .. } if is(u, b'/') => Url::Slashes {
                done: 1,
                backslashes: 0,
            },
            Url::Slashes { .. } if is(u, b'/') => Url::User(0),
            Url::Slashes { .. } => return Step::Dead,
            Url::User(_) if is(u, b':') => Url::Password { n: 0, from: after },
            Url::User(n) if url_user(u) && n < URL_USER_MAX => Url::User(n + 1),
            Url::User(_) => return Step::Dead,
            Url::Password { n, .. } if is(u, b'@') && n >= 1 => return Step::Last,
            Url::Password { n, from } if url_password_char(u) && n < URL_PASSWORD_MAX => {
                Url::Password { n: n + 1, from }
            }
            Url::Password { .. } => return Step::Dead,
        };
        *self = next;
        Step::Live
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldPhase {
    /// This many bytes of the quoted key matched.
    Key(u8),
    /// The first `\s*`.
    BeforeColon,
    /// The second `\s*`.
    AfterColon,
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
    fn step(&mut self, key: &[u8], scalar: bool, u: Unit, off: u64, len: usize) -> Step {
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
            FieldPhase::AfterColon if !scalar && is(u, b'"') => {
                self.value_start = off + len as u64;
                FieldPhase::String(false)
            }
            // `"` opens the first alternative and is also a character of the second.
            FieldPhase::AfterColon if scalar && scalar_char(u) => {
                self.value_start = off;
                FieldPhase::Scalar {
                    quoted: is(u, b'"'),
                    bare: true,
                }
            }
            FieldPhase::AfterColon => return Step::Dead,
            FieldPhase::String(some) => match u {
                '"' if some => return Step::Last,
                _ if string_char(u) => FieldPhase::String(true),
                _ => return Step::Dead,
            },
            FieldPhase::Scalar { quoted, bare } => {
                if quoted && is(u, b'"') {
                    return Step::Last;
                }
                let quoted = quoted && string_char(u);
                let bare = bare && scalar_char(u);
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
    fn step(&mut self, policy: &Policy, u: Unit, after: u64) -> bool {
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
                let mut buf = [0u8; 4];
                self.key
                    .extend_from_slice(u.encode_utf8(&mut buf).as_bytes());
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
                let secret = !self.key.is_empty() && policy.json_key_is_secret(&self.key);
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
                '\\' => {
                    self.phase = JsonPhase::ValueEnd(1);
                    true
                }
                '"' => false,
                _ => {
                    self.n = (self.n + 1).min(16);
                    true
                }
            },
            // Backslashes after a value character: one to seven, then either one character that
            // is not a quote (an escaped unit of the value, `\/`), or the quote that closes it.
            // The closing quote completes the match, which the regex then sees.
            JsonPhase::ValueEnd(k) if is(u, b'\\') => match more(k) {
                Some(k) => {
                    self.phase = JsonPhase::ValueEnd(k);
                    true
                }
                None => false,
            },
            JsonPhase::ValueEnd(_) if is(u, b'"') => false,
            JsonPhase::ValueEnd(_) => {
                self.phase = JsonPhase::Value;
                self.n = (self.n + 1).min(16);
                true
            }
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
            Shell::BeforeSep if is(u, b'=') => Shell::Direct,
            Shell::BeforeSep if is(u, b':') => Shell::AfterSep,
            // Only a `=` with no space after it can start the form-encoded value.
            Shell::Direct if space(u) => Shell::AfterSep,
            Shell::BeforeSep | Shell::AfterSep if space(u) => return true,
            Shell::AfterSep | Shell::Direct if is(u, b'\\') => Shell::Escape(1),
            Shell::Escape(k) if is(u, b'\\') && k < MAX_QUOTE_ESCAPES => Shell::Escape(k + 1),
            Shell::AfterSep | Shell::Direct | Shell::Escape(_) if is(u, b'\'') || is(u, b'"') => {
                Shell::Quoted
            }
            Shell::AfterSep | Shell::Quoted if token_folded(u) => Shell::Value(1),
            Shell::Direct => match encoded_step(0, true, 0, u) {
                Some(next) => next,
                None => return false,
            },
            Shell::Encoded {
                run,
                token_only,
                hex,
            } => match encoded_step(run, token_only, hex, u) {
                Some(next) => next,
                None => return false,
            },
            Shell::Value(n) if token_folded(u) => Shell::Value((n + 1).min(16)),
            _ => return false,
        };
        *self = next;
        true
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
    /// The candidate a unit starts for literal `lit`, before that unit is stepped. A guarded
    /// literal's candidate is not started when `glued` says the unit is glued to a word.
    fn spawn(policy: &Policy, lit: usize, u: Unit) -> Option<Walk> {
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
            Form::UrlPassword => is(u, b'/') || is(u, b'\\'),
            Form::Field { key, .. } => key.first().is_some_and(|&b| is(u, b)),
            Form::HeaderPrefix { prefix } => prefix.first().is_some_and(|&b| is(u, b)),
            Form::Literal { form } => policy
                .secrets
                .form(*form)
                .first()
                .is_some_and(|&b| is(u, b)),
        };
        if !starts {
            return None;
        }
        Some(match form {
            Form::Prefixed(_) => Walk::Prefixed { pos: 0, count: 0 },
            Form::MaskedKey => Walk::Masked(MaskWalk::Prefix(0)),
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
            Form::UrlPassword => Walk::Url(Url::Slashes {
                done: 0,
                backslashes: 0,
            }),
            Form::Field { .. } => Walk::Field(Field {
                phase: FieldPhase::Key(0),
                value_start: 0,
            }),
            Form::HeaderPrefix { .. } => Walk::Header(Header::Lit(0)),
            Form::Literal { .. } => Walk::Literal(Progress::default()),
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
            (Walk::Json(json), _) => json.step(policy, u, off + len as u64),
            (Walk::Shell(shell), _) => shell.step(u),
            (Walk::Url(url), _) => return url.step(u, off + len as u64),
            (Walk::Field(field), Form::Field { key, scalar }) => {
                return field.step(key, *scalar, u, off, len);
            }
            (Walk::Header(header), Form::HeaderPrefix { prefix }) => {
                return header.step(prefix, u);
            }
            // A literal cannot extend once complete: its last byte ends it.
            (Walk::Literal(progress), Form::Literal { form }) => {
                let Some(b) = ascii(u) else {
                    return Step::Dead;
                };
                return match progress.step(policy.secrets.form(*form), b) {
                    Advance::Live => Step::Live,
                    Advance::Complete => Step::Last,
                    Advance::Dead => Step::Dead,
                };
            }
            _ => false,
        })
    }

    /// Where held bytes of this candidate already look like secret material, for a
    /// discontinuity that can no longer complete it. `tail` is where the stream's undecoded
    /// bytes (an incomplete UTF-8 sequence) start, when it has any.
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
            (Walk::Shell(Shell::Value(_) | Shell::Encoded { .. }), _) => Some(self.start),
            (Walk::Url(Url::Password { from, .. }), _) => Some(*from),
            (Walk::Field(field), Form::Field { scalar, .. }) => match field.phase {
                FieldPhase::String(_) | FieldPhase::Scalar { .. } => Some(field.value_start),
                // A bare scalar value can begin with a multi-byte character cut inside its
                // encoding: the undecoded bytes are the value's first bytes.
                FieldPhase::AfterColon if *scalar => tail,
                FieldPhase::Key(_) | FieldPhase::BeforeColon | FieldPhase::AfterColon => None,
            },
            (Walk::Header(Header::Value { .. }), _) => Some(self.start),
            (Walk::Literal(progress), _) => (progress.matched() >= 1).then_some(self.start),
            _ => None,
        }
    }
}

// ── one stream's undecided bytes ───────────────────────────────────────────────────────

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

/// A fail-closed window in progress: every byte from `start` is starred until a `\n` arrives
/// that no candidate begun inside the window reaches, or a message end, a gap or the end. Only
/// recognizer state is kept, never the bytes.
struct Redacting {
    /// Candidates of every literal that began inside the window and are still live.
    live: Vec<Candidate>,
    /// Too many live candidates to follow: then only a message end, a gap or the end ends it.
    saturated: bool,
    /// The end of a block released in part ([`OpenBlock`]): the window does not end before.
    until: u64,
}

/// A lone `BEGIN`'s window the stream released in part. Its `BEGIN` has left the hold, so the
/// regexes no longer find it; its rest, from `held_start` to `end`, is claimed from the quote
/// state there, as the whole-record rules claim it (`rules::block_pieces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenBlock {
    /// Where its `BEGIN` was: the cut point is `origin + rules::PEM_SLICE`.
    origin: u64,
    end: u64,
    /// The quote state at `held_start`.
    quotes: rules::Quotes,
}

/// The last raw bytes before the hold, for the left guard: in memory only, cleared at a gap
/// and at a message end.
#[derive(Debug, Clone, Copy, Default)]
struct Ring {
    buf: [u8; GUARD_BYTES],
    len: u8,
}

impl Ring {
    fn bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }

    fn push(&mut self, bytes: &[u8]) {
        let mut next = [0u8; GUARD_BYTES];
        let n = rules::guard_context(self.bytes(), bytes, &mut next);
        self.buf = next;
        self.len = n as u8;
    }
}

/// An incomplete UTF-8 sequence at the end of the input so far: at most three bytes.
#[derive(Debug, Clone, Copy, Default)]
struct Tail {
    buf: [u8; 3],
    len: u8,
}

impl Tail {
    fn bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }
}

/// The bounded state of one stream: held raw bytes (`held_start .. decoded_end`), an
/// incomplete UTF-8 tail after them and live candidates, or a fail-closed window in progress
/// (then nothing is held). Offsets count the bytes pushed since the stream began.
pub(crate) struct Scanner {
    policy: Arc<Policy>,
    held: Vec<u8>,
    held_start: u64,
    decoded_end: u64,
    tail: Tail,
    ring: Ring,
    candidates: Vec<Candidate>,
    redacting: Option<Box<Redacting>>,
    open_block: Option<OpenBlock>,
}

impl std::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scanner")
            .field("held_start", &self.held_start)
            .field("candidates", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

/// Star `len` more bytes of `out` as `fail_closed`, joined to a `fail_closed` run that ends
/// where they start.
fn star(out: &mut Masked, raw_len: usize, rule: usize) {
    if raw_len == 0 {
        return;
    }
    let at = out.bytes.len();
    out.bytes.resize(at + raw_len, b'*');
    match out.spans.last_mut() {
        Some(last) if last.rule == rule && last.start + last.len == at => last.len += raw_len,
        _ => out.spans.push(Span {
            start: at,
            len: raw_len,
            rule,
        }),
    }
}

impl Scanner {
    pub(crate) fn new(policy: Arc<Policy>) -> Scanner {
        Scanner {
            policy,
            held: Vec::new(),
            held_start: 0,
            decoded_end: 0,
            tail: Tail::default(),
            ring: Ring::default(),
            candidates: Vec::new(),
            redacting: None,
            open_block: None,
        }
    }

    pub(crate) fn policy(&self) -> &Policy {
        &self.policy
    }

    /// The source offset of the next byte this scanner expects.
    fn end_offset(&self) -> u64 {
        self.decoded_end + u64::from(self.tail.len)
    }

    /// Whether nothing is held: every byte pushed so far has been returned.
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        self.held.len() + usize::from(self.tail.len)
    }

    pub(crate) fn feed(&mut self, data: &[u8], out: &mut Masked) {
        let joined;
        let buf: &[u8] = if self.tail.len == 0 {
            data
        } else {
            joined = [self.tail.bytes(), data].concat();
            &joined
        };
        let base = self.decoded_end;
        self.tail = Tail::default();
        let policy = Arc::clone(&self.policy);
        let mut pos = 0usize;
        while pos < buf.len() {
            let (u, len) = match decode(&buf[pos..]) {
                Decoded::Incomplete => {
                    let rest = &buf[pos..];
                    self.tail.buf[..rest.len()].copy_from_slice(rest);
                    self.tail.len = rest.len() as u8;
                    break;
                }
                Decoded::Unit(u, len) => (u, len),
            };
            let off = base + pos as u64;
            if self.redacting.is_some() {
                if self.redact_unit(&policy, u, &buf[pos..pos + len], off, out) {
                    pos += len;
                }
                // Otherwise the terminator is dispatched again, outside the window.
                continue;
            }
            if self.held.len() + len > MAX_HELD_BYTES {
                self.release(out);
                if self.held.len() + len > MAX_HELD_BYTES {
                    self.fail_closed(out);
                    continue;
                }
            }
            self.held.extend_from_slice(&buf[pos..pos + len]);
            self.decoded_end = off + len as u64;
            self.step_unit(&policy, u, off, len);
            pos += len;
            if self.candidates.len() > MAX_CANDIDATES {
                self.fail_closed(out);
            }
        }
        if self.redacting.is_none() {
            self.release(out);
        }
        if self.held.is_empty() {
            // Idle: no buffer is kept (section 6). No candidate outlives the bytes it holds.
            self.held = Vec::new();
            self.candidates = Vec::new();
        }
    }

    /// Whether the unit just before `held[..at]` ends is glued, from the ring and the hold.
    fn glued_at(&self, at: usize) -> bool {
        let mut buf = [0u8; GUARD_BYTES];
        let n = rules::guard_context(self.ring.bytes(), &self.held[..at], &mut buf);
        rules::glued(&buf[..n])
    }

    /// Start every candidate this unit starts; `glued` says the bytes before it are glued.
    fn spawn_all(
        policy: &Policy,
        u: Unit,
        off: u64,
        len: usize,
        mut glued: impl FnMut() -> bool,
        into: &mut Vec<Candidate>,
    ) {
        let mut is_glued: Option<bool> = None;
        for &lit in policy.starts(u) {
            let Some(walk) = Candidate::spawn(policy, lit, u) else {
                continue;
            };
            if policy.guarded(lit) && *is_glued.get_or_insert_with(&mut glued) {
                continue;
            }
            let mut c = Candidate {
                lit,
                start: off,
                walk,
            };
            if c.step(policy, u, off, len) == Step::Live {
                into.push(c);
            }
        }
    }

    /// One unit during a fail-closed window. True when the unit is starred; false when the
    /// window ended before it (a `\n` no live candidate includes).
    fn redact_unit(
        &mut self,
        policy: &Policy,
        u: Unit,
        raw: &[u8],
        off: u64,
        out: &mut Masked,
    ) -> bool {
        let Some(red) = self.redacting.as_mut() else {
            return false;
        };
        let len = raw.len();
        if !red.saturated {
            let included = Self::follow(policy, red, self.ring, u, off, len);
            if is(u, b'\n') && !included && off >= red.until {
                self.end_window(off);
                return false;
            }
        }
        star(out, len, policy.fail_closed());
        self.ring.push(raw);
        self.decoded_end = off + len as u64;
        true
    }

    /// Steps the window's live candidates over one unit and starts new ones; whether a live
    /// candidate includes the unit. Past [`MAX_CANDIDATES`] the window is saturated, and only
    /// a message end, a gap or the end can close it, so nothing is followed any more.
    fn follow(
        policy: &Policy,
        red: &mut Redacting,
        ring: Ring,
        u: Unit,
        off: u64,
        len: usize,
    ) -> bool {
        let mut included = false;
        red.live.retain_mut(|c| {
            let step = c.step(policy, u, off, len);
            included |= step != Step::Dead;
            step == Step::Live
        });
        Self::spawn_all(
            policy,
            u,
            off,
            len,
            || rules::glued(ring.bytes()),
            &mut red.live,
        );
        dedup(&mut red.live);
        if red.live.len() > MAX_CANDIDATES {
            red.live = Vec::new();
            red.saturated = true;
        }
        included
    }

    fn step_unit(&mut self, policy: &Policy, u: Unit, off: u64, len: usize) {
        self.candidates
            .retain_mut(|c| c.step(policy, u, off, len) == Step::Live);
        let at = self.held.len() - len;
        let mut spawned = Vec::new();
        Self::spawn_all(policy, u, off, len, || self.glued_at(at), &mut spawned);
        self.candidates.append(&mut spawned);
        dedup(&mut self.candidates);
    }

    /// Release `held[..n]` into `out` with `runs` (offsets in the hold) starred.
    fn emit(&mut self, n: usize, runs: Vec<Run>, out: &mut Masked) {
        let bytes = &self.held[..n];
        fill(&self.policy, bytes, runs, out);
        self.ring.push(bytes);
        self.held.drain(..n);
        self.held_start += n as u64;
    }

    /// Every match of every rule in `held[..n]`, and the pieces of a block released in part
    /// ([`OpenBlock`]) there; the index where those pieces begin.
    fn found(&self, n: usize) -> (Vec<Claim>, usize) {
        let policy = &self.policy;
        let mut found = candidates(policy.rules(), &self.held[..n], self.ring.bytes());
        let first = found.len();
        let (Some(ob), Some(lit)) = (self.open_block, pem_literal()) else {
            return (found, first);
        };
        let base = self.held_start;
        let view = rules::match_view(&self.held[..n]);
        let whole = (self.open_end(ob) - base) as usize;
        let cut = (ob.origin + rules::PEM_SLICE as u64)
            .checked_sub(base)
            .map(|c| c as usize);
        for (start, end) in rules::block_pieces(&view, 0, whole.min(n), ob.quotes, cut) {
            found.push(Claim {
                priority: lit,
                start,
                end,
                whole_start: 0,
                whole_end: whole,
                soft: false,
                block: true,
                bounded: true,
            });
        }
        (found, first)
    }

    /// Where a block released in part ends: its own end, or a `BEGIN` held inside it.
    fn open_end(&self, ob: OpenBlock) -> u64 {
        let base = self.held_start;
        let to = ((ob.end - base) as usize).min(self.held.len());
        rules::first_begin(&rules::match_view(&self.held), 0, to)
            .map_or(ob.end, |at| base + at as u64)
    }

    /// The selected claims of `held[..n]` as runs.
    fn runs(&self, n: usize) -> Vec<Run> {
        selected(self.found(n).0)
    }

    /// Release every byte before the earliest live candidate: a clean cut, where no match
    /// of any rule can cross whatever arrives next, and not right after a backslash, so the
    /// quote cut and escape alignment read the same bytes the whole-record rules read.
    ///
    /// A lone `BEGIN`'s window that is decided does not hold the cut back to its start: the
    /// cut moves only to the start of the piece it falls in, and the rest of the window is
    /// carried as an [`OpenBlock`].
    fn release(&mut self, out: &mut Masked) {
        if self.held.is_empty() {
            return;
        }
        let end = self.decoded_end;
        let base = self.held_start;
        let mut cut = self.candidates.iter().map(|c| c.start).min().unwrap_or(end);
        if cut == base {
            return;
        }
        let scanned = cut < end || self.open_block.is_some();
        let (found, open_from) = if scanned {
            self.found(self.held.len())
        } else {
            (Vec::new(), 0)
        };
        let pem = pem_literal();
        let decided = |i: usize, m: &Claim| {
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
            while cut > base && self.held[(cut - base) as usize - 1] == b'\\' {
                cut -= 1;
                moved = true;
            }
            if !moved {
                break;
            }
        }
        if cut == base {
            return;
        }
        let n = (cut - base) as usize;
        let carry = carried.map(|i| (i, &found[i])).map(|(i, m)| {
            let view = rules::match_view(&self.held[..n]);
            let (origin, quotes) = match self.open_block.filter(|_| i >= open_from) {
                Some(ob) => (ob.origin, ob.quotes.after(&view)),
                None => (
                    base + m.whole_start as u64,
                    rules::Quotes::START.after(&view[m.whole_start..]),
                ),
            };
            OpenBlock {
                origin,
                end: base + m.whole_end as u64,
                quotes,
            }
        });
        // No match crosses the cut and no block is carried, so the matches of `held[..n]` are
        // those of the whole hold that end by `n`: the hold is not searched twice.
        let runs = if scanned && self.open_block.is_none() && carry.is_none() {
            selected(found.into_iter().filter(|c| c.whole_end <= n).collect())
        } else {
            self.runs(n)
        };
        self.emit(n, runs, out);
        self.open_block = carry;
    }

    /// The hold is full and some candidate is still undecided: fail closed. The bytes before
    /// the earliest undecided byte leave with every match over them starred; from there every
    /// byte is starred as `fail_closed`.
    fn fail_closed(&mut self, out: &mut Masked) {
        let base = self.held_start;
        let until = self.open_block.map_or(0, |ob| self.open_end(ob));
        let Some(mut start) = self.candidates.iter().map(|c| c.start).min() else {
            // No candidate: release the hold, but keep one byte of an odd trailing backslash
            // run, so the rest of the run keeps the parity the whole input has.
            let tail = self.held.iter().rev().take_while(|&&b| b == b'\\').count();
            let n = self.held.len() - tail % 2;
            let runs = self.runs(n);
            return self.emit(n, runs, out);
        };
        // Every match in the hold, not only this window's claims; a match crossing the start
        // moves it back.
        let (raw, _) = self.found(self.held.len());
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
        let before: Vec<Run> = raw
            .iter()
            .filter(|m| base + (m.end as u64) <= start)
            .map(|m| Run {
                start: m.start,
                end: m.end,
                lit: m.priority,
            })
            .collect();
        self.emit((start - base) as usize, before, out);
        star(out, self.held.len(), self.policy.fail_closed());
        self.ring.push(&self.held);
        self.held.clear();
        self.held_start = self.decoded_end;
        self.open_block = None;
        self.redacting = Some(Box::new(Redacting {
            live: std::mem::take(&mut self.candidates),
            saturated: false,
            until,
        }));
    }

    /// The fail-closed window ends before `end`; what follows starts afresh.
    fn end_window(&mut self, end: u64) {
        self.redacting = None;
        self.held.clear();
        self.held_start = end;
        self.decoded_end = end;
        self.candidates = Vec::new();
        self.open_block = None;
    }

    /// Every held byte is decided as end of input.
    pub(crate) fn finish_complete(&mut self, out: &mut Masked) {
        let end = self.end_offset();
        let tail = self.tail;
        self.tail = Tail::default();
        if self.redacting.is_some() {
            star(out, tail.bytes().len(), self.policy.fail_closed());
            self.ring.push(tail.bytes());
            self.end_window(end);
            return;
        }
        self.held.extend_from_slice(tail.bytes());
        self.decoded_end = end;
        let n = self.held.len();
        let runs = self.runs(n);
        self.emit(n, runs, out);
        self.candidates = Vec::new();
        self.open_block = None;
    }

    /// A discontinuity: nothing after it can complete a held candidate. Held bytes that
    /// already look like secret material are starred to the cut; the rest is decided as end
    /// of input.
    pub(crate) fn finish_cut(&mut self, out: &mut Masked) {
        if self.redacting.is_some() {
            return self.finish_complete(out);
        }
        let end = self.end_offset();
        let tail = (self.tail.len > 0).then_some(self.decoded_end);
        let bytes = self.tail;
        self.tail = Tail::default();
        self.held.extend_from_slice(bytes.bytes());
        self.decoded_end = end;
        let base = self.held_start;
        let mut runs = self.runs(self.held.len());
        self.open_block = None;
        let policy = Arc::clone(&self.policy);
        let mut pick: Option<(u64, usize)> = None;
        for c in &self.candidates {
            if let Some(x) = c.secret_from(&policy, tail)
                && pick.is_none_or(|p| (x, c.lit) < p)
            {
                pick = Some((x, c.lit));
            }
        }
        self.candidates = Vec::new();
        if let Some((x, lit)) = pick {
            runs.push(Run {
                start: (x.max(base) - base) as usize,
                end: (end - base) as usize,
                lit,
            });
        }
        let n = self.held.len();
        self.emit(n, runs, out);
    }

    /// After a message end or a gap: the next bytes start like a new stream (the offsets go
    /// on), and the guard ring is cleared.
    pub(crate) fn restart(&mut self) {
        self.end_window(self.end_offset());
        self.held = Vec::new();
        self.ring = Ring::default();
    }
}

/// The claims [`select`] keeps of `found`, as runs.
fn selected(found: Vec<Claim>) -> Vec<Run> {
    select(found)
        .iter()
        .map(|c| Run {
            start: c.start,
            end: c.end,
            lit: c.priority,
        })
        .collect()
}

/// Append `bytes` to `out` with `runs` (offsets in `bytes`) finished and starred.
fn fill(policy: &Policy, bytes: &[u8], runs: Vec<Run>, out: &mut Masked) {
    let at = out.bytes.len();
    out.bytes.extend_from_slice(bytes);
    for r in rules::finish_spans(bytes, runs) {
        let end = r.end.min(bytes.len());
        if r.start >= end {
            continue;
        }
        out.bytes[at + r.start..at + end].fill(b'*');
        out.spans.push(Span {
            start: at + r.start,
            len: end - r.start,
            rule: policy.rule(r.lit),
        });
    }
}

/// The whole-record masker: the oracle of the stream, with the same rules and guard and never
/// a `fail_closed` span.
pub(crate) fn mask_whole(policy: &Policy, bytes: &[u8]) -> Masked {
    let runs = selected(candidates(policy.rules(), bytes, &[]));
    let mut out = Masked::default();
    fill(policy, bytes, runs, &mut out);
    out
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
        b"\\\"api_key\\\": \\\"",
        b"\\\"",
        b"password: \\\"",
        b"\"token\" : \"",
        b"token = '",
        b"Bearer  ",
        b"eyJabcdefghij.",
        b"-----BEGIN RSA PRIVATE KEY-----\n",
        b"SK-ab",
        b"***",
        b"...",
        "\u{2026}".as_bytes(),
        b"\\u2026",
        b"wxyz",
        b"\\n",
        b"\\\\n",
        b"%3D",
        b"\x1b[32m",
        b"\\u001b[1m",
        b"x",
        b"_",
        b"//",
        b"https://",
        b"\\/\\/",
        b"u:",
        b":p@",
        b"postgres://app:",
        b"?",
        b"#",
        b"<",
        "\u{e9}".as_bytes(),
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

    fn policy() -> Result<Arc<Policy>, MaskError> {
        Ok(Arc::new(Policy::new(&[], &[])?))
    }

    /// Every recognizer is pinned to the exact source of the `shapes.tsv` line it models, and
    /// the masker refuses to start when they differ.
    #[test]
    fn shapes_follow_the_shared_list() -> Result<(), MaskError> {
        let tsv: Vec<(&str, &str)> = rules::rows()
            .map(|r| r.map_or(("", ""), |r| (r.kind, r.source)))
            .collect();
        let shapes: Vec<(&str, &str)> = SHAPES.iter().map(|s| (s.kind, s.source)).collect();
        assert_eq!(tsv, shapes);
        policy()?;
        Ok(())
    }

    /// Every connector form maps onto its recognizer with the span its regex declares: a JSON
    /// field's value is group 1, a header line the whole match. A connector kind that would
    /// take the name of `run_secret` or `fail_closed` is refused, so `mask_rules()` names each
    /// index once.
    #[test]
    fn every_rule_form_streams_with_its_span() -> Result<(), MaskError> {
        let rule = |kind: &str, pattern| MaskRule {
            kind: kind.into(),
            pattern,
        };
        let field = |name: &str, value| MaskPattern::JsonField {
            field: name.into(),
            value,
        };
        let all = [
            rule("t_string", field("Tier-2", FieldValue::String)),
            rule("t_scalar", field("k", FieldValue::Scalar)),
            rule("t_header", MaskPattern::HeaderPrefix("x-a".into())),
        ];
        let policy = Policy::new(&all, &[])?;
        assert_eq!(policy.len(), SHAPES.len() + all.len());
        for (lit, group) in (SHAPES.len()..).zip([1, 1, 0]) {
            let extra = &policy.extra[lit - SHAPES.len()];
            let lead = match &extra.form {
                Form::Field { key, .. } => key,
                Form::HeaderPrefix { prefix } => prefix,
                _ => panic!("rule {lit} has no connector recognizer"),
            };
            let lead = regex::escape(&String::from_utf8_lossy(lead));
            assert!(extra.pattern.re.as_str().starts_with(&lead));
            assert_eq!(extra.pattern.span_group, group);
        }
        for name in [RUN_SECRET_KIND, FAIL_CLOSED_KIND] {
            let bad = rule(name, MaskPattern::HeaderPrefix("x-a".into()));
            assert_eq!(Policy::new(&[bad], &[]).err(), Some(MaskError::Rule(0)));
        }
        Ok(())
    }

    /// Soundness of the hold against the rules themselves: for every kept match of every rule,
    /// every proper prefix keeps a candidate of that rule starting at or before the match (or
    /// is still undecoded), and no byte of it is released. Precision: once a few units after a
    /// match are decoded, its candidate is gone.
    #[test]
    fn recognizers_hold_every_prefix_of_a_match() -> Result<(), MaskError> {
        let policy = policy()?;
        let mut checked = 0usize;
        for seed in 1..=2500u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let text = random_text(&mut rng);
            let view = rules::match_view(&text);
            let mut matches = Vec::new();
            for (lit, p) in policy.rules().enumerate() {
                for m in p.re.find_iter(&view) {
                    if p.guard && rules::glued(&text[..m.start()]) {
                        continue;
                    }
                    matches.push((lit, m.start() as u64, m.end() as u64));
                }
            }
            let mut scanner = Scanner::new(Arc::clone(&policy));
            let mut sink = Masked::default();
            for k in 1..=text.len() {
                scanner.feed(&text[k - 1..k], &mut sink);
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
                            "seed {seed}: rule {lit} match {s}..{t} not held at {k}"
                        );
                        assert!(
                            scanner.held_start <= s,
                            "seed {seed}: a live match released"
                        );
                    }
                    if k >= t + 4 && k < t + 5 {
                        checked += 1;
                        let gone = !scanner
                            .candidates
                            .iter()
                            .any(|c| c.lit == lit && c.start == s);
                        assert!(gone, "seed {seed}: rule {lit} match {s}..{t} held at {k}");
                    }
                }
            }
            assert_eq!(
                sink.bytes.len() as u64 + scanner.held_bytes() as u64,
                text.len() as u64
            );
        }
        assert!(
            checked > 5_000,
            "the generator reaches the rules: {checked}"
        );
        Ok(())
    }

    /// Whatever a long candidate is, the hold and the candidates stay within their bounds
    /// after every push, and every byte comes back.
    #[test]
    fn held_state_stays_bounded() -> Result<(), MaskError> {
        let bodies: [(&[u8], &[u8]); 8] = [
            (
                b"-----BEGIN RSA PRIVATE KEY-----\n",
                b"MIIEowIBAAKCAQEA+/=\n",
            ),
            (b"{\"", b"abcdefghijklmnopqrstuvwxyz"),
            (b"{\"api_key\":\"", b"abc/+= xyz"),
            (b"bearer", b" \t "),
            (b"eyJ", b"eyJeyJeyJ0123456789."),
            (b"", b"\"a\"b\"token\":\""),
            (b"-----BEGIN ", b"RSA PRIVATE "),
            (b"https://", b"user.name+tag"),
        ];
        let policy = policy()?;
        for (lead, unit) in bodies {
            let mut s = Scanner::new(Arc::clone(&policy));
            let mut out = Masked::default();
            let mut fed = 0;
            let mut data = lead.to_vec();
            for _ in 0..4 {
                while data.len() < 3 * MAX_HELD_BYTES / 2 {
                    data.extend_from_slice(unit);
                }
                fed += data.len();
                s.feed(&std::mem::take(&mut data), &mut out);
                assert!(s.held_bytes() <= MAX_HELD_BYTES + 3);
                assert!(s.candidates.len() <= MAX_CANDIDATES);
            }
            s.finish_complete(&mut out);
            assert_eq!(out.bytes.len(), fed);
        }
        Ok(())
    }
}
