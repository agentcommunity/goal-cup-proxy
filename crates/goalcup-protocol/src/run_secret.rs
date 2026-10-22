//! A run's own exact secret values and the byte forms they take (D60).
//!
//! Goal Cup Proxy hands a harness one secret of its own, which it knows exactly: the per-run proxy
//! credential in `HTTPS_PROXY` (H8). It never handles a participant's API key: the harness holds
//! that in its own config. The launcher passes the proxy credential to the daemon on the capture
//! create ([`crate::control::CaptureWorkspaceRequest::run_secrets`]), in memory only, and the
//! daemon redacts every form below from every recorded body before the chain hashes it. This
//! module defines the forms and how they are found. It performs no redaction.
//!
//! The forms of a value `v`:
//!
//! * `v` itself;
//! * `v` with every byte percent-encoded (`%73%6B…`), in upper and in lower case hex. A value of
//!   unreserved characters is unchanged by ordinary percent-encoding, which `v` itself covers;
//! * `v` with every byte JSON `\u`-escaped (`sk…`), in lower and in upper case hex;
//! * `v` inside a longer base64 text, standard and URL-safe alphabets, at each of the three
//!   byte alignments: the characters that depend on no byte outside `v` (unpadded). At most the
//!   first and the last character of an encoded `v` are not covered, and each holds less than
//!   one byte of it.
//!
//! A form is found with a line break between any two of its characters ([`Progress`],
//! [`wrapped_regex`], [`WrappedForms`]): `base64` wraps its output at 76 columns (`env | base64`
//! on Linux), `fold` wraps text, a tool's output inside a JSON string carries each break as the
//! two bytes `\n`, and JSON nested in a JSON string (Hermes's tool results) as the three bytes
//! `\\n`. A break is LF or CR LF, raw, or escaped with one or two backslashes before each of
//! `r` and `n` (`\n`, `\r\n`, `\\n`, `\\r\\n`): at most one break per gap, at most six bytes. An
//! escaped break is not taken right before a backslash of the form itself, so every text
//! matches a form in one way only. Three levels of escaping are not caught.
//!
//! Every derived buffer is overwritten when it is dropped. That is hygiene, not a boundary: the
//! value stays in the launching process's environment for its life, which D37 accepts. The
//! serialized capture create (the CLI's request body, the bytes the daemon received) and each
//! compiled regex keep copies that are not wiped, for the same reason.

use serde::{Deserialize, Serialize};

/// The shortest value a run may name: a shorter one yields forms common in ordinary data.
pub const MIN_RUN_SECRET_BYTES: usize = 20;

/// The longest value a run may name.
pub const MAX_RUN_SECRET_BYTES: usize = 256;

/// The most values one capture create may carry: the proxy credential.
pub const MAX_RUN_SECRETS: usize = 1;

/// The longest form of a value: every byte `\u`-escaped, six bytes each.
pub const MAX_FORM_BYTES: usize = 6 * MAX_RUN_SECRET_BYTES;

/// The longest break: CR LF with two backslashes before each letter (six bytes).
pub const MAX_BREAK_BYTES: usize = 6;

/// The longest text a form of `form_len` bytes can match: the longest break in every gap
/// between two of its characters.
pub const fn wrapped_len(form_len: usize) -> usize {
    form_len + MAX_BREAK_BYTES * form_len.saturating_sub(1)
}

/// The longest text any form of any run secret can match. A stream that holds a form from its
/// first byte must be able to hold this many.
pub const MAX_WRAPPED_FORM_BYTES: usize = wrapped_len(MAX_FORM_BYTES);

/// One exact secret value of a run. `Debug` never shows it, and its bytes are overwritten when
/// it drops. It serializes as a plain string because it crosses the loopback control API, the
/// one place it is meant to go.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunSecret(String);

