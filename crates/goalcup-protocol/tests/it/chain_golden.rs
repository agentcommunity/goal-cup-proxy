use goalcup_protocol::chain::{Chain, GENESIS, link_hash};
use std::path::PathBuf;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The first two links of the M0 leg-1 capture
/// (spikes/claude-code-cli/out/run-20260903T073054Z/leg1/capture.jsonl, git-ignored).
/// If these two values change, the daemon's chain no longer verifies the M0 captures.
#[test]
fn reproduces_the_m0_leg1_capture_links() {
    let line1 =
        br#"{"type":"mode","mode":"normal","sessionId":"95addb64-9dac-4046-8ef6-8aa53fa0c7f1"}"#;
    let line2 = br#"{"type":"permission-mode","permissionMode":"auto","sessionId":"95addb64-9dac-4046-8ef6-8aa53fa0c7f1"}"#;

    let mut chain = Chain::new();
    let l1 = chain.try_append(line1).unwrap();
    assert_eq!(l1.seq, 1);
    assert_eq!(l1.prev, GENESIS);
    assert_eq!(
        l1.hash,
        "4e8c478cc81cc43c4fcd273ffd454b5383ed3963c2a07bdf238a33d2afd6d6ca"
    );

    let l2 = chain.try_append(line2).unwrap();
    assert_eq!(l2.seq, 2);
    assert_eq!(l2.prev, l1.hash);
    assert_eq!(
        l2.hash,
        "eb231e6e0a99e829966ba74b109c8f582e0659e687631d9fed4df14912638018"
    );
    assert_eq!(chain.head(), l2.hash);
    assert_eq!(chain.seq(), 2);
}

fn chain_over_file(rel: &str) -> (u64, String) {
    let text = std::fs::read_to_string(fixture(rel)).expect("fixture readable");
    let mut chain = Chain::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        chain.try_append(line.as_bytes()).unwrap();
    }
    (chain.seq(), chain.head().to_string())
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

#[test]
fn resume_continues_an_existing_chain() {
    let mut a = Chain::new();
    a.try_append(b"one").unwrap();
    let l2 = a.try_append(b"two").unwrap();

    let mut b = Chain::resume(l2.seq, l2.hash.clone());
    let l3 = b.try_append(b"three").unwrap();
    assert_eq!(l3.seq, 3);
    assert_eq!(l3.prev, l2.hash);
    assert_eq!(l3.hash, link_hash(&l2.hash, b"three"));
}

#[test]
fn a_resumed_maximum_sequence_refuses_to_wrap() {
    let mut chain = Chain::resume(u64::MAX, "f".repeat(64));
    let error = chain
        .try_append(b"one more record")
        .expect_err("a writer cannot represent a successor after u64::MAX");
    assert_eq!(error.to_string(), "chain sequence exhausted");
    assert_eq!(
        chain.seq(),
        u64::MAX,
        "refusal leaves the existing head intact"
    );
}
