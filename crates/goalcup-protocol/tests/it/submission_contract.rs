//! The workspace-submission vocabulary the daemon configuration still names.

#[test]
fn unknown_submission_profile_is_not_legacy() {
    use goalcup_protocol::submission::SubmissionProfileV1;
    assert!(serde_json::from_str::<SubmissionProfileV1>("\"future_profile\"").is_err());
}