impl RunSecret {
    /// `value` as a run secret, or `None` when it is not [`MIN_RUN_SECRET_BYTES`] to
    /// [`MAX_RUN_SECRET_BYTES`] bytes of printable ASCII without a space. Never echoes it.
    pub fn new(value: &[u8]) -> Option<RunSecret> {
        is_usable(value).then(|| RunSecret(String::from_utf8_lossy(value).into_owned()))
    }

    /// Whether this value (perhaps deserialized from anywhere) has the shape [`Self::new`]
    /// admits.
    pub fn is_usable(&self) -> bool {
        is_usable(self.0.as_bytes())
    }

    /// The value. Every caller is a redactor or a matcher.
    pub fn expose(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl std::fmt::Debug for RunSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunSecret(<redacted>)")
    }
}

impl Drop for RunSecret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        wipe(&mut bytes);
    }
}

fn is_usable(value: &[u8]) -> bool {
    (MIN_RUN_SECRET_BYTES..=MAX_RUN_SECRET_BYTES).contains(&value.len())
        && value.iter().all(|b| (0x21..=0x7e).contains(b))
}

/// Overwrite `bytes` with zeros so the stores are not removed as dead.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

/// Every form of `value` (see the module documentation), distinct, each at least eight bytes.
/// The caller owns the buffers and should [`wipe`] them when done.
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

/// The base64 text that `form` always produces when it sits inside a longer encoded stream at
/// byte offset `alignment` modulo three: the characters that depend on no byte outside the
/// form. Unpadded, since the end of the form is rarely the end of the stream.
///
/// With `alignment` bytes of unknown data before it, the first `ceil(alignment * 8 / 6)`
/// characters mix those bytes in, and the characters after the last complete group are
/// determined only as far as whole six-bit values of the form's own bits go.
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
    // Drop the characters that mix in the unknown bytes before the form, and wipe what is
    // dropped: `split_off` would leave the lead in the buffer it keeps.
    let lead = (alignment * 8).div_ceil(6).min(out.len());
    let kept = out[lead..].to_vec();
    wipe(&mut out);
    kept
}

/// How far a text has matched one form, a line break included ([`Self::step`]). It holds counts,
/// never a byte of the form, so it may be printed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Progress {
    matched: u16,
    brk: Break,
}

/// The line break in progress in the gap before the next character of a form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
enum Break {
    /// None yet: the next byte is the form's next character, or starts a break.
    #[default]
    None,
    /// A raw CR: an LF must follow.
    Cr,
    /// A backslash: `n` ends an escaped LF, `r` starts an escaped CR LF, and a second
    /// backslash makes it doubly escaped.
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
    /// Still a match in progress.
    Live,
    /// This byte was the form's last character: the match is complete and cannot extend.
    Complete,
    /// No match of the form goes through this byte.
    Dead,
}

impl Progress {
    /// How many characters of the form have matched, breaks not counted.
    pub fn matched(&self) -> usize {
        usize::from(self.matched)
    }

    /// Feed the next byte of the text to a match of `form` (at most [`MAX_FORM_BYTES`] bytes of
    /// printable ASCII) that started at its first character. A break may come between any two
    /// characters, never before the first: LF, CR LF, or, when the form's next character is
    /// not a backslash, `\n` or `\r\n` as text with one or two backslashes before each letter.
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

/// The regular expression ([`regex` crate] syntax) of `form` with the breaks [`Progress`] takes:
/// the daemon's whole-record oracle of its streaming recognizer. Every character that is not a
/// letter or a digit is written as a hex escape. The text spells the form: wipe it once it is
/// compiled, and never print it.
///
/// [`regex` crate]: https://docs.rs/regex
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

/// Every form of one value ([`encoded_forms`]), found with the breaks [`Progress`] takes: an
/// independent search for tests and readers of a trace. `Debug` prints a count; the forms are
/// wiped when it drops.
pub struct WrappedForms {
    /// The forms, by their first byte.
    by_first: Vec<Vec<Vec<u8>>>,
    count: usize,
    span: usize,
}

impl std::fmt::Debug for WrappedForms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WrappedForms({} forms)", self.count)
    }
}

