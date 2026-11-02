//! Exact values (PRD-05 section 9): the participant's own secrets, masked by their exact bytes
//! in every form they take, and the one value the default build reads, the project's
//! `GH_TOKEN`.
//!
//! Copied from the 0.x `goalcup-protocol` `run_secret.rs`. The forms of a value `v`:
//!
//! * `v` itself;
//! * `v` with every byte percent-encoded, in upper and in lower case hex;
//! * `v` with every byte JSON `\u`-escaped, in lower and in upper case hex;
//! * `v` inside a longer base64 text, standard and URL-safe alphabets, at each of the three
//!   byte alignments: the characters that depend on no byte outside `v` (unpadded);
//! * for a value holding `"` or `\`, `v` JSON-string escaped once (`\"`, `\\`) and twice
//!   (`\\\"`, `\\\\`), as it travels inside a JSON string and inside a JSON string nested in
//!   one (a `cat .env` tool result in a request body);
//! * for a value holding `<`, `>` or `&`, the same two with those bytes written `\u003c`,
//!   `\u003e` and `\u0026`, as an HTML-safe JSON encoder (Go's `encoding/json` by default)
//!   writes them, the other bytes raw (a `"` or `\` escaped as above).
//!
//! Equal forms count once, so a value has 11 forms, 13 with one escape kind and 15 at most. Not
//! covered (PRD-05 section 10, a stated loss): a value with both kinds, plainly escaped and then
//! nested HTML-safe, or the reverse.
//!
//! A form is found with a line break between any two of its characters: LF or CR LF, raw, or
//! escaped with one or two backslashes before each of `r` and `n`, at most one break per gap.
//! An escaped break is not taken right before a backslash of the form itself, so every text
//! matches a form in one way only. One trie over every form of every value ([`RunSecrets`])
//! finds them all in a single pass, with that break grammar in its steps; a plain literal
//! matcher cannot take a break inside a form. The test module keeps each form's regular
//! expression as the oracle that this finds the same matches.
//!
//! Values are read once, in memory only, and never written, hashed or logged.
//!
//! Sources (section 9, owner answer of 2026-11-01 to question 3, sources 1 to 3): the project's
//! `GH_TOKEN` (1), the credential-named variables of the harness environment goalcup builds
//! (2) and the other credential-named values of the project's key files (3).
//! Sources 2 and 3 pass the one name gate ([`gated`]). A harness key-file value (source 4) is
//! not built: it is left out and counted in `skipped`. The readers order, dedupe and cut the
//! values before the first control line ([`finish`]). This header is the one place that counts
//! the forms and states the reader order; `mod.rs` and PRD-05 link here.

use super::{SecretValue, ValueCounts, ValueSource};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// The shortest value used: a shorter one yields forms common in ordinary data.
pub const MIN_VALUE_BYTES: usize = 20;

/// The longest value used.
pub const MAX_VALUE_BYTES: usize = 256;

/// The longest form of a value: every byte HTML-safe JSON-escaped and nested once
/// (`\\u003c`), seven bytes each.
pub const MAX_FORM_BYTES: usize = 7 * MAX_VALUE_BYTES;

/// The longest break: CR LF with two backslashes before each letter (six bytes).
pub const MAX_BREAK_BYTES: usize = 6;

/// The longest text a form of `form_len` bytes can match.
pub const fn wrapped_len(form_len: usize) -> usize {
    form_len + MAX_BREAK_BYTES * form_len.saturating_sub(1)
}

/// The longest text any form of any value can match. The stream holds a form from its first
/// byte, so the hold must take this many.
pub const MAX_WRAPPED_FORM_BYTES: usize = wrapped_len(MAX_FORM_BYTES);

const _: () = assert!(MAX_WRAPPED_FORM_BYTES <= super::stream::MAX_HELD_BYTES);

/// The most values used (PRD-05 section 9): a stated guard on start-up time and memory, not a
/// cap of records. Each value adds up to fifteen forms to the trie.
pub const MAX_VALUES: usize = 32;

/// Whether `value` is one the masker uses: 20 to 256 bytes of printable ASCII without a space.
pub fn is_usable(value: &[u8]) -> bool {
    (MIN_VALUE_BYTES..=MAX_VALUE_BYTES).contains(&value.len())
        && value.iter().all(|b| (0x21..=0x7e).contains(b))
}

