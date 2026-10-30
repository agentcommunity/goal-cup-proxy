//! The whole-record matcher (PRD-05 sections 2 to 4): the shared literals of `shapes.tsv`
//! (seam S12), the connector rules and the exact values, each found on the raw bytes, resolved
//! into claimed spans, then cut at quotes, aligned to escapes and characters, and starred.
//!
//! Copied from the 0.x `goalcup-daemon` redactor (`redact.rs`): `match_view`, `candidates`,
//! `select` and the PEM pieces are as they were. New here: the left guard ([`glued`]), the
//! same-length fill ([`finish_spans`]) in place of the marker, and the connector rule type.
//!
//! **No interpretation.** A match is a byte comparison against a regular expression, never a
//! judgement about what the text means.

use regex::bytes::Regex;
use std::sync::OnceLock;

/// The shared literals as data (S12). The verifier includes the same file.
pub(crate) const SHAPES_TSV: &str = include_str!("shapes.tsv");

/// The kind of the one soft rule: it loses every overlap and keeps what the winner did not
/// cover (see [`select`]).
pub(crate) const SOFT_KIND: &str = "masked_key";

/// The kind of the one block rule of the shared list: a PEM private key, from its `BEGIN` line
/// to the first `END` line that starts within [`PEM_BODY_MAX`] bytes, else those bytes. A
/// block absorbs every hard claim it contains or that crosses its edge (see [`select`]).
pub(crate) const BLOCK_KIND: &str = "private_key";

/// The most bytes the `private_key` rule takes after its `BEGIN` line. Both arms of the
/// literal spell it (`{0,15872}`). 15.5 KiB holds an RSA-16384 body as Claude Code's Read tool
/// prints it, escaped twice; it stays 512 bytes under the stream's 16 KiB hold, room for a
/// `BEGIN` and an `END` line, so a block is decided below the hold.
pub(crate) const PEM_BODY_MAX: usize = 15_872;

/// Where a lone `BEGIN`'s window is cut in two, so the stream may release the first part and
/// hold the rest. Half the hold.
pub(crate) const PEM_SLICE: usize = 8 * 1024;

/// How many bytes before a match the left guard reads.
pub(crate) const GUARD_BYTES: usize = 24;

/// What an invalid byte reads as when the rules match: one ASCII `0`. Every key body class
/// takes it and no shared rule's fixed prefix starts or completes with it, so a key with an
/// invalid byte inside its body is still one match.
pub(crate) const INVALID_BYTE_AS: u8 = b'0';

/// One data line of `shapes.tsv`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Row {
    pub(crate) kind: &'static str,
    pub(crate) guard: bool,
    pub(crate) source: &'static str,
}

/// Every data line of `shapes.tsv` in order, `None` for a line that is not `kind`, `guard`
/// (`0` or `1`) and a regex separated by tabs.
pub(crate) fn rows() -> impl Iterator<Item = Option<Row>> {
    SHAPES_TSV
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split('\t');
            let (kind, guard, source) = (fields.next()?, fields.next()?, fields.next()?);
            if fields.next().is_some() || kind.is_empty() || source.is_empty() {
                return None;
            }
            let guard = match guard {
                "1" => true,
                "0" => false,
                _ => return None,
            };
            Some(Row {
                kind,
                guard,
                source,
            })
        })
}

/// One compiled rule. `Debug` never prints the regex: an exact value's source spells the value.
pub(crate) struct Pattern {
    pub(crate) re: Regex,
    /// `0` for the whole match, `1` for the first capture group (the regex has one).
    pub(crate) span_group: usize,
    /// Loses every overlap (see [`select`]).
    pub(crate) soft: bool,
    /// A match is one whole block that no claim inside it or across its edge may shadow.
    pub(crate) block: bool,
    /// The PEM block rule: its matches are cut into string pieces ([`pem_pieces`]).
    pub(crate) pem: bool,
    /// The left guard applies ([`glued`]).
    pub(crate) guard: bool,
}

impl std::fmt::Debug for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pattern")
            .field("span_group", &self.span_group)
            .field("soft", &self.soft)
            .field("block", &self.block)
            .field("guard", &self.guard)
            .finish_non_exhaustive()
    }
}

/// Group 1 when the regex declares one, otherwise the whole match.
fn span_group_of(re: &Regex) -> usize {
    usize::from(re.captures_len() > 1)
}