impl Drop for WrappedForms {
    fn drop(&mut self) {
        for group in &mut self.by_first {
            for form in group {
                wipe(form);
            }
        }
    }
}

impl WrappedForms {
    /// The forms of `value`, or `None` when it is not a usable run secret ([`RunSecret::new`]).
    pub fn of(value: &[u8]) -> Option<WrappedForms> {
        if !is_usable(value) {
            return None;
        }
        let mut by_first: Vec<Vec<Vec<u8>>> = vec![Vec::new(); 256];
        let (mut count, mut span) = (0, 0);
        for form in encoded_forms(value) {
            span = span.max(wrapped_len(form.len()));
            count += 1;
            by_first[usize::from(form[0])].push(form);
        }
        Some(WrappedForms {
            by_first,
            count,
            span,
        })
    }

    /// The longest text one match can cover, breaks included. A scan that carries the last
    /// `span() - 1` bytes of one buffer into the next misses no match across the two.
    pub fn span(&self) -> usize {
        self.span
    }

    /// Whether `text` holds any form, with or without breaks.
    pub fn contains(&self, text: &[u8]) -> bool {
        for (start, &first) in text.iter().enumerate() {
            for form in &self.by_first[usize::from(first)] {
                let mut progress = Progress::default();
                for &b in &text[start..] {
                    match progress.step(form, b) {
                        Advance::Live => {}
                        Advance::Complete => return true,
                        Advance::Dead => break,
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &[u8] = b"sk-0123456789abcdef0123456789abcdef";

    /// Each form is what the encoder it names writes, found inside a longer text of it.
    #[test]
    fn every_form_is_found_in_what_its_encoder_writes() {
        let forms = encoded_forms(VALUE);
        let holds = |text: &[u8]| forms.iter().any(|f| text.windows(f.len()).any(|w| w == f));
        assert!(holds(VALUE));
        let percent: String = VALUE.iter().map(|b| format!("%{b:02X}")).collect();
        assert!(holds(percent.as_bytes()));
        assert!(holds(percent.to_lowercase().as_bytes()));
        let escaped: String = VALUE.iter().map(|b| format!("\\u{:04x}", b)).collect();
        assert!(holds(escaped.as_bytes()));
        assert!(holds(
            escaped.to_uppercase().replace("\\U", "\\u").as_bytes()
        ));
        for lead in ["", "a", "ab", "abc", "HTTPS_PROXY=x"] {
            let mut text = lead.as_bytes().to_vec();
            text.extend_from_slice(VALUE);
            text.extend_from_slice(b" and more");
            assert!(holds(standard(&text, false).as_bytes()), "{lead:?}");
            assert!(holds(standard(&text, true).as_bytes()), "{lead:?} url-safe");
        }
        // And nothing of an unrelated text.
        assert!(!holds(b"sk-0123456789abcdef0123456789abcdeX"));
    }

    fn standard(bytes: &[u8], url_safe: bool) -> String {
        // An independent encoder: the plain RFC 4648 one, padded.
        let table: &[u8] = if url_safe {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
        } else {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        };
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = (u32::from(chunk[0]) << 16)
                | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                | u32::from(*chunk.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(table[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// `text` cut into lines of `width` characters, each ended by `end`: what `base64` (76),
    /// `openssl base64` (64) and `fold -w N` write.
    fn wrap(text: &[u8], width: usize, end: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for line in text.chunks(width) {
            out.extend_from_slice(line);
            out.extend_from_slice(end.as_bytes());
        }
        out
    }

    /// A form with a line break between two of its characters is still the form: base64 at
    /// every alignment and in both alphabets wrapped at 76 and at 64 columns, the raw value and
    /// its percent-encoding folded at 20, with LF, CR LF and either as JSON text.
    #[test]
    fn a_wrapped_form_is_still_found() {
        let forms = WrappedForms::of(VALUE).unwrap();
        assert!(!format!("{forms:?}").contains("0123"));
        let percent: String = VALUE.iter().map(|b| format!("%{b:02X}")).collect();
        // Raw, escaped once (a JSON string), and escaped twice (JSON in a JSON string, as Hermes
        // records a tool's output).
        for end in ["\n", "\r\n", "\\n", "\\r\\n", "\\\\n", "\\\\r\\\\n"] {
            for lead in 0..80 {
                let mut text = vec![b'x'; lead];
                text.extend_from_slice(b"HTTPS_PROXY=http://goalcup:");
                text.extend_from_slice(VALUE);
                text.extend_from_slice(b"\nPATH=/usr/bin\n");
                for url_safe in [false, true] {
                    for width in [76, 64] {
                        let wrapped = wrap(standard(&text, url_safe).as_bytes(), width, end);
                        assert!(forms.contains(&wrapped), "{end:?} lead {lead} at {width}");
                    }
                }
            }
            for width in [1, 7, 20] {
                assert!(
                    forms.contains(&wrap(VALUE, width, end)),
                    "{end:?} fold {width}"
                );
                assert!(forms.contains(&wrap(percent.as_bytes(), width, end)));
            }
        }
        // The longest wrapping of a form is its bound, and within the span.
        let longest = wrap(VALUE, 1, "\\\\r\\\\n");
        assert_eq!(longest.len() - MAX_BREAK_BYTES, wrapped_len(VALUE.len()));
        assert!(wrapped_len(VALUE.len()) <= forms.span());
        assert!(forms.span() <= MAX_WRAPPED_FORM_BYTES);
    }

    /// A break is raw or escaped once or twice, one per gap, and only between two characters; any other byte in
    /// a gap ends the match.
    #[test]
    fn only_a_line_break_may_come_between_two_characters() {
        let forms = WrappedForms::of(VALUE).unwrap();
        let (head, tail) = VALUE.split_at(10);
        for gap in [
            "\n\n",
            "\r",
            " ",
            "\t",
            "\\t",
            "\\",
            "\\r",
            "\\n\\n",
            "\n\\n",
            "\\\\\\n",
            "\\\\",
            "\\\\r\\\\",
        ] {
            let text = [head, gap.as_bytes(), tail].concat();
            assert!(!forms.contains(&text), "{gap:?}");
        }
        for gap in ["\n", "\r\n", "\\n", "\\r\\n", "\\\\n", "\\\\r\\\\n"] {
            let text = [head, gap.as_bytes(), tail].concat();
            assert!(forms.contains(&text), "{gap:?}");
        }
        // Three levels of escaping are not a break.
        assert!(!forms.contains(&[head, b"\\\\\\n".as_slice(), tail].concat()));
        // A value with a backslash in it: its own backslash wins over an escaped break.
        let odd = b"ab\\nopqrstuvwxyz0123456789";
        let forms = WrappedForms::of(odd).unwrap();
        assert!(forms.contains(odd));
        assert!(forms.contains(b"ab\n\\nopqrstuvwxyz0123456789"));
        // An escaped break is not taken right before the form's own backslash.
        assert!(!forms.contains(b"ab\\r\\n\\nopqrstuvwxyz0123456789"));
    }

    #[test]
    fn a_run_secret_has_one_shape_and_never_prints() {
        assert!(RunSecret::new(VALUE).is_some());
        assert!(RunSecret::new(b"short").is_none());
        assert!(RunSecret::new(b"has a space in it, twenty+").is_none());
        assert!(RunSecret::new(&[b'a'; MAX_RUN_SECRET_BYTES + 1]).is_none());
        let secret = RunSecret::new(VALUE).unwrap();
        assert!(!format!("{secret:?}").contains("0123"));
        let json = serde_json::to_string(&secret).unwrap();
        let back: RunSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.expose(), VALUE);
    }
}
