//! The rules both binaries share: the masker's left guard and the host pattern match.

use goalcup_format::shared::{GUARD_BYTES, glued, host_matches};

#[test]
fn glued_is_a_word_byte_that_ends_no_escape() {
    assert!(glued(b"abc"));
    assert!(!glued(b""));
    assert!(!glued(b"x\\u0041"));
    assert!(!glued(b"%41"));
    assert!(!glued(b"\x1b[31m"));
}

/// Each escape the guard knows, and its limits: the verifier no longer has its own copy to
/// disagree with, so these are the only check on the rule both binaries use.
#[test]
fn glued_knows_every_escape_and_its_limits() {
    for b in [
        &b"x\\n"[..],    // backslash then a letter
        b"x\\\\n",       // several backslashes
        b"x\\x41",       // \x and 2 hex
        b"x\\e[31m",     // \e[ ANSI intro
        b"x\\033[1;31m", // \033[ ANSI intro
        b"x\\u001b[0m",  // JSON-escaped ESC
        b"x\\u001B[0m",
        b"x\\x1b[0m",
    ] {
        assert!(!glued(b), "{:?} ends an escape", String::from_utf8_lossy(b));
    }
    // Not escapes: a letter after `[` with no ESC, `%` with one hex, `\x` with one hex.
    for b in [&b"x[31m"[..], b"x%4", b"x_", b"x\\x4"] {
        assert!(glued(b), "{:?} is glued", String::from_utf8_lossy(b));
    }
    // ANSI parameters: up to 16 of `[0-9;]`; 17 is no escape.
    assert!(!glued(format!("\x1b[{}m", "1".repeat(16)).as_bytes()));
    assert!(glued(format!("\x1b[{}m", "1".repeat(17)).as_bytes()));
    // Only the last GUARD_BYTES are read: the longest escape, `\u001b[` and 16 parameters,
    // fills them exactly and is still seen after any prefix.
    let longest = format!("\\u001b[{}m", ";".repeat(16));
    assert_eq!(longest.len(), GUARD_BYTES);
    assert!(!glued(format!("{}{longest}", "a".repeat(100)).as_bytes()));
}

#[test]
fn host_matches_a_star_label_or_a_star_prefix() {
    assert!(host_matches("*.openai.azure.com", "a.openai.azure.com"));
    assert!(!host_matches("*.openai.azure.com", "openai.azure.com"));
    assert!(host_matches(
        "bedrock-runtime.*.amazonaws.com",
        "bedrock-runtime.us-east-1.amazonaws.com"
    ));
}
