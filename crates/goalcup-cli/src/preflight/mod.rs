//! D12 and D23, in code, and nothing else.
//!
//! Preflight is the set of questions this CLI asks **before** a run starts, where the
//! participant can still do something about the answers. Every one of them is a rule that
//! already exists somewhere else — D23's measured tier table, D12's tested version range,
//! the manifest's own `[preflight]` block, the release-hash allowlist — and nothing
//! here invents a rule of its own.
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
//! path, and **every place it matters says so on stdout**: [`manifest_pin_gate`] keeps the
//! check and re-words it rather than dropping it, so a report from a run that came through
//! the door can never be read as a report from a run that did not. [`DEV_DOOR`] is the
//! flag's one spelling.
//!
//! The fourth of those, the grant, was the V1 `goalcup start`'s rather than this module's:
//! it was not a preflight check, and a capture start asks for no grant. It is named here
//! because this is where the door is described, and a list of what one flag admits that is
//! missing one of them is the kind of list a reader trusts and should not.
//!
//! # The environment is handed in
//!
//! [`connector_checks`] and [`sample_harness_version`] take a `BTreeMap` rather than reading
//! `std::env`. A check that read the participant's environment could not be tested without
//! mutating this process's — which is `unsafe` in edition 2024, and for good reason beside a
//! `Command::output()` on another thread — and a probe that inherited it would be asking a
//! different server which version the harness is on. The probe's own environment is narrower
//! still; see [`probe_env`].

mod gates;
mod probe;

pub use gates::*;
pub use probe::*;