impl Pattern {
    /// A shared literal, or `None` when its regex does not compile.
    pub(crate) fn shared(row: &Row) -> Option<Pattern> {
        let re = Regex::new(row.source).ok()?;
        Some(Pattern {
            span_group: span_group_of(&re),
            soft: row.kind == SOFT_KIND,
            block: row.kind == BLOCK_KIND,
            pem: row.kind == BLOCK_KIND,
            guard: row.guard,
            re,
        })
    }

    /// An exact-value form: a block with no group and no guard.
    pub(crate) fn exact(re: Regex) -> Pattern {
        Pattern {
            re,
            span_group: 0,
            soft: false,
            block: true,
            pem: false,
            guard: false,
        }
    }
}

/// A connector rule (PRD-11 data, seam S3) as the masker holds it. `object` values are not a
/// form here: a rule cannot say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnRule {
    pub(crate) kind: String,
    pub(crate) pattern: ConnPattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnPattern {
    /// `"name": value`: a non-empty string without escapes, or with `scalar` a string without
    /// escapes, else a bare token.
    Field { name: String, scalar: bool },
    /// `prefix[a-z0-9-]+: value`
    HeaderPrefix(String),
}

impl ConnRule {
    /// The regex the rule stands for, or `None` when its kind or name is outside the grammar
    /// (`kind` `^[a-z][a-z0-9_]{0,47}$`, a field 1 to 64 bytes of `[A-Za-z0-9_-]`, a prefix 1 to
    /// 64 bytes of `[a-z0-9-]` starting with a letter). The recognizer compares names byte for
    /// byte, which the grammar makes safe.
    pub(crate) fn regex(&self) -> Option<String> {
        let mut k = self.kind.bytes();
        let kind_ok = self.kind.len() <= 48
            && k.next().is_some_and(|b| b.is_ascii_lowercase())
            && k.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !kind_ok {
            return None;
        }
        match &self.pattern {
            ConnPattern::Field { name, scalar } => {
                let ok = (1..=64).contains(&name.len())
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
                let value = if *scalar {
                    r#"("[^"\\]*"|[^,}\s]+)"#
                } else {
                    r#""([^"\\]+)""#
                };
                ok.then(|| format!(r#""{}"\s*:\s*{value}"#, regex::escape(name)))
            }
            ConnPattern::HeaderPrefix(prefix) => {
                let ok = (1..=64).contains(&prefix.len())
                    && prefix.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                    && prefix.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
                    });
                ok.then(|| format!(r#"{}[a-z0-9-]+:\s*[^\r\n"]+"#, regex::escape(prefix)))
            }
        }
    }

    /// The compiled rule, or `None` when the rule is outside the grammar.
    pub(crate) fn compile(&self) -> Option<Pattern> {
        let re = Regex::new(&self.regex()?).ok()?;
        Some(Pattern {
            span_group: span_group_of(&re),
            soft: false,
            block: false,
            pem: false,
            guard: false,
            re,
        })
    }
}

/// The bytes the rules match on: `bytes`, with every byte that is not part of a valid UTF-8
/// sequence read as [`INVALID_BYTE_AS`]. One byte stands for one byte, so every offset is an
/// offset in `bytes`. Borrowed when `bytes` is valid UTF-8, which is nearly every record.
pub(crate) fn match_view(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let Err(mut error) = std::str::from_utf8(bytes) else {
        return std::borrow::Cow::Borrowed(bytes);
    };
    let mut view = bytes.to_vec();
    let mut at = 0;
    loop {
        let from = at + error.valid_up_to();
        let bad = error.error_len().unwrap_or(bytes.len() - from);
        view[from..from + bad].fill(INVALID_BYTE_AS);
        at = from + bad;
        match std::str::from_utf8(&bytes[at..]) {
            Ok(_) => return std::borrow::Cow::Owned(view),
            Err(next) => error = next,
        }
    }
}

// ── the left guard ─────────────────────────────────────────────────────────────────────

/// Whether a guarded match whose raw bytes before it are `before` is glued to a word: the byte
/// just before it is one of `[A-Za-z0-9_]` and ends no escape (PRD-05 section 4). Reads at most
/// the last [`GUARD_BYTES`]. Nothing before the match (start of input, after a gap, after a
/// message end) is never glued. Never `\b`: that misses keys after `\n` in JSON text.
pub(crate) fn glued(before: &[u8]) -> bool {
    let before = &before[before.len().saturating_sub(GUARD_BYTES)..];
    let Some(&last) = before.last() else {
        return false;
    };
    (last.is_ascii_alphanumeric() || last == b'_') && !ends_an_escape(before)
}

/// Whether the last byte of `b` ends a backslash escape (one or more backslashes, then a letter
/// or digit, or `u` and 4 hex, or `x` and 2 hex), a percent escape, or an ANSI colour code.
fn ends_an_escape(b: &[u8]) -> bool {
    let n = b.len();
    let Some(&last) = b.last() else {
        return false;
    };
    let backslash_before = |i: usize| i > 0 && b[i - 1] == b'\\';
    let hex_tail = |k: usize| n >= k && b[n - k..].iter().all(u8::is_ascii_hexdigit);
    if last.is_ascii_alphanumeric() && backslash_before(n - 1) {
        return true;
    }
    if hex_tail(4) && n >= 5 && b[n - 5] == b'u' && backslash_before(n - 5) {
        return true;
    }
    if hex_tail(2) && n >= 3 && (b[n - 3] == b'%' || (b[n - 3] == b'x' && backslash_before(n - 3)))
    {
        return true;
    }
    if !last.is_ascii_alphabetic() {
        return false;
    }
    // ESC `[`, up to 16 of `[0-9;]`, then the letter.
    let mut i = n - 1;
    let mut params = 0;
    while i > 0 && params < 16 && (b[i - 1].is_ascii_digit() || b[i - 1] == b';') {
        i -= 1;
        params += 1;
    }
    if i == 0 || b[i - 1] != b'[' {
        return false;
    }
    let head = &b[..i - 1];
    if head.last() == Some(&0x1b) {
        return true;
    }
    [&b"u001b"[..], b"u001B", b"x1b", b"x1B", b"033", b"e"]
        .iter()
        .any(|intro| head.ends_with(intro) && backslash_before(head.len() - intro.len()))
}

/// The last up-to-[`GUARD_BYTES`] bytes of `a` then `b`, into `buf`; their count.
pub(crate) fn guard_context(a: &[u8], b: &[u8], buf: &mut [u8; GUARD_BYTES]) -> usize {
    let from_b = b.len().min(GUARD_BYTES);
    let from_a = (GUARD_BYTES - from_b).min(a.len());
    buf[..from_a].copy_from_slice(&a[a.len() - from_a..]);
    buf[from_a..from_a + from_b].copy_from_slice(&b[b.len() - from_b..]);
    from_a + from_b
}

// ── candidates and selection ───────────────────────────────────────────────────────────

/// One interval a rule matched: a candidate before selection, or a claimed span after
/// [`select`]. `priority` is the rule's position in the set it was found with; among hard
/// claims a lower number wins an overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Claim {
    pub(crate) priority: usize,
    /// The span: the whole match, or the rule's capture group.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The whole match, which contains the span.
    pub(crate) whole_start: usize,
    pub(crate) whole_end: usize,
    pub(crate) soft: bool,
    pub(crate) block: bool,
    /// A piece of a PEM block that ended at its bound or at a later `BEGIN`, with no `END`.
    pub(crate) bounded: bool,
}

/// Every match of every rule in `set` over `bytes`, in the rule's span form. `before` is the
/// raw bytes just before `bytes` (at most [`GUARD_BYTES`] are read), for the left guard.
///
/// The rules match on [`match_view`]`(bytes)`. A guarded match glued to the bytes before it is
/// dropped and the search goes on from the byte after its start, so a match that starts inside
/// it is still found, as the stream's recognizers find it.
pub(crate) fn candidates<'a>(
    set: impl Iterator<Item = &'a Pattern>,
    bytes: &[u8],
    before: &[u8],
) -> Vec<Claim> {
    let raw = bytes;
    let view = match_view(bytes);
    let bytes: &[u8] = &view;
    let mut found_all = Vec::new();
    let claim = |priority, (start, end), (whole_start, whole_end), p: &Pattern| Claim {
        priority,
        start,
        end,
        whole_start,
        whole_end,
        soft: p.soft,
        block: p.block,
        bounded: false,
    };
    for (priority, p) in set.enumerate() {
        if p.pem {
            for ((from, to), (start, end), bounded) in pem_pieces(&p.re, bytes) {
                found_all.push(Claim {
                    bounded,
                    ..claim(priority, (from, to), (start, end), p)
                });
            }
            continue;
        }
        if p.guard {
            let mut at = 0;
            while let Some(m) = p.re.find_at(bytes, at) {
                let mut buf = [0u8; GUARD_BYTES];
                let n = guard_context(before, &raw[..m.start()], &mut buf);
                if glued(&buf[..n]) {
                    at = m.start() + 1;
                    continue;
                }
                let span = (m.start(), m.end());
                found_all.push(claim(priority, span, span, p));
                at = m.end().max(m.start() + 1);
            }
            continue;
        }
        type Found = ((usize, usize), (usize, usize));
        let found: Box<dyn Iterator<Item = Found> + '_> = if p.span_group == 0 {
            Box::new(p.re.find_iter(bytes).map(|m| {
                let span = (m.start(), m.end());
                (span, span)
            }))
        } else {
            let g = p.span_group;
            Box::new(p.re.captures_iter(bytes).filter_map(move |c| {
                let whole = c.get(0)?;
                let span = c.get(g).unwrap_or(whole);
                Some(((span.start(), span.end()), (whole.start(), whole.end())))
            }))
        };
        for ((start, mut end), (whole_start, mut whole_end)) in found {
            if p.block {
                end = char_end(bytes, end);
                whole_end = whole_end.max(end);
            }
            found_all.push(claim(priority, (start, end), (whole_start, whole_end), p));
        }
    }
    found_all
}

