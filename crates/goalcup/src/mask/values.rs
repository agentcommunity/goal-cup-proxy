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
//!   byte alignments: the characters that depend on no byte outside `v` (unpadded).
//!
//! A form is found with a line break between any two of its characters ([`Progress`],
//! [`wrapped_regex`]): LF or CR LF, raw, or escaped with one or two backslashes before each of
//! `r` and `n`, at most one break per gap. An escaped break is not taken right before a
//! backslash of the form itself, so every text matches a form in one way only.
//!
//! Values are read once, in memory only, and never written, hashed or logged. Every derived
//! buffer is overwritten when it drops; the compiled regexes keep copies of their own, which is
//! hygiene and not a boundary.

use super::{SecretValue, ValueCounts, ValueSource};
use regex::bytes::Regex;
use std::path::Path;

/// The shortest value used: a shorter one yields forms common in ordinary data.
pub const MIN_VALUE_BYTES: usize = 20;

/// The longest value used.
pub const MAX_VALUE_BYTES: usize = 256;

/// The longest form of a value: every byte `\u`-escaped, six bytes each.
pub const MAX_FORM_BYTES: usize = 6 * MAX_VALUE_BYTES;

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

/// Whether `value` is one the masker uses: 20 to 256 bytes of printable ASCII without a space.
pub fn is_usable(value: &[u8]) -> bool {
    (MIN_VALUE_BYTES..=MAX_VALUE_BYTES).contains(&value.len())
        && value.iter().all(|b| (0x21..=0x7e).contains(b))
}

/// Overwrite `bytes` with zeros so the stores are not removed as dead.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

/// Every form of `value`, distinct, each at least eight bytes. The caller wipes them.
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
    forms.sort();
    let mut unique: Vec<Vec<u8>> = Vec::with_capacity(forms.len());
    for mut form in forms {
        if form.len() < 8 || unique.last() == Some(&form) {
            wipe(&mut form);
        } else {
            unique.push(form);
        }
    }
    unique
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
    wipe(&mut padded);
    let lead = (alignment * 8).div_ceil(6).min(out.len());
    let kept = out[lead..].to_vec();
    wipe(&mut out);
    kept
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
/// not a letter or a digit is a hex escape. The text spells the form: wipe it once compiled, and
/// never print it.
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
/// (kind `run_secret`). `Debug` prints a count; the forms are wiped when it drops.
#[derive(Default)]
pub(crate) struct RunSecrets {
    forms: Vec<(Vec<u8>, Regex)>,
}

impl std::fmt::Debug for RunSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunSecrets({} forms)", self.forms.len())
    }
}

impl Drop for RunSecrets {
    fn drop(&mut self) {
        for (form, _) in &mut self.forms {
            wipe(form);
        }
    }
}

impl RunSecrets {
    /// The forms of `values`, compiled, and the counts for the start event. A value that is not
    /// usable, or one of whose forms does not compile, is left out and counted in `skipped`.
    pub(crate) fn new(values: &[SecretValue]) -> (RunSecrets, ValueCounts) {
        let mut counts = ValueCounts::default();
        let mut forms: Vec<(Vec<u8>, Regex)> = Vec::new();
        for value in values {
            let compiled = is_usable(value.bytes())
                .then(|| compile(value.bytes(), &forms))
                .flatten();
            let Some(mut new) = compiled else {
                counts.skipped = counts.skipped.saturating_add(1);
                continue;
            };
            forms.append(&mut new);
            let slot = match value.source() {
                ValueSource::Workspace => &mut counts.workspace,
                ValueSource::Env => &mut counts.env,
                ValueSource::Harness => &mut counts.harness,
            };
            *slot = slot.saturating_add(1);
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
    let mut out: Vec<(Vec<u8>, Regex)> = Vec::new();
    let mut failed = false;
    for mut form in encoded_forms(value) {
        let seen = known.iter().chain(out.iter()).any(|(k, _)| *k == form);
        if seen || failed {
            wipe(&mut form);
            continue;
        }
        let source = wrapped_regex(&form);
        let compiled = Regex::new(&source);
        wipe(&mut source.into_bytes());
        match compiled {
            Ok(re) => out.push((form, re)),
            Err(_) => {
                wipe(&mut form);
                failed = true;
            }
        }
    }
    if failed {
        for (form, _) in &mut out {
            wipe(form);
        }
        return None;
    }
    Some(out)
}

/// The variable the participant keeps their GitHub token in.
const TOKEN_VARIABLE: &str = "GH_TOKEN";

/// The key file the guide tells the participant to keep it in.
const TOKEN_FILE: &str = ".env.local";

/// The exact values of the project at `project` (the default build: the `GH_TOKEN` value
/// only). The key files (`.env`, and `.env.` and anything) are read by name, `.env.local`
/// first; the first that holds a `GH_TOKEN=` line gives its last such line's value. `export `
/// before it and one pair of quotes around the value are allowed. The value is returned only
/// when it is usable ([`is_usable`]); nothing is ever printed.
pub fn workspace_values(project: &Path) -> Vec<SecretValue> {
    let Ok(entries) = std::fs::read_dir(project) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_key_file(name))
        .collect();
    names.sort_by_key(|name| (name != TOKEN_FILE, name.clone()));
    let found = names.into_iter().find_map(|name| {
        let mut text = std::fs::read_to_string(project.join(&name)).ok()?;
        let value = token_in(&text);
        wipe_string(&mut text);
        value
    });
    match found {
        Some(mut value) if is_usable(value.as_bytes()) => {
            let bytes = value.as_bytes().to_vec();
            wipe_string(&mut value);
            vec![SecretValue::new(ValueSource::Workspace, bytes)]
        }
        Some(mut value) => {
            wipe_string(&mut value);
            Vec::new()
        }
        None => Vec::new(),
    }
}

/// Overwrite a string's bytes with zeros.
fn wipe_string(text: &mut String) {
    let mut bytes = std::mem::take(text).into_bytes();
    wipe(&mut bytes);
}

/// Whether `name` is one of the participant's key files: `.env`, or `.env.` and anything.
fn is_key_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// The value of the last `GH_TOKEN=` line of a key file.
fn token_in(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let value = line.strip_prefix(TOKEN_VARIABLE)?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
            .unwrap_or(value);
        (!unquoted.is_empty()).then(|| unquoted.to_string())
    })
}
