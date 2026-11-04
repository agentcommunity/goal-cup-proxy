//! The rules both binaries share: the masker's left guard and the host pattern match.

use goalcup_format::shared::{glued, host_matches};

#[test]
fn glued_is_a_word_byte_that_ends_no_escape() {
    assert!(glued(b"abc"));
    assert!(!glued(b""));
    assert!(!glued(b"x\\u0041"));
    assert!(!glued(b"%41"));
    assert!(!glued(b"\x1b[31m"));
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