/// The `BEGIN` line of the `private_key` rule, alone, and its `END` line at the end of a match.
fn pem_lines() -> Option<&'static (Regex, Regex)> {
    static LINES: OnceLock<Option<(Regex, Regex)>> = OnceLock::new();
    LINES
        .get_or_init(|| {
            let begin = Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").ok()?;
            let end = Regex::new(r"-----END [A-Z ]*PRIVATE KEY-----\z").ok()?;
            Some((begin, end))
        })
        .as_ref()
}

/// The first `BEGIN` line that starts in `from..to` of `bytes` (it may end after `to`).
pub(crate) fn first_begin(bytes: &[u8], from: usize, to: usize) -> Option<usize> {
    pem_lines()?
        .0
        .find_at(bytes, from)
        .map(|m| m.start())
        .filter(|&at| at < to)
}

/// Every `private_key` match in `bytes` as `(start, end, bounded)`, left to right. A match
/// without an `END` line (`bounded`) ends at the first later `BEGIN` inside it, and the search
/// goes on from that `BEGIN`, so a real key late in a lone mention's window gets its own window.
pub(crate) fn pem_matches(re: &Regex, bytes: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(m) = re.find_at(bytes, at) {
        let start = m.start();
        let bounded = pem_lines().is_none_or(|lines| !lines.1.is_match(m.as_bytes()));
        let (end, next) = match first_begin(bytes, start + 1, m.end()).filter(|_| bounded) {
            Some(begin) => (begin, begin),
            None => {
                let end = char_end(bytes, m.end());
                (end, end.max(start + 1))
            }
        };
        out.push((start, end, bounded));
        at = next;
    }
    out
}