/// Every form of `value`, distinct, each at least eight bytes.
pub fn encoded_forms(value: &[u8]) -> Vec<Vec<u8>> {
    const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
    const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
    let mut forms: Vec<Vec<u8>> = vec![value.to_vec()];
    for table in [HEX_UPPER, HEX_LOWER] {
        let mut percent = Vec::with_capacity(value.len() * 3);
        let mut escaped = Vec::with_capacity(value.len() * 6);
        for &b in value {
            let (hi, lo) = (table[usize::from(b >> 4)], table[usize::from(b & 0x0f)]);
            percent.extend_from_slice(&[b'%', hi, lo]);
            escaped.extend_from_slice(&[b'\\', b'u', b'0', b'0', hi, lo]);
        }
        forms.push(percent);
        forms.push(escaped);
    }
    for url_safe in [false, true] {
        for alignment in 0..3 {
            forms.push(base64_inside(value, alignment, url_safe));
        }
    }
    for html in [false, true] {
        let escapes =
            |b: &u8| matches!(b, b'"' | b'\\') || (html && matches!(b, b'<' | b'>' | b'&'));
        if value.iter().any(escapes) {
            let once = json_escaped(value, html);
            let twice = json_escaped(&once, html);
            forms.push(once);
            forms.push(twice);
        }
    }
    forms.retain(|form| form.len() >= 8);
    forms.sort();
    forms.dedup();
    forms
}

/// `value` as the inside of a JSON string: a backslash before every `"` and `\`, and with
/// `html` each `<`, `>` and `&` as `\u003c`, `\u003e`, `\u0026`. A usable value is printable
/// ASCII, so nothing else is escaped.
fn json_escaped(value: &[u8], html: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 6);
    for &b in value {
        match b {
            b'"' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b'<' | b'>' | b'&' if html => out.extend_from_slice(format!("\\u{b:04x}").as_bytes()),
            _ => out.push(b),
        }
    }
    out
}

/// The base64 text `form` always produces inside a longer encoded stream at byte offset
/// `alignment` modulo three: the characters that depend on no byte outside it. Unpadded.
pub fn base64_inside(form: &[u8], alignment: usize, url_safe: bool) -> Vec<u8> {
    let table: &[u8; 64] = if url_safe {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut padded = vec![0u8; alignment];
    padded.extend_from_slice(form);
    let mut out = Vec::with_capacity(padded.len() * 4 / 3 + 4);
    for group in padded.chunks(3) {
        let b = [
            group[0],
            group.get(1).copied().unwrap_or(0),
            group.get(2).copied().unwrap_or(0),
        ];
        let sextets = [
            b[0] >> 2,
            ((b[0] & 0x03) << 4) | (b[1] >> 4),
            ((b[1] & 0x0f) << 2) | (b[2] >> 6),
            b[2] & 0x3f,
        ];
        // A partial group of `len` bytes carries `len * 8 / 6` whole sextets.
        let whole = match group.len() {
            3 => 4,
            len => len * 8 / 6,
        };
        out.extend(sextets.iter().take(whole).map(|s| table[*s as usize]));
    }
    let lead = (alignment * 8).div_ceil(6).min(out.len());
    out.split_off(lead)
}

/// The line break in progress in the gap before the next character of a form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Break {
    #[default]
    None,
    /// A raw CR: an LF must follow.
    Cr,
    /// A backslash: `n`, `r` or a second backslash must follow.
    Escape,
    /// Two backslashes: `n` or `r` must follow.
    Escape2,
    /// `\r` (or `\\r`): a backslash must follow.
    EscapeR,
    /// `\r\`: an `n` or a second backslash must follow.
    EscapeRBackslash,
    /// `\r\\`: an `n` must follow.
    EscapeRBackslash2,
    /// A whole break: the form's next character must follow.
    Done,
}

/// What one byte did to a match in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    Live,
    /// This byte was a form's last character and no match from this start goes on.
    Complete,
    Dead,
}

/// No node, no form.
const NONE: u32 = u32::MAX;

/// One distinct prefix of the forms: its children are `edges[first..first + count]`, sorted by
/// byte, and `end` is the form that ends here ([`NONE`] when none does). Forms are distinct, so
/// at most one ends at a node; a form may be a prefix of a longer one, so an end node can
/// still have children.
#[derive(Clone, Copy)]
struct Node {
    first: u32,
    count: u32,
    end: u32,
}

/// One way a text is matching the forms from one start: the prefix matched so far (`node`),
/// the line break in progress, and `no_backslash`, set when an escaped break began at a node
/// that has a backslash child, which may then not follow (so every text matches a form in
/// one way only). Node ids only, never a byte, so it may be printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    node: u32,
    brk: Break,
    no_backslash: bool,
}

impl State {
    const ROOT: State = State {
        node: 0,
        brk: Break::None,
        no_backslash: false,
    };
}

