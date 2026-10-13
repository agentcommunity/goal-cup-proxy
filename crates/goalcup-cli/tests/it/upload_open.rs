//! The checks `goalcup start` makes on what it sends to, and takes from, the upload server.

use goalcup_cli::upload::http::ServerError;
use goalcup_cli::upload::{checked_repo_url, is_run_id};

#[test]
fn a_repo_is_a_github_owner_and_name_and_nothing_else() {
    for (given, kept) in [
        (
            "https://github.com/goal-cup/demo",
            "https://github.com/goal-cup/demo",
        ),
        (
            "https://github.com/goal-cup/demo.git",
            "https://github.com/goal-cup/demo",
        ),
        ("https://github.com/a/b.c_d/", "https://github.com/a/b.c_d"),
    ] {
        assert_eq!(checked_repo_url(given).unwrap(), kept);
    }
    for bad in [
        "http://github.com/a/b",
        "https://gitlab.com/a/b",
        "https://github.com/a",
        "https://github.com/a/b/tree/main",
        "https://github.com/a b/c",
        "https://github.com//b",
        "git@github.com:a/b.git",
        "https://github.com/a/..",
        "https://github.com/a_b/c",
    ] {
        let refusal = checked_repo_url(bad).unwrap_err();
        assert_eq!(refusal.exit_code(), 1, "{bad}");
    }
}

#[test]
fn run_ids_have_one_shape() {
    assert!(is_run_id("r0123456789abcdef"));
    for bad in [
        "r0123",
        "R0123456789abcdef",
        "e0123456789abcdef",
        "r0123456789abcdeg",
    ] {
        assert!(!is_run_id(bad), "{bad}");
    }
}

/// What is asked again and what is not: no answer, a server error, a rate limit, the
/// maintenance switch and a damaged body are; a refusal of the request itself is not.
#[test]
fn only_a_transient_failure_is_retried() {
    let status = |status: u16| ServerError::Status {
        status,
        code: String::new(),
        message: String::new(),
        retry_after: Some(7),
    };
    assert!(ServerError::Transport("refused".into()).is_retryable());
    for retryable in [500, 502, 503, 429, 422] {
        assert!(status(retryable).is_retryable(), "{retryable}");
    }
    for permanent in [400, 401, 403, 404, 409, 410, 413] {
        assert!(!status(permanent).is_retryable(), "{permanent}");
    }
    assert_eq!(status(503).retry_after(), Some(7));
    assert!(status(409).is_conflict());
}