/// The quote state at a point of a block: inside a string or not, and whether an odd run of
/// backslashes ends just before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Quotes {
    pub(crate) inside: bool,
    pub(crate) escaped: bool,
}

impl Quotes {
    /// At a block's `BEGIN`: text, so inside a string.
    pub(crate) const START: Quotes = Quotes {
        inside: true,
        escaped: false,
    };

    /// This state after `bytes`.
    pub(crate) fn after(mut self, bytes: &[u8]) -> Quotes {
        for &b in bytes {
            if b == b'"' && !self.escaped {
                self.inside = !self.inside;
            }
            self.escaped = b == b'\\' && !self.escaped;
        }
        self
    }
}

/// The claimed pieces of a PEM block's `from..end`: [`string_pieces_from`], then, when `cut`
/// (a bounded block's `BEGIN` plus [`PEM_SLICE`]) falls strictly inside a piece, that piece in
/// two. The cut first moves past a run of backslashes and the byte they escape, then to the end
/// of a character, so no escape and no character is split.
pub(crate) fn block_pieces(
    bytes: &[u8],
    from: usize,
    end: usize,
    quotes: Quotes,
    cut: Option<usize>,
) -> Vec<(usize, usize)> {
    let mut pieces = string_pieces_from(bytes, from, end, quotes);
    if let Some(mut cut) = cut {
        while cut > 0 && cut < end && bytes[cut - 1] == b'\\' {
            cut += 1;
        }
        if cut < bytes.len() {
            cut = char_end(bytes, cut);
        }
        if let Some(i) = pieces.iter().position(|&(a, b)| a < cut && cut < b) {
            let (a, b) = pieces[i];
            pieces.splice(i..=i, [(a, cut), (cut, b)]);
        }
    }
    pieces
}

