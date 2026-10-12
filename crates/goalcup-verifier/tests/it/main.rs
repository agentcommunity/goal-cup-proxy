//! The fast integration tests of `goalcup-verifier`, folded into one binary so the crate
//! links once. Each module was a `tests/<name>.rs` binary of its own; the shared helpers
//! are `tests/common/`. `m2_decoder_v1_quiet_panic` stays a binary of its own because it
//! replaces the process-wide panic hook.

#[path = "../common/mod.rs"]
mod common;

mod adversarial;
mod chain_load;
mod checks_v12;
mod checks_v13;
mod checks_v1_v5;
mod checks_v6_v11;
mod compliant;
mod conformance;
mod end_to_end;
mod gaps_attribution;
mod human_rule;
mod legacy_manifest_binding;
mod legs_load;
mod m2_anthropic_decoder;
#[cfg(unix)]
mod m2_capture_integrity;
mod m2_chat_decoder;
mod m2_decoder_v1;
#[cfg(unix)]
mod m2_hermes_capture_only;
#[cfg(unix)]
mod m2_legacy_baseline;
mod m2_native_observations;
mod m2_openai_decoder;
#[cfg(unix)]
mod m2_research_report;
mod no_session;
mod parser_claude_code;
mod parser_codex;
#[cfg(unix)]
mod submission_integrity;
mod trace_containment;
mod trace_containment_baseline;
mod trace_export_roundtrip;
#[cfg(unix)]
mod v5_corpus;
