//! Compile-only guard for the daemon's immutable-submission writer surface.
//!
//! This consumer has no direct `goalcup-platform` dependency.  The daemon owns the
//! platform capability inputs exposed by `capture_submission`, so a development
//! consumer can construct and pass those inputs through the daemon crate alone.
#![cfg(feature = "v1-compat")]

use goalcup_daemon::{
    EntryArtifactRoot, NativeHandleBudgetV1, SourceDir, SourceRootJournalV1, SubmissionRequestV1,
    TraceDir, capture_submission, prepare_source_journal, readback_submission, recover_submission,
};

#[test]
fn writer_inputs_and_operations_are_public_from_daemon() {
    let _ = std::mem::size_of::<EntryArtifactRoot>();
    let _ = std::mem::size_of::<NativeHandleBudgetV1>();
    let _ = std::mem::size_of::<SourceDir>();
    let _ = std::mem::size_of::<SourceRootJournalV1>();
    let _ = std::mem::size_of::<SubmissionRequestV1>();
    let _ = std::mem::size_of::<TraceDir>();
    let _ = capture_submission;
    let _ = prepare_source_journal;
    let _ = readback_submission;
    let _ = recover_submission;
}