/// The forms of the exact values and one trie over them all, shared by the whole-record
/// matcher and the stream (kind `run_secret`). One left-to-right pass finds every form, each
/// with at most one line break in any gap. `Debug` prints a count.
#[derive(Default)]
pub(crate) struct RunSecrets {
    forms: Vec<Vec<u8>>,
    nodes: Vec<Node>,
    edges: Vec<(u8, u32)>,
    /// The root's child for each byte ([`NONE`] for none): every text position asks it.
    root: Vec<u32>,
}

impl std::fmt::Debug for RunSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunSecrets({} forms)", self.forms.len())
    }
}

impl RunSecrets {
    /// The forms of `values` and the counts for the start event. A count-only entry
    /// ([`SecretValue::left_out`], the readers' cut) adds its count to `skipped`. A value whose
    /// bytes equal one already used adds nothing and is counted nowhere. A harness value
    /// (source 4, not built), an unusable value, and every value once [`MAX_VALUES`] are in
    /// use are counted in `skipped`. A form equal to one of an earlier value is kept once. The
    /// readers' [`finish`] is the bound on the wire; this dedupe and cut is the backstop for
    /// callers that pass values directly.
    pub(crate) fn new(values: &[SecretValue]) -> (RunSecrets, ValueCounts) {
        let mut counts = ValueCounts::default();
        let mut forms: Vec<Vec<u8>> = Vec::new();
        let mut known: HashSet<Vec<u8>> = HashSet::new();
        let mut used: HashSet<&[u8]> = HashSet::new();
        for value in values {
            if value.left_out > 0 {
                counts.skipped = counts.skipped.saturating_add(value.left_out);
                continue;
            }
            if used.contains(value.bytes()) {
                continue;
            }
            if value.source() == ValueSource::Harness
                || used.len() >= MAX_VALUES
                || !is_usable(value.bytes())
            {
                counts.skipped = counts.skipped.saturating_add(1);
                continue;
            }
            used.insert(value.bytes());
            for form in encoded_forms(value.bytes()) {
                if known.insert(form.clone()) {
                    forms.push(form);
                }
            }
            match value.source() {
                ValueSource::Env => counts.env += 1,
                _ => counts.workspace += 1,
            }
        }
        (RunSecrets::build(forms), counts)
    }

    /// The trie of `forms`: one node per distinct prefix, each node's children contiguous.
    fn build(forms: Vec<Vec<u8>>) -> RunSecrets {
        let mut children: Vec<Vec<(u8, u32)>> = vec![Vec::new()];
        let mut ends: Vec<u32> = vec![NONE];
        for (f, form) in forms.iter().enumerate() {
            let mut node = 0usize;
            for &b in form {
                let known = children[node].iter().find(|(c, _)| *c == b).map(|e| e.1);
                node = match known {
                    Some(child) => child as usize,
                    None => {
                        let child = children.len();
                        children.push(Vec::new());
                        ends.push(NONE);
                        children[node].push((b, child as u32));
                        child
                    }
                };
            }
            ends[node] = f as u32;
        }
        // Number the nodes breadth first, so the shallow nodes every start visits sit together
        // in memory and stay in cache.
        let mut order: Vec<usize> = vec![0];
        let mut at = 0;
        while let Some(&node) = order.get(at) {
            order.extend(children[node].iter().map(|e| e.1 as usize));
            at += 1;
        }
        let mut id = vec![0u32; children.len()];
        for (new, &old) in order.iter().enumerate() {
            id[old] = new as u32;
        }
        let mut root = vec![NONE; 256];
        for &(b, child) in &children[0] {
            root[usize::from(b)] = id[child as usize];
        }
        let mut nodes = Vec::with_capacity(children.len());
        let mut edges = Vec::with_capacity(children.len());
        for &old in &order {
            let mut kids: Vec<(u8, u32)> = children[old]
                .iter()
                .map(|&(b, child)| (b, id[child as usize]))
                .collect();
            kids.sort_unstable();
            nodes.push(Node {
                first: edges.len() as u32,
                count: kids.len() as u32,
                end: ends[old],
            });
            edges.append(&mut kids);
        }
        RunSecrets {
            forms,
            nodes,
            edges,
            root,
        }
    }

    /// The number of forms.
    pub(crate) fn len(&self) -> usize {
        self.forms.len()
    }

    /// Whether no value gave a form.
    pub(crate) fn is_empty(&self) -> bool {
        self.forms.is_empty()
    }

    /// Form `i`'s bytes (the tests' oracle reads them).
    #[cfg(test)]
    pub(crate) fn form(&self, i: usize) -> &[u8] {
        self.forms.get(i).map_or(&[], Vec::as_slice)
    }

    fn children(&self, node: u32) -> &[(u8, u32)] {
        self.nodes.get(node as usize).map_or(&[], |n| {
            let first = n.first as usize;
            self.edges
                .get(first..first + n.count as usize)
                .unwrap_or(&[])
        })
    }