type PemPiece = ((usize, usize), (usize, usize), bool);

/// The claimed pieces of every `private_key` match in `bytes`, as `(piece, whole match,
/// bounded)`, a bounded one cut once at [`PEM_SLICE`].
pub(crate) fn pem_pieces(re: &Regex, bytes: &[u8]) -> Vec<PemPiece> {
    let mut out = Vec::new();
    for (start, end, bounded) in pem_matches(re, bytes) {
        let cut = bounded.then_some(start + PEM_SLICE);
        for piece in block_pieces(bytes, start, end, Quotes::START, cut) {
            out.push((piece, (start, end), bounded));
        }
    }
    out
}

/// The parts of a PEM block's range that are claimed, from a point whose quote state is
/// `quotes`: the range is cut at every unescaped `"`, and every piece inside a string is kept,
/// so a key streamed as many string deltas loses every delta's text and keeps the JSON between
/// them. A piece loses the backslashes it ends on. A piece outside the strings is kept too when
/// it holds a key-shaped run ([`holds_a_base64_run`]): raw text whose quotes do not pair.
pub(crate) fn string_pieces_from(
    bytes: &[u8],
    start: usize,
    end: usize,
    quotes: Quotes,
) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut push = |from: usize, mut to: usize| {
        while to > from && bytes[to - 1] == b'\\' {
            to -= 1;
        }
        if to > from {
            pieces.push((from, to));
        }
    };
    let (mut inside, mut from, mut slashes) = (quotes.inside, start, usize::from(quotes.escaped));
    for (at, &b) in bytes.iter().enumerate().take(end).skip(start) {
        if b == b'"' && slashes % 2 == 0 {
            if inside || holds_a_base64_run(&bytes[from..at]) {
                push(from, at);
            }
            from = at + 1;
            inside = !inside;
        }
        slashes = if b == b'\\' { slashes + 1 } else { 0 };
    }
    if inside || holds_a_base64_run(&bytes[from..end]) {
        push(from, end);
    }
    pieces
}

/// Whether `bytes` holds a run of sixteen or more base64 characters with a letter in it other
/// than `e` or `E` (a number's exponent does not count): a line of a key's body.
pub(crate) fn holds_a_base64_run(bytes: &[u8]) -> bool {
    let (mut run, mut letter) = (0usize, false);
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') {
            run += 1;
            letter |= b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E');
            if run >= 16 && letter {
                return true;
            }
        } else {
            (run, letter) = (0, false);
        }
    }
    false
}

/// The width of the valid UTF-8 character that starts at `p`, if one does.
fn char_at(bytes: &[u8], p: usize) -> Option<usize> {
    let width = match *bytes.get(p)? {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    bytes
        .get(p..p + width)
        .is_some_and(|c| std::str::from_utf8(c).is_ok())
        .then_some(width)
}

/// The end of the UTF-8 character that `end` falls inside, or `end` on a boundary.
pub(crate) fn char_end(bytes: &[u8], end: usize) -> usize {
    for p in end.saturating_sub(3)..end {
        if let Some(width) = char_at(bytes, p)
            && p + width > end
        {
            return p + width;
        }
    }
    end
}

/// The start of the UTF-8 character that `start` falls inside, or `start` on a boundary.
fn char_start(bytes: &[u8], start: usize) -> usize {
    for p in start.saturating_sub(3)..start {
        if let Some(width) = char_at(bytes, p)
            && p + width > start
        {
            return p;
        }
    }
    start
}

/// Resolve every rule's candidates into the claimed spans, in source order, never overlapping.
///
/// The hard rules are resolved first-wins, by list position and then by source order: a
/// candidate that overlaps a claim already made is dropped whole. A block is then merged with
/// every candidate that overlaps it, selected or dropped, transitively, so a block is never
/// shadowed and never leaves the outside part of a claim raw. Then the one soft rule is cut
/// against the hard claims and only its uncovered pieces are kept, so it can only add masking.
pub(crate) fn select(candidates: Vec<Claim>) -> Vec<Claim> {
    if !candidates.iter().any(|c| c.soft) {
        return select_blocks(candidates);
    }
    let (mut soft, hard): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| c.soft);
    let mut claimed = select_blocks(hard);
    soft.sort_unstable_by_key(|c| (c.start, c.end));
    let mut remainders: Vec<Claim> = Vec::new();
    let mut from = 0;
    for candidate in soft {
        while from < claimed.len() && claimed[from].end <= candidate.start {
            from += 1;
        }
        let mut at = candidate.start;
        let mut next = from;
        while next < claimed.len() && claimed[next].start < candidate.end {
            if claimed[next].start > at {
                remainders.push(Claim {
                    start: at,
                    end: claimed[next].start,
                    ..candidate
                });
            }
            at = at.max(claimed[next].end);
            next += 1;
        }
        if at < candidate.end {
            remainders.push(Claim { start: at, ..candidate });
        }
    }
    claimed.extend(remainders);
    claimed.sort_unstable_by_key(|c| (c.start, c.end));
    claimed
}

