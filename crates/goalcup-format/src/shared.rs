//! The rules the recorder and the verifier share, so the two cannot drift: the masker's left
//! guard (PRD-05 section 4) and the host pattern match of the host list.

/// How many bytes before a match the left guard reads.
pub const GUARD_BYTES: usize = 24;

/// Whether a guarded match whose raw bytes before it are `before` is glued to a word: the byte
/// just before it is one of `[A-Za-z0-9_]` and ends no escape (PRD-05 section 4). Reads at most
/// the last [`GUARD_BYTES`]. Nothing before the match (start of input, after a gap, after a
/// message end) is never glued. Never `\b`: that misses keys after `\n` in JSON text.
pub fn glued(before: &[u8]) -> bool {
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

/// Whether `name` matches `pattern`: exact, `*.a.b` (labels before `a.b`), or one `*` in a label.
pub fn host_matches(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.").filter(|s| !s.contains('*')) {
        let labels_before = |b: &str| b.len() > 1 && b.ends_with('.');
        return name.strip_suffix(suffix).is_some_and(labels_before);
    }
    let (p, n): (Vec<&str>, Vec<&str>) = (pattern.split('.').collect(), name.split('.').collect());
    p.len() == n.len()
        && p.iter().zip(&n).all(|(p, n)| match p.split_once('*') {
            Some((head, tail)) => {
                n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail)
            }
            None => p == n,
        })
}