    fn child(&self, node: u32, b: u8) -> Option<u32> {
        if node == 0 {
            return self
                .root
                .get(usize::from(b))
                .copied()
                .filter(|&c| c != NONE);
        }
        let kids = self.children(node);
        let at = kids.binary_search_by_key(&b, |e| e.0).ok()?;
        kids.get(at).map(|e| e.1)
    }

    /// The states after byte `b` from state `s` (up to two: a backslash can be a form byte
    /// and begin an escaped break), and the form this byte completes, if any. The rules are
    /// those of each form's `wrapped_regex` (the test oracle): a form byte after no break or a
    /// whole break; after a matched character (never before the first), LF, CR LF, or one or
    /// two backslashes before each of `r` and `n`, an escaped break never right before a
    /// backslash of the form; one break per gap.
    fn next(&self, s: State, b: u8) -> ([Option<State>; 2], Option<usize>) {
        let mut out = [None, None];
        let mut done = None;
        let gap = |brk| Some(State { brk, ..s });
        match (s.brk, b) {
            (Break::None | Break::Done, _) => {
                let byte = if b == b'\\' && s.no_backslash {
                    None
                } else {
                    self.child(s.node, b)
                };
                if let Some(child) = byte {
                    let node = self.nodes.get(child as usize);
                    done = node
                        .map(|n| n.end)
                        .filter(|&f| f != NONE)
                        .map(|f| f as usize);
                    if node.is_some_and(|n| n.count > 0) {
                        out[0] = Some(State {
                            node: child,
                            brk: Break::None,
                            no_backslash: false,
                        });
                    }
                }
                if s.brk == Break::None && s.node != 0 {
                    out[1] = match b {
                        b'\n' => gap(Break::Done),
                        b'\r' => gap(Break::Cr),
                        b'\\' if self.children(s.node).iter().any(|e| e.0 != b'\\') => {
                            Some(State {
                                node: s.node,
                                brk: Break::Escape,
                                no_backslash: self.child(s.node, b'\\').is_some(),
                            })
                        }
                        _ => None,
                    };
                }
            }
            (Break::Cr, b'\n') => out[1] = gap(Break::Done),
            (Break::Escape, b'\\') => out[1] = gap(Break::Escape2),
            (Break::Escape | Break::Escape2, b'n') => out[1] = gap(Break::Done),
            (Break::Escape | Break::Escape2, b'r') => out[1] = gap(Break::EscapeR),
            (Break::EscapeR, b'\\') => out[1] = gap(Break::EscapeRBackslash),
            (Break::EscapeRBackslash, b'\\') => out[1] = gap(Break::EscapeRBackslash2),
            (Break::EscapeRBackslash | Break::EscapeRBackslash2, b'n') => {
                out[1] = gap(Break::Done);
            }
            _ => {}
        }
        (out, done)
    }

    /// Every match of every form in `view`, as (form, start, end), in one left-to-right pass:
    /// per form the leftmost matches that do not overlap, sorted by form and start. A text
    /// matches a form from one start in one way only, so this is what each form's
    /// `wrapped_regex` finds, one by one (the test oracle).
    pub(crate) fn find_all(&self, view: &[u8]) -> Vec<(usize, usize, usize)> {
        let mut found = Vec::new();
        if self.is_empty() {
            return found;
        }
        let mut live: Vec<(State, usize)> = Vec::new();
        let mut next: Vec<(State, usize)> = Vec::new();
        for (i, &b) in view.iter().enumerate() {
            for &(s, start) in live.iter().chain(std::iter::once(&(State::ROOT, i))) {
                let (out, done) = self.next(s, b);
                if let Some(f) = done {
                    found.push((f, start, i + 1));
                }
                next.extend(out.into_iter().flatten().map(|s| (s, start)));
            }
            std::mem::swap(&mut live, &mut next);
            next.clear();
        }
        found.sort_unstable();
        let mut last_end = vec![0usize; self.forms.len()];
        found.retain(|&(f, start, end)| match last_end.get_mut(f) {
            Some(last) if start >= *last => {
                *last = end;
                true
            }
            _ => false,
        });
        found
    }

    /// The match a text starting with byte `b` begins, before `b` is stepped; `None` when no
    /// form starts with `b`.
    pub(crate) fn spawn(&self, b: u8) -> Option<LiteralWalk> {
        self.child(0, b)
            .map(|_| LiteralWalk(Walks::One(State::ROOT)))
    }
}

/// Every way a text is matching the forms from one start, for the stream's hold. Node ids
/// only, never a byte, so it may be printed; two walks with the same states are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiteralWalk(Walks);

/// One state (nearly always), or several in order after a backslash forked one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Walks {
    One(State),
    Many(Vec<State>),
}

