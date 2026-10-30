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
    line(Some(&upload(json!({"server_word": word}))), false, T0)
}

#[test]
fn a_fresh_word_counts_down_from_asked_at() {
    let fresh = asked(300, false).expect("fresh");
    assert!(fresh.contains(&left_words(7200 - 300)), "{fresh}");
    let stop = asked(300, true).expect("time is up");
    assert_ne!(stop, fresh);
}

#[test]
fn a_null_word_or_one_older_than_11_minutes_is_unknown() {
    assert!(asked(660, false).is_ok());
    assert!(asked(661, false).is_err());
    assert!(asked(-661, false).is_err());
    assert!(line(Some(&upload(json!({}))), false, T0).is_err());
    assert!(line(None, true, T0).is_err());
}
