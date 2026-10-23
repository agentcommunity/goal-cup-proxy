//! The fast integration tests of `goalcup-verifier`, folded into one binary so the crate
//! links once. Each module was a `tests/<name>.rs` binary of its own; the shared helpers
//! are `tests/common/`. `m2_decoder_v1_quiet_panic` stays a binary of its own because it
//! replaces the process-wide panic hook.

#[path = "../common/mod.rs"]
mod common;

mod connector_samples;
#[cfg(unix)]
mod harness_contract_golden;
mod m2_anthropic_decoder;
#[cfg(unix)]
mod m2_capture_integrity;
mod m2_chat_decoder;
mod m2_decoder_v1;
#[cfg(unix)]
mod m2_hermes_capture_only;
mod m2_native_observations;
mod m2_openai_decoder;
#[cfg(unix)]
mod m2_research_report;
#[cfg(unix)]
mod m2_run_stats;
mod m2_thread_decoder;
#[cfg(unix)]
mod v5_corpus;
