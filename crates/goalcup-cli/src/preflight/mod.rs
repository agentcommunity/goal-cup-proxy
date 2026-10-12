//! The questions this CLI asks **before** a run starts, where the participant can still do
//! something about the answers. Every one of them is a rule that already exists somewhere
//! else — the release-hash allowlist, the connector's own pinned `harness.toml` — and nothing
//! here invents a rule of its own. The V1 tier, version-range, banned-environment,
//! required-path and free-port checks read the V1 `manifest.toml` and went with it (OS-29);
//! the schema-1 profile pins the exact harness version and binary digest, which `goalcup
//! start` checks at launch.
//!
//! # Every check carries a fix
//!
//! [`Check::fix`] is not decoration. A refusal a participant cannot act on is a refusal
//! they will work around, and the way around a tracker is not a way anybody wants taken.
//! So a check that can say `Fail` says, in the same breath, the command that makes it stop
//! saying it.
//!
//! # `--dev` is the one door
//!
//! Exactly one flag admits a rulebook this build does not pin, a placeholder server key, a
//! loopback development server or a machine that has no stored grant, and it is `--dev`
//! (the V1 `goalcup auth` stored the grant and the V1 `--verify-with-dev` passed the flag
//! to the verifier; D48 removed both). It is typed by a human, never set by a shipped code
//! path, and **every place it matters says so on stdout**: [`harness_pin_gate`] keeps the
//! check and re-words it rather than dropping it, so a report from a run that came through
//! the door can never be read as a report from a run that did not. [`DEV_DOOR`] is the
//! flag's one spelling.
//!
//! The fourth of those, the grant, was the V1 `goalcup start`'s rather than this module's:
//! it was not a preflight check, and a capture start asks for no grant. It is named here
//! because this is where the door is described, and a list of what one flag admits that is
//! missing one of them is the kind of list a reader trusts and should not.

mod gates;
mod probe;

pub use gates::*;
pub use probe::*;
