//! The devserver's seed and the protocol's published placeholder are one key.
//!
//! The daemon must say, at startup, that its configured server key is the published
//! placeholder (every receipt and acknowledgement under it can be forged), and it cannot link
//! the devserver to know which key that is. So the public half lives in `goalcup-protocol`,
//! and this file is what keeps it from drifting from the seed that signs under it.

use goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX;

#[test]
fn the_devserver_seed_derives_the_protocol_placeholder() {
    assert_eq!(
        goalcup_devserver::dev_server_public_key_hex(),
        PLACEHOLDER_SERVER_PUBLIC_KEY_HEX,
        "DEV_SERVER_SEED and goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX drifted"
    );
}

/// The verifier keeps its own spelling of the same key (`PLACEHOLDER_SERVER_PUBLIC_KEY`) for
/// now: `S5-family` collapses it into the protocol constant. Until then this is the guard
/// that the two cannot differ.
#[test]
fn the_verifier_s_own_constant_is_still_the_protocol_one() {
    assert_eq!(
        goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY,
        PLACEHOLDER_SERVER_PUBLIC_KEY_HEX
    );
}
