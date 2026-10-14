//! The link formula, pinned against hashes recorded before any of this code existed.
//!
//! `link_hash(prev, bytes)` is `hex(sha256(prev_hex_ascii || bytes))` from the 64-zero
//! `GENESIS`. Every v5 chain links its envelopes through it, so a change to it is a change to
//! every chain ever written, and these goldens are what would catch one.

use goalcup_protocol::chain::{GENESIS, link_hash};
use std::path::PathBuf;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Fold `link_hash` over `lines` from `head`, returning how many links were made and the
/// last hash.
fn fold<'a>(head: &str, lines: impl IntoIterator<Item = &'a [u8]>) -> (u64, String) {
    let mut count = 0u64;
    let mut head = head.to_string();
    for line in lines {
        head = link_hash(&head, line);
        count += 1;
    }
    (count, head)
}

/// The first two links of the M0 leg-1 capture
/// (spikes/claude-code-cli/out/run-20260903T073054Z/leg1/capture.jsonl, git-ignored).
/// If these two values change, the formula no longer reproduces the M0 captures.
#[test]
fn reproduces_the_m0_leg1_capture_links() {
    let line1 =
        br#"{"type":"mode","mode":"normal","sessionId":"95addb64-9dac-4046-8ef6-8aa53fa0c7f1"}"#;
    let line2 = br#"{"type":"permission-mode","permissionMode":"auto","sessionId":"95addb64-9dac-4046-8ef6-8aa53fa0c7f1"}"#;

    let l1 = link_hash(GENESIS, line1);
    assert_eq!(
        l1,
        "4e8c478cc81cc43c4fcd273ffd454b5383ed3963c2a07bdf238a33d2afd6d6ca"
    );
    let l2 = link_hash(&l1, line2);
    assert_eq!(
        l2,
        "eb231e6e0a99e829966ba74b109c8f582e0659e687631d9fed4df14912638018"
    );
}

fn chain_over_file(rel: &str) -> (u64, String) {
    let text = std::fs::read_to_string(fixture(rel)).expect("fixture readable");
    fold(
        GENESIS,
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::as_bytes),
    )
}

#[test]
fn chains_the_committed_claude_code_fixtures() {
    assert_eq!(
        chain_over_file("spikes/claude-code-cli/fixtures/main-session.jsonl"),
        (
            148,
            "c2161937817723a5c4f33bb5b04eff93d45afc8a91a09cdf3d08154a275da5c5".to_string()
        )
    );
    assert_eq!(
        chain_over_file("spikes/claude-code-cli/fixtures/hooks.jsonl"),
        (
            50,
            "5f62bc8e67dc71d8bec57ddb8362eb5c28bc8e0b95e70f0d70a6862c5ec606c1".to_string()
        )
    );
    assert_eq!(
        chain_over_file("spikes/claude-code-cli/fixtures/subagent-a8ca2ea8d4a8a1673.jsonl"),
        (
            17,
            "cd69fd57bba70605571379c4bfb0f48b085373f0878cef8e44204e727ea0dabc".to_string()
        )
    );
}

/// A chain continued from a recorded head is the chain that would have been written in one
/// go: the head is the whole state a continuation needs.
#[test]
fn continuing_from_a_head_equals_one_pass() {
    let records: [&[u8]; 3] = [b"one", b"two", b"three"];
    let (_, whole) = fold(GENESIS, records);
    let (_, head) = fold(GENESIS, records[..2].iter().copied());
    let (_, continued) = fold(&head, records[2..].iter().copied());
    assert_eq!(continued, whole);
    assert_eq!(continued, link_hash(&head, b"three"));
}

/// A stray edit to any earlier record moves every later hash: the property P2 rests on.
#[test]
fn an_edit_to_an_earlier_record_moves_the_head() {
    let honest: [&[u8]; 3] = [b"one", b"two", b"three"];
    let edited: [&[u8]; 3] = [b"one", b"tw0", b"three"];
    assert_ne!(fold(GENESIS, honest).1, fold(GENESIS, edited).1);
}