impl LiteralWalk {
    /// Feed the next byte of the text. Live while any state lives (a form may have completed
    /// on this byte all the same); Complete when none lives and a form completed on it.
    pub(crate) fn step(&mut self, secrets: &RunSecrets, b: u8) -> Advance {
        let mut complete = false;
        let mut one: Option<State> = None;
        let mut more: Vec<State> = Vec::new();
        let mut take = |s: State| {
            let (out, done) = secrets.next(s, b);
            complete |= done.is_some();
            for s in out.into_iter().flatten() {
                if let Some(earlier) = one.replace(s) {
                    more.push(earlier);
                }
            }
        };
        match &self.0 {
            Walks::One(s) => take(*s),
            Walks::Many(all) => all.iter().copied().for_each(&mut take),
        }
        let live = one.is_some();
        self.0 = match one {
            Some(s) if more.is_empty() => Walks::One(s),
            _ => {
                more.extend(one);
                more.sort_unstable();
                more.dedup();
                if more.len() == 1 {
                    Walks::One(more[0])
                } else {
                    Walks::Many(more)
                }
            }
        };
        match (live, complete) {
            (true, _) => Advance::Live,
            (false, true) => Advance::Complete,
            (false, false) => Advance::Dead,
        }
    }

    /// Whether some state has matched at least one character of a form.
    pub(crate) fn matched_any(&self) -> bool {
        match &self.0 {
            Walks::One(s) => s.node != 0,
            Walks::Many(states) => states.iter().any(|s| s.node != 0),
        }
    }
}

/// The variable the participant keeps their GitHub token in.
const TOKEN_VARIABLE: &str = "GH_TOKEN";

/// The key file the guide tells the participant to keep it in.
const TOKEN_FILE: &str = ".env.local";

/// The credential-named variables of `env`, the environment goalcup builds for the harness
/// (source 2, `connector::LaunchPlan.env`), behind the name gate.
fn env_values(env: &BTreeMap<String, String>) -> Vec<SecretValue> {
    env.iter()
        .filter(|(name, value)| gated(name, value))
        .map(|(_, value)| SecretValue::new(ValueSource::Env, value.clone().into_bytes()))
        .collect()
}

/// Every exact value of a launch, through [`finish`]: the project's `GH_TOKEN` value (source 1),
/// every other credential-named value of its key files (source 3), all `workspace`, then the
/// credential-named variables of `env` (source 2). Pass an empty `env` for the key files alone.
pub fn launch_values(project: &Path, env: &BTreeMap<String, String>) -> Vec<SecretValue> {
    let (token, others) = key_file_values(project);
    finish(token.into_iter().chain(others).chain(env_values(env)))
}

