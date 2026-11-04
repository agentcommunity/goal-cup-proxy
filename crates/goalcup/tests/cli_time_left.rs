//! `goalcup time-left` (PRD-10 section 10, D66): the server's last word counted down, "unknown"
//! when it is null or older than 11 minutes, and "time is up" on its `stop`.

mod cli_common;

use cli_common::{T0, upload};
use goalcup::cli::text::left_words;
use goalcup::cli::time_left::line;
use serde_json::json;

/// `server_word` asked `ago` seconds before [`T0`] with two hours left, read at [`T0`].
fn asked(ago: i64, stop: bool) -> Result<String, String> {
    let at = goalcup_format::dump::wall(T0.saturating_add_signed(-ago) * 1_000_000_000);
    let word = json!({"stop": stop, "reason": null, "seconds_left": 7200, "asked_at": at});
    line(Some(&upload(json!({"server_word": word}))), false, None, T0)
}

#[test]
fn a_fresh_word_counts_down_from_asked_at() {
    let fresh = asked(300, false).expect("fresh");
    assert!(fresh.contains(&left_words(7200 - 300)), "{fresh}");
    let stop = asked(300, true).expect("time is up");
    assert!(!stop.contains(&left_words(7200 - 300)), "{stop}");
}

#[test]
fn a_null_word_or_one_older_than_11_minutes_is_unknown() {
    assert!(asked(660, false).is_ok());
    assert!(asked(661, false).is_err());
    assert!(asked(-661, false).is_err());
    assert!(line(Some(&upload(json!({}))), false, None, T0).is_err());
    assert!(line(None, true, None, T0).is_err());
}

#[test]
fn without_a_fresh_word_the_run_deadline_answers() {
    let ahead = line(None, false, Some(T0 + 7200), T0).expect("deadline");
    assert!(ahead.contains("stops on"), "{ahead}");
    assert_eq!(
        line(None, false, Some(T0 - 5), T0),
        Ok(goalcup::cli::text::TIME_IS_UP.to_owned())
    );
    assert!(line(None, false, None, T0).is_err());
}

#[test]
fn a_stale_stop_is_still_time_is_up_even_before_the_deadline() {
    let at = goalcup_format::dump::wall((T0 - 3600) * 1_000_000_000);
    let word = json!({"stop": true, "reason": "server_stop", "seconds_left": null, "asked_at": at});
    let said = line(
        Some(&upload(json!({"server_word": word}))),
        false,
        Some(T0 + 7200),
        T0,
    );
    assert_eq!(said, Ok(goalcup::cli::text::TIME_IS_UP.to_owned()));
}
