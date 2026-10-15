//! Protocol hints: the open vocabulary a capture source uses to name its retained-byte format
//! (harness contract v1 §2.6).
//!
//! A hint is data on a manifest: `[[capture.hosts]].protocol_hint` for a wire host and
//! `[[capture.native]].protocol_hint` for a native store. One grammar covers both. This module
//! is the grammar for hints. The registry supplies candidates; each decoder receives the full
//! `StreamOpen` in `accepts` and decides which fields matter. The return values, not the hint
//! grammar, are authoritative for selection.
//!
//! **A syntactically valid hint does not imply a decoder exists. A stream is capture-only when no
//! registered decoder accepts its full StreamOpen.** D43, ruling 8: the bytes are recorded and no
//! decoder reads them. The decode outcome names the hint, and the research report lists it under
//! `analysis.undecoded`, so a misspelling is inspectable. An unsupported source or content type may instead be accepted and emitted as
//! `unknown`; the session records capture-only when it finds no accepting decoder. A capture-only
//! format needs no protocol edit.

/// The longest hint, in bytes.
pub(crate) const MAX_HINT_BYTES: usize = 64;

/// The one hint grammar: `^[a-z][a-z0-9.-]{0,63}$`.
pub fn valid_hint(hint: &str) -> bool {
    let mut bytes = hint.bytes();
    hint.len() <= MAX_HINT_BYTES
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grammar_is_one_lowercase_word_with_dots_and_hyphens() {
        for ok in ["a", "openai-responses", "x.y-z9", &"a".repeat(64)] {
            assert!(valid_hint(ok), "{ok}");
        }
        for bad in ["", "A", "1a", "-a", "a_b", "a b", "a/b", &"a".repeat(65)] {
            assert!(!valid_hint(bad), "{bad}");
        }
    }

    #[test]
    fn the_grammar_does_not_require_a_central_hint_registry() {
        assert!(valid_hint("some-new-protocol"));
        assert!(valid_hint("openai-responses"));
        assert!(valid_hint("openai-chat-completions"));
    }
}
