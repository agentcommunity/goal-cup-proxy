//! An entry that bound no session is a Fail with a document, not an unreadable trace with
//! none.
//!
//! The `goalcup-verify` line for an entry whose harness never started must not exit **3** with
//! `tracked_sessions must not be empty` and no verdict at all: an abandoned run has to yield a
//! Fail somebody can file and a trace somebody can read.
//!
//! "The trace could not be read" and "the trace says nothing happened" are different
//! answers, and only the first one is exit 3. This is the second: the descriptor parses, its
//! paths resolve, and what it says is that no harness ever started.
//!
//! The trace is a current compliant one, generated into a temporary directory and mutated
//! there — nothing under `tests/fixtures/` or `tests/traces/` is written. The committed
//! fixture deliberately retains historical connector pins, so it is not the pass control for
//! a current verifier build.

use crate::common as support;

use goalcup_verifier::checks::Severity;

/// A current compliant trace in a fresh temporary directory.
fn current_compliant_trace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    support::compliant::generate(dir.path());
    dir
}

/// Take the sessions out of a descriptor, leaving every other byte of it alone.
///
/// A line rewrite rather than a parse-and-re-emit: `trace.toml` is what the daemon wrote,
/// and round-tripping it through this crate's TOML writer would test the writer as much as
/// the loader.
fn empty_the_tracked_sessions(dir: &std::path::Path) {
    let path = dir.join("trace.toml");
    let text = std::fs::read_to_string(&path).expect("the descriptor reads");
    let rewritten: String = text
        .lines()
        .map(|line| match line.starts_with("tracked_sessions") {
            true => "tracked_sessions = []".to_string(),
            false => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rewritten.contains("tracked_sessions = []"),
        "the fixture still declares tracked_sessions somewhere this can find"
    );
    std::fs::write(&path, format!("{rewritten}\n")).expect("the descriptor writes");
}

#[test]
fn a_trace_that_bound_no_session_is_a_written_fail_and_not_a_refusal_to_read() {
    let dir = current_compliant_trace();
    empty_the_tracked_sessions(dir.path());

    let verdict = goalcup_verifier::verify_trace(dir.path())
        .expect("the descriptor parses, so a verdict is produced rather than an error");

    assert_eq!(
        verdict.overall,
        Severity::Fail,
        "nothing was captured, and that is a Fail rather than an unreadable trace"
    );
    assert_eq!(verdict.exit_code(), 2, "2 is the Fail rung, and 3 is not");

    let found = verdict
        .review_queue
        .iter()
        .find(|f| f.code == "no_session")
        .expect("the queue names the reason");
    assert_eq!(found.severity, Severity::Fail);
    assert!(
        found.message.contains("compliant-0001"),
        "the finding names the entry: {}",
        found.message
    );
    // A verdict document, on disk, that a reviewer can file — which is the whole of the
    // difference from exit 3.
    assert!(
        verdict
            .to_json()
            .expect("it serialises")
            .contains("no_session"),
        "the document carries it"
    );
}