/// The hard half of [`select`]: first-wins, then every block joined with what it touches.
fn select_blocks(candidates: Vec<Claim>) -> Vec<Claim> {
    if !candidates.iter().any(|c| c.block) {
        return select_hard(candidates);
    }
    let mut blocks: Vec<Claim> = candidates.iter().filter(|c| c.block).copied().collect();
    blocks.sort_unstable_by_key(|c| (c.start, c.end));
    let touching: Vec<Claim> = candidates
        .iter()
        .filter(|c| !c.block)
        .filter(|c| {
            let at = blocks.partition_point(|b| b.end <= c.start);
            blocks.get(at).is_some_and(|b| b.start < c.end)
        })
        .copied()
        .collect();
    let claimed = select_hard(candidates);
    absorb_into_blocks(blocks, touching, claimed)
}

/// Join every block with everything it overlaps, transitively: each connected group is one
/// claim from the earliest start to the latest end, with the block's priority. A claim
/// connected to no block passes through unchanged.
fn absorb_into_blocks(blocks: Vec<Claim>, touching: Vec<Claim>, claimed: Vec<Claim>) -> Vec<Claim> {
    let mut all = Vec::with_capacity(blocks.len() + touching.len() + claimed.len());
    all.extend(blocks);
    all.extend(touching);
    all.extend(claimed.into_iter().filter(|c| !c.block));
    all.sort_unstable_by_key(|c| (c.start, c.end, !c.block));
    let mut out: Vec<Claim> = Vec::with_capacity(all.len());
    let mut current: Option<Claim> = None;
    for c in all {
        current = match current {
            Some(mut group) if c.start < group.end => {
                if c.block && !group.block {
                    group.priority = c.priority;
                    group.block = true;
                }
                group.end = group.end.max(c.end);
                group.whole_start = group.whole_start.min(c.whole_start);
                group.whole_end = group.whole_end.max(c.whole_end);
                Some(group)
            }
            Some(group) => {
                out.push(group);
                Some(c)
            }
            None => Some(c),
        };
    }
    out.extend(current);
    out
}

/// First-wins claiming among hard rules, one priority group at a time against a source-sorted
/// claimed set, merged linearly (no sorted-Vec insertion, which went quadratic).
fn select_hard(mut candidates: Vec<Claim>) -> Vec<Claim> {
    candidates.sort_unstable_by_key(|c| (c.priority, c.start, c.end));
    let mut claimed: Vec<Claim> = Vec::new();
    let mut group_start = 0;
    while group_start < candidates.len() {
        let priority = candidates[group_start].priority;
        let mut group_end = group_start + 1;
        while group_end < candidates.len() && candidates[group_end].priority == priority {
            group_end += 1;
        }
        let mut admitted: Vec<Claim> = Vec::new();
        let mut claimed_at = 0;
        for candidate in &candidates[group_start..group_end] {
            while claimed_at < claimed.len() && claimed[claimed_at].end <= candidate.start {
                claimed_at += 1;
            }
            let overlaps_claimed =
                claimed_at < claimed.len() && claimed[claimed_at].start < candidate.end;
            let overlaps_admitted = admitted
                .last()
                .is_some_and(|last| last.end > candidate.start);
            if !overlaps_claimed && !overlaps_admitted {
                admitted.push(*candidate);
            }
        }
        let mut merged = Vec::with_capacity(claimed.len() + admitted.len());
        let (mut left, mut right) = (0, 0);
        while left < claimed.len() || right < admitted.len() {
            if right == admitted.len()
                || (left < claimed.len() && claimed[left].start <= admitted[right].start)
            {
                merged.push(claimed[left]);
                left += 1;
            } else {
                merged.push(admitted[right]);
                right += 1;
            }
        }
        claimed = merged;
        group_start = group_end;
    }
    claimed
}