/// The environment the running process `pid` was started with, the values that are valid text,
/// read from `/proc/<pid>/environ`. `goalcup recover` passes it to [`launch_values`] so the
/// recovered recorder masks the values the live harness holds, not those of the terminal that
/// runs `recover`. None where the system has no such file or it cannot be read (macOS, a
/// `hidepid` mount): the caller then has only its own environment (PRD-05 section 10, a stated
/// loss). Nothing read is printed.
pub fn process_env(pid: u32) -> Option<BTreeMap<String, String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(
        bytes
            .split(|b| *b == 0)
            .filter_map(|pair| {
                let at = pair.iter().position(|b| *b == b'=')?;
                let name = std::str::from_utf8(&pair[..at]).ok()?;
                let value = std::str::from_utf8(&pair[at + 1..]).ok()?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect(),
    )
}

/// What a reader sends on the first control line. The order is `GH_TOKEN`, the other key-file
/// values, then the environment values, so the values the participant keeps in `.env` win the
/// cut on a busy shell. A value whose bytes equal an earlier one is dropped uncounted (a value in
/// both the environment and `.env` counts once, as `workspace`), the first [`MAX_VALUES`]
/// distinct values are kept, and when any were left out one count-only entry
/// ([`SecretValue::left_out`]) carries their number into `skipped`. So the line holds at most 33
/// entries, each at most 256 value bytes (twice that when every byte is a JSON-escaped `"` or
/// `\`), and the whole start line stays below `watchdog::MAX_LINE` whatever the size of the key
/// files (`mask_values.rs`, `the_start_line_with_32_long_values_fits`).
fn finish(values: impl IntoIterator<Item = SecretValue>) -> Vec<SecretValue> {
    let mut kept: Vec<SecretValue> = Vec::new();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut left_out: u32 = 0;
    for value in values {
        if !seen.insert(value.bytes().to_vec()) {
            continue;
        }
        if kept.len() < MAX_VALUES {
            kept.push(value);
        } else {
            left_out = left_out.saturating_add(1);
        }
    }
    if left_out > 0 {
        kept.push(SecretValue::left_out(left_out));
    }
    kept
}

/// The key files (`.env`, and `.env.` and anything) that are regular files, symlinks followed,
/// so a FIFO or a device never blocks start. They are read in order: `.env.local` first, example
/// and sample files last, the rest by name. The first that holds a non-empty `GH_TOKEN=` line
/// gives its last such line's value when it is usable ([`is_usable`]). Every line of every key
/// file whose name and value pass [`gated`] gives one more value, an older `GH_TOKEN` included.
/// A file is read whole, at any size, and as lossy UTF-8 without a leading BOM: a byte that is
/// not UTF-8 costs only its own line (U+FFFD is not usable), never the file. Nothing is ever
/// printed.
fn key_file_values(project: &Path) -> (Option<SecretValue>, Vec<SecretValue>) {
    let Ok(entries) = std::fs::read_dir(project) else {
        return (None, Vec::new());
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_key_file(name))
        .filter(|name| std::fs::metadata(project.join(name)).is_ok_and(|m| m.is_file()))
        .collect();
    names.sort_by_key(|name| {
        let example = name.contains("example") || name.contains("sample");
        (name != TOKEN_FILE, example, name.clone())
    });
    let texts: Vec<String> = names
        .iter()
        .filter_map(|name| std::fs::read(project.join(name)).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect();
    let files: Vec<Vec<(&str, &str)>> = texts
        .iter()
        .map(|text| {
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            text.lines().filter_map(pair_in).collect()
        })
        .collect();
    let token = files
        .iter()
        .find_map(|pairs| {
            pairs
                .iter()
                .rev()
                .find(|(name, value)| *name == TOKEN_VARIABLE && !value.is_empty())
        })
        .filter(|(_, value)| is_usable(value.as_bytes()))
        .map(|(_, value)| SecretValue::new(ValueSource::Workspace, value.as_bytes().to_vec()));
    let others = files
        .iter()
        .flatten()
        .filter(|(name, value)| gated(name, value))
        .map(|(_, value)| SecretValue::new(ValueSource::Workspace, value.as_bytes().to_vec()))
        .collect();
    (token, others)
}

/// Whether `name` is one of the participant's key files: `.env`, or `.env.` and anything.
fn is_key_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// The name and value of one `NAME=value` line of a key file: `export ` before it, one pair of
/// quotes around the value and a `#` comment after any whitespace are allowed (a usable value
/// holds no whitespace, so the cut never shortens one). A `#` comment line has no pair.
fn pair_in(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let (name, value) = line.split_once('=')?;
    let name = name.trim_end();
    let is_name = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-');
    if name.is_empty() || !name.bytes().all(is_name) {
        return None;
    }
    let comment = value
        .as_bytes()
        .windows(2)
        .position(|w| w[0].is_ascii_whitespace() && w[1] == b'#');
    let value = comment.map_or(value, |at| &value[..at]).trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
        .unwrap_or(value);
    Some((name, unquoted))
}

/// The one name gate of section 9: `name` (a variable or dotenv name, the last segment of its
/// key) contains, ignoring case, a credential word, and `value` is usable ([`is_usable`]),
/// neither a URL (holds `://`) nor a path (starts with `/`, `~` or `.`), and not a name
/// ([`is_name_like`]: a model id or a host, which starring would break usage reading).
fn gated(name: &str, value: &str) -> bool {
    const WORDS: [&str; 8] = [
        "key",
        "token",
        "secret",
        "password",
        "pass",
        "pwd",
        "auth",
        "credential",
    ];
    let name = name.to_ascii_lowercase();
    WORDS.iter().any(|word| name.contains(word))
        && is_usable(value.as_bytes())
        && !value.contains("://")
        && !value.starts_with(['/', '~', '.'])
        && !is_name_like(value)
}

/// Whether `value` looks like a model id or a host rather than a secret: it is only lowercase
/// ASCII letters, digits and the separators `- . _ / : @`, holds at least one separator, every
/// separator-delimited segment is at most 15 bytes, and at least one segment holds a letter
/// from `g` to `z`. A UUID-style key (all hex segments), and a random token with an uppercase
/// letter or a segment of 16 bytes or more, are not name-like.
fn is_name_like(value: &str) -> bool {
    const SEPARATORS: [char; 6] = ['-', '.', '_', '/', ':', '@'];
    value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || SEPARATORS.contains(&c))
        && value.contains(SEPARATORS)
        && value.split(SEPARATORS).all(|segment| segment.len() <= 15)
        && value
            .split(SEPARATORS)
            .any(|segment| segment.bytes().any(|b| (b'g'..=b'z').contains(&b)))
}

#[cfg(test)]
mod tests {
    //! The single-pass matcher against the per-form oracle it replaced: each form's
    //! [`wrapped_regex`] for the whole-record matcher, and [`Progress`] for the stream's hold.

    use super::*;
    use regex::bytes::Regex;

    /// How far a text has matched one form, a line break included: the per-form recognizer the
    /// trie replaced, kept as the oracle of [`LiteralWalk`].
    #[derive(Debug, Clone, Copy, Default)]
    struct Progress {
        matched: usize,
        brk: Break,
    }

    impl Progress {
        fn step(&mut self, form: &[u8], b: u8) -> Advance {
            let at = self.matched;
            let Some(&want) = form.get(at) else {
                return Advance::Dead;
            };
            let next = match (self.brk, b) {
                (Break::None | Break::Done, b) if b == want => {
                    self.matched += 1;
                    self.brk = Break::None;
                    return if at + 1 == form.len() {
                        Advance::Complete
                    } else {
                        Advance::Live
                    };
                }
                (Break::None, _) if at == 0 => return Advance::Dead,
                (Break::None, b'\n') => Break::Done,
                (Break::None, b'\r') => Break::Cr,
                (Break::None, b'\\') if want != b'\\' => Break::Escape,
                (Break::Cr, b'\n') => Break::Done,
                (Break::Escape, b'\\') => Break::Escape2,
                (Break::Escape | Break::Escape2, b'n') => Break::Done,
                (Break::Escape | Break::Escape2, b'r') => Break::EscapeR,
                (Break::EscapeR, b'\\') => Break::EscapeRBackslash,
                (Break::EscapeRBackslash, b'\\') => Break::EscapeRBackslash2,
                (Break::EscapeRBackslash | Break::EscapeRBackslash2, b'n') => Break::Done,
                _ => return Advance::Dead,
            };
            self.brk = next;
            Advance::Live
        }
    }

    /// The regular expression of `form` with the breaks the grammar takes. Every character
    /// that is not a letter or a digit is a hex escape. The text spells the form: never print
    /// it.
    fn wrapped_regex(form: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut re = String::with_capacity(form.len() * 24);
        for (i, &b) in form.iter().enumerate() {
            if i > 0 {
                re.push_str(if b == b'\\' {
                    r"(?:\r?\n)?"
                } else {
                    r"(?:\r?\n|(?:\\{1,2}r)?\\{1,2}n)?"
                });
            }
            if b.is_ascii_alphanumeric() {
                re.push(char::from(b));
            } else {
                re.push_str(r"\x");
                re.push(char::from(HEX[usize::from(b >> 4)]));
                re.push(char::from(HEX[usize::from(b & 0x0f)]));
            }
        }
        re
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }

        fn pick(&mut self, set: &[u8]) -> u8 {
            set[self.below(set.len())]
        }
    }

    /// The breaks the grammar takes, and one it does not take (`\r` alone).
    const BREAKS: [&[u8]; 9] = [
        b"\n",
        b"\r\n",
        b"\\n",
        b"\\\\n",
        b"\\r\\n",
        b"\\\\r\\\\n",
        b"\\r\\\\n",
        b"\\\\r\\n",
        b"\r",
    ];

    /// A usable value of 20 to 31 bytes; some hold a `"`, `\`, `<` or `&`.
    fn value(rng: &mut Rng) -> Vec<u8> {
        const PLAIN: &[u8] = b"ABCDEFabcdef0123456789nru-_.";
        const SPECIAL: &[u8] = b"\"\\<&>";
        let len = 20 + rng.below(12);
        (0..len)
            .map(|_| {
                if rng.below(8) == 0 {
                    rng.pick(SPECIAL)
                } else {
                    rng.pick(PLAIN)
                }
            })
            .collect()
    }

    /// A text that plants forms of `forms`, whole, cut short, with breaks in random gaps (two
    /// in one gap, before the first byte, before a backslash of the form), between noise.
    fn text(rng: &mut Rng, forms: &[Vec<u8>]) -> Vec<u8> {
        const NOISE: &[u8] = b"Aa0%u\\nr\r\n\"<&x\xc3";
        let mut out = Vec::new();
        for _ in 0..1 + rng.below(6) {
            for _ in 0..rng.below(6) {
                out.push(rng.pick(NOISE));
            }
            let form = &forms[rng.below(forms.len())];
            let keep = if rng.below(4) == 0 {
                rng.below(form.len())
            } else {
                form.len()
            };
            if rng.below(10) == 0 {
                out.extend_from_slice(BREAKS[rng.below(BREAKS.len())]);
            }
            for (i, &b) in form[..keep].iter().enumerate() {
                let before_backslash = b == b'\\' && rng.below(4) == 0;
                if i > 0 && (before_backslash || rng.below(form.len()) == 0) {
                    for _ in 0..1 + usize::from(rng.below(8) == 0) {
                        out.extend_from_slice(BREAKS[rng.below(BREAKS.len())]);
                    }
                }
                out.push(b);
            }
        }
        out
    }

    /// The trie finds exactly what each form's regex finds, and its walk from every start
    /// agrees with the per-form recognizers on live, complete and dead at every byte.
    #[test]
    fn the_trie_finds_what_the_per_form_matchers_found() {
        let mut rng = Rng(0x7A1E_5EED);
        let mut planted = 0usize;
        for case in 0..80 {
            let values: Vec<SecretValue> = (0..1 + rng.below(8))
                .map(|_| SecretValue::new(ValueSource::Workspace, value(&mut rng)))
                .collect();
            let (secrets, _) = RunSecrets::new(&values);
            let forms: Vec<Vec<u8>> = (0..secrets.len())
                .map(|i| secrets.form(i).to_vec())
                .collect();
            assert!(!forms.is_empty(), "case {case}: no forms");
            let regexes: Vec<Regex> = forms
                .iter()
                .map(|f| Regex::new(&wrapped_regex(f)).expect("a form compiles"))
                .collect();
            for _ in 0..6 {
                let text = text(&mut rng, &forms);
                let mut expected: Vec<(usize, usize, usize)> = Vec::new();
                for (f, re) in regexes.iter().enumerate() {
                    expected.extend(re.find_iter(&text).map(|m| (f, m.start(), m.end())));
                }
                planted += expected.len();
                assert!(
                    secrets.find_all(&text) == expected,
                    "case {case}: the trie and the regexes differ"
                );
                walks_agree(&secrets, &forms, &text, case);
            }
        }
        assert!(planted > 300, "too few matches planted: {planted}");
    }

    #[test]
    fn find_all_alone_is_far_above_50_mb_s() {
        if cfg!(debug_assertions) {
            println!("find_all_alone_is_far_above_50_mb_s: skipped: debug build");
            return;
        }
        let mut rng = Rng(0xA11_0256);
        const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let values: Vec<SecretValue> = (0..32)
            .map(|_| {
                SecretValue::new(
                    ValueSource::Workspace,
                    (0..256).map(|_| rng.pick(SET)).collect(),
                )
            })
            .collect();
        let (secrets, counts) = RunSecrets::new(&values);
        assert_eq!(counts.workspace, 32);
        let forms: Vec<Vec<u8>> = (0..secrets.len())
            .map(|i| secrets.form(i).to_vec())
            .collect();
        let mut view = Vec::new();
        while view.len() < 4_000_000 {
            view.extend_from_slice(&text(&mut rng, &forms));
        }
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            let t = std::time::Instant::now();
            std::hint::black_box(secrets.find_all(&view));
            best = best.min(t.elapsed());
        }
        let rate = view.len() as f64 / 1e6 / best.as_secs_f64();
        println!("find_all_alone_is_far_above_50_mb_s: {rate:.1} MB/s");
        assert!(rate > 50.0, "find_all below 50 MB/s");
    }

    /// From every start of `text`, a [`LiteralWalk`] and the per-form [`Progress`] set (the
    /// old hold recognizer) agree byte by byte.
    fn walks_agree(secrets: &RunSecrets, forms: &[Vec<u8>], text: &[u8], case: usize) {
        for start in 0..text.len() {
            let mut old: Vec<(usize, Progress)> = (0..forms.len())
                .filter(|&f| forms[f].first() == text.get(start))
                .map(|f| (f, Progress::default()))
                .collect();
            let walk = secrets.spawn(text[start]);
            assert_eq!(
                walk.is_some(),
                !old.is_empty(),
                "case {case}: spawn at {start}"
            );
            let Some(mut walk) = walk else {
                continue;
            };
            for (at, &b) in text.iter().enumerate().skip(start) {
                let mut complete = false;
                old.retain_mut(|(f, p)| match p.step(&forms[*f], b) {
                    Advance::Live => true,
                    Advance::Complete => {
                        complete = true;
                        false
                    }
                    Advance::Dead => false,
                });
                let want = match (old.is_empty(), complete) {
                    (false, _) => Advance::Live,
                    (true, true) => Advance::Complete,
                    (true, false) => Advance::Dead,
                };
                let got = walk.step(secrets, b);
                assert_eq!(got, want, "case {case}: walk from {start} at {at}");
                if got != Advance::Live {
                    break;
                }
                assert_eq!(
                    walk.matched_any(),
                    old.iter().any(|(_, p)| p.matched >= 1),
                    "case {case}: matched from {start} at {at}"
                );
            }
        }
    }
}
