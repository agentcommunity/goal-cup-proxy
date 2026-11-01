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
//! A form is found with a line break between any two of its characters ([`Progress`],
//! [`wrapped_regex`]): LF or CR LF, raw, or escaped with one or two backslashes before each of
//! `r` and `n`, at most one break per gap. An escaped break is not taken right before a
//! backslash of the form itself, so every text matches a form in one way only.
//!
//! Values are read once, in memory only, and never written, hashed or logged.
//!
//! Sources (section 9, owner answer of 2026-11-01 to question 3, sources 1 to 3): the project's
//! `GH_TOKEN` (1), the credential-named variables of the harness environment goalcup builds
//! (2, [`env_values`]) and the other credential-named values of the project's key files (3).
//! Sources 2 and 3 pass the one name gate ([`gated`]). A harness key-file value (source 4) is
//! not built: it is left out and counted in `skipped`. The readers order, dedupe and cut the
//! values before the first control line ([`finish`]).

use super::{SecretValue, ValueCounts, ValueSource};
use regex::bytes::Regex;
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
/// cap of records. Each value adds up to fifteen compiled forms.
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

/// How far a text has matched one form, a line break included. It holds counts, never a byte
/// of the form, so it may be printed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Progress {
    matched: u16,
    brk: Break,
}

/// The line break in progress in the gap before the next character of a form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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

/// What one byte did to a [`Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    Live,
    /// This byte was the form's last character: the match is complete and cannot extend.
    Complete,
    Dead,
}

impl Progress {
    /// How many characters of the form have matched, breaks not counted.
    pub fn matched(&self) -> usize {
        usize::from(self.matched)
    }

    /// Feed the next byte of the text to a match of `form` that started at its first character.
    /// [`wrapped_regex`] is the same language.
    pub fn step(&mut self, form: &[u8], b: u8) -> Advance {
        let at = usize::from(self.matched);
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

/// The regular expression of `form` with the breaks [`Progress`] takes. Every character that is
/// not a letter or a digit is a hex escape. The text spells the form: never print it.
pub fn wrapped_regex(form: &[u8]) -> String {
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

/// The compiled forms of the exact values, shared by the whole-record matcher and the stream
/// (kind `run_secret`). `Debug` prints a count.
#[derive(Default)]
pub(crate) struct RunSecrets {
    forms: Vec<(Vec<u8>, Regex)>,
}

impl std::fmt::Debug for RunSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunSecrets({} forms)", self.forms.len())
    }
}

impl RunSecrets {
    /// The forms of `values`, compiled, and the counts for the start event. A count-only entry
    /// ([`SecretValue::left_out`], the readers' cut) adds its count to `skipped`. A value whose
    /// bytes equal one already compiled adds nothing and is counted nowhere. A harness value
    /// (source 4, not built), an unusable value, a value with a form that does not compile, and
    /// every value once [`MAX_VALUES`] are in use are counted in `skipped`. The readers'
    /// [`finish`] is the bound on the wire; this dedupe and cut is the backstop for callers that
    /// pass values directly.
    pub(crate) fn new(values: &[SecretValue]) -> (RunSecrets, ValueCounts) {
        let mut counts = ValueCounts::default();
        let mut forms: Vec<(Vec<u8>, Regex)> = Vec::new();
        let mut used: HashSet<&[u8]> = HashSet::new();
        for value in values {
            if value.left_out > 0 {
                counts.skipped = counts.skipped.saturating_add(value.left_out);
                continue;
            }
            if used.contains(value.bytes()) {
                continue;
            }
            let compiled = (value.source() != ValueSource::Harness
                && used.len() < MAX_VALUES
                && is_usable(value.bytes()))
            .then(|| compile(value.bytes(), &forms))
            .flatten();
            let Some(mut new) = compiled else {
                counts.skipped = counts.skipped.saturating_add(1);
                continue;
            };
            used.insert(value.bytes());
            forms.append(&mut new);
            match value.source() {
                ValueSource::Env => counts.env += 1,
                _ => counts.workspace += 1,
            }
        }
        (RunSecrets { forms }, counts)
    }

    /// The number of forms.
    pub(crate) fn len(&self) -> usize {
        self.forms.len()
    }

    /// Form `i`'s bytes.
    pub(crate) fn form(&self, i: usize) -> &[u8] {
        self.forms.get(i).map_or(&[], |(form, _)| form.as_slice())
    }

    /// Form `i`'s compiled rule.
    pub(crate) fn regex(&self, i: usize) -> Option<&Regex> {
        self.forms.get(i).map(|(_, re)| re)
    }
}

/// The new forms of one value (those not already in `known`), compiled, or `None` when one
/// does not compile.
fn compile(value: &[u8], known: &[(Vec<u8>, Regex)]) -> Option<Vec<(Vec<u8>, Regex)>> {
    encoded_forms(value)
        .into_iter()
        .filter(|form| !known.iter().any(|(k, _)| k == form))
        .map(|form| Regex::new(&wrapped_regex(&form)).ok().map(|re| (form, re)))
        .collect()
}

/// The variable the participant keeps their GitHub token in.
const TOKEN_VARIABLE: &str = "GH_TOKEN";

/// The key file the guide tells the participant to keep it in.
const TOKEN_FILE: &str = ".env.local";

/// The exact values of the project at `project`: the `GH_TOKEN` value (source 1), then every
/// other credential-named value of its key files (source 3), all `workspace`, through [`finish`].
pub fn workspace_values(project: &Path) -> Vec<SecretValue> {
    launch_values(project, &BTreeMap::new())
}

/// The credential-named variables of `env`, the environment goalcup builds for the harness
/// (source 2, `connector::LaunchPlan.env`), behind the name gate.
pub fn env_values(env: &BTreeMap<String, String>) -> Vec<SecretValue> {
    env.iter()
        .filter(|(name, value)| gated(name, value))
        .map(|(_, value)| SecretValue::new(ValueSource::Env, value.clone().into_bytes()))
        .collect()
}

/// Every exact value of a launch: [`workspace_values`] then the environment values (source 2),
/// through [`finish`].
pub fn launch_values(project: &Path, env: &BTreeMap<String, String>) -> Vec<SecretValue> {
    let (token, others) = key_file_values(project);
    finish(token.into_iter().chain(others).chain(env_values(env)))
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