// ── same-length fill ───────────────────────────────────────────────────────────────────

/// One starred run over a buffer: `start..end`, named by rule `lit` of the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) lit: usize,
}

/// How many backslashes end just before `at`.
fn backslashes_before(bytes: &[u8], at: usize) -> usize {
    bytes[..at].iter().rev().take_while(|&&b| b == b'\\').count()
}

/// The runs that rule spans star over `bytes` (PRD-05 section 3): each span is cut at every
/// `"` that an even run of backslashes precedes, and the quote is kept; a piece that starts
/// after an odd run of backslashes starts one byte earlier, and one that ends on an escaping
/// backslash ends one byte later; a piece over part of a character covers the character.
/// Overlapping runs then become one, named by the lowest rule.
pub(crate) fn finish_spans(bytes: &[u8], spans: impl IntoIterator<Item = Run>) -> Vec<Run> {
    let mut out = Vec::new();
    for span in spans {
        let end = span.end.min(bytes.len());
        let mut from = span.start;
        let mut slashes = backslashes_before(bytes, from);
        for at in span.start..end {
            let b = bytes[at];
            if b == b'"' && slashes % 2 == 0 {
                push_aligned(bytes, from, at, span.lit, &mut out);
                from = at + 1;
            }
            slashes = if b == b'\\' { slashes + 1 } else { 0 };
        }
        push_aligned(bytes, from, end, span.lit, &mut out);
    }
    union(out)
}

fn push_aligned(bytes: &[u8], start: usize, end: usize, lit: usize, out: &mut Vec<Run>) {
    if start >= end {
        return;
    }
    let mut start = start;
    let mut end = end;
    if backslashes_before(bytes, start) % 2 == 1 {
        start -= 1;
    }
    if end < bytes.len() && backslashes_before(bytes, end) % 2 == 1 {
        end += 1;
    }
    out.push(Run {
        start: char_start(bytes, start),
        end: char_end(bytes, end),
        lit,
    });
}

/// Overlapping runs become one, named by their lowest rule. Touching runs stay apart.
pub(crate) fn union(mut runs: Vec<Run>) -> Vec<Run> {
    runs.sort_by_key(|r| (r.start, r.lit));
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        if r.start >= r.end {
            continue;
        }
        match out.last_mut() {
            Some(last) if r.start < last.end => {
                last.end = last.end.max(r.end);
                last.lit = last.lit.min(r.lit);
            }
            _ => out.push(r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(priority: usize, start: usize, end: usize, block: bool) -> Claim {
        Claim {
            priority,
            start,
            end,
            whole_start: start,
            whole_end: end,
            soft: false,
            block,
            bounded: false,
        }
    }

    fn spans(claims: &[Claim]) -> Vec<(usize, usize, usize)> {
        claims.iter().map(|c| (c.start, c.end, c.priority)).collect()
    }

    /// Two blocks and the one claim across both edges become one span; a claim inside a block
    /// vanishes; a claim near no block is untouched; no block means plain first-wins.
    #[test]
    fn a_block_absorbs_what_it_contains_and_what_crosses_it() {
        let got = select(vec![
            claim(7, 0, 10, true),
            claim(7, 20, 30, true),
            claim(9, 8, 22, false),
            claim(3, 24, 26, false),
            claim(3, 40, 50, false),
            claim(7, 60, 70, true),
            claim(3, 62, 65, false),
        ]);
        assert_eq!(spans(&got), vec![(0, 30, 7), (40, 50, 3), (60, 70, 7)]);
        let got = select(vec![claim(7, 0, 10, true), claim(3, 10, 20, false)]);
        assert_eq!(spans(&got), vec![(0, 10, 7), (10, 20, 3)]);
        let got = select(vec![claim(3, 0, 10, false), claim(9, 5, 15, false)]);
        assert_eq!(spans(&got), vec![(0, 10, 3)]);
    }
}
