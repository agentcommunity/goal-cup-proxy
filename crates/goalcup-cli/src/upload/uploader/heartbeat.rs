//! Heartbeats and the run's own count to its deadline: the ordinary heartbeat every few
//! minutes, the stop it can bring, and the count that stops the run when the server cannot.

use super::Policy;
use super::local::local_end_io;
use crate::run::RunSignals;
use crate::upload::UploadRecord;
use crate::upload::api::{HeartbeatAnswer, HeartbeatRequest};
use crate::upload::http::{ServerClient, ServerError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Set as soon as the first final heartbeat has been sent, whatever came of it (a lost answer
/// or a `5xx` is asked again after a backoff of minutes, and the run's ordinary heartbeats must
/// not go in between). The run's heartbeat thread sends an ordinary heartbeat only while it is
/// unset, holding the lock for the whole call; the final heartbeat is sent under the same lock,
/// so an ordinary one is never in flight behind it or after it.
pub type EndedGate = std::sync::Arc<Mutex<bool>>;

/// What sending a heartbeat came to.
pub(super) enum Beat {
    /// The server answered, or did not.
    Answer(Result<HeartbeatAnswer, ServerError>),
    /// The chain holds no whole record yet: there is nothing to say.
    Empty,
    /// The chain could not be read just now (the error): not the same as having no record.
    Unreadable(String),
}

/// `POST /v1/runs/{id}/heartbeat` with the local chain's last whole record: `final` once the
/// run has ended, with the workspace repository's commit. `vitals` adds how the recording is
/// and how many model requests were seen; a heartbeat without them (`goalcup upload`) says
/// neither.
pub(super) fn heartbeat(
    server: &ServerClient,
    record: &UploadRecord,
    is_final: bool,
    repo_head: Option<String>,
    vitals: Option<&RunSignals>,
    timeout: Duration,
) -> Beat {
    let parts = match crate::upload::prune::parts_now(Path::new(&record.trace_dir)) {
        Ok(parts) => parts,
        Err(error) => return Beat::Unreadable(error.to_string()),
    };
    let (offset_local, seq, head) = match local_end_io(&parts) {
        Ok(Some(end)) => end,
        Ok(None) => return Beat::Empty,
        Err(error) => return Beat::Unreadable(error.to_string()),
    };
    let request = HeartbeatRequest {
        entry_id: record.entry_id.clone(),
        seq,
        head,
        offset_local,
        is_final,
        repo_head,
        recording: vitals.map(|vitals| vitals.recording().wire().to_string()),
        model_requests: vitals.map(RunSignals::model_requests),
        vitals: vitals.and_then(RunSignals::heartbeat_vitals),
    };
    Beat::Answer(server.json_timed(
        "POST",
        &format!("/v1/runs/{}/heartbeat", record.run_id),
        Some(&request),
        timeout,
    ))
}

/// How long one ordinary heartbeat may take: well inside the 5-minute cadence, so a server that
/// hangs cannot hold the local deadline check back for long.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

/// What the run is told when a newer run on the account has replaced it.
pub const REPLACED_REASON: &str = "a newer run on your account replaced this one";

/// Why the run must stop, from a heartbeat answer that says `stop`: a newer run replaced it,
/// or (also from a server that gives no reason) its deadline came.
pub fn stop_reason(answer: &HeartbeatAnswer) -> Option<String> {
    if !answer.stop {
        return None;
    }
    Some(match answer.reason.as_deref() {
        Some("replaced") => REPLACED_REASON.to_string(),
        _ => format!(
            "the server says the run is over (its deadline was {})",
            answer.deadline_at
        ),
    })
}

/// One ordinary heartbeat, counted in `vitals` and said when it fails; `Some` is why the run
/// must stop, and only an answer that says `stop` gives one ([`stop_reason`]). A failure of any
/// kind, every status included, is asked again at the next beat (D64): the run's own count
/// ([`LocalDeadline`]) still stops it at its deadline. Nothing is sent once the final heartbeat
/// has been sent (`ended`), and none is in flight while it goes: the lock is held for the call.
fn ordinary_beat(
    server: &ServerClient,
    record: &UploadRecord,
    vitals: &RunSignals,
    ended: &EndedGate,
    count: &mut Option<LocalDeadline>,
) -> Option<String> {
    let ended = ended.lock().unwrap_or_else(PoisonError::into_inner);
    if *ended {
        return None;
    }
    let failed = |why: String| {
        let failures = vitals.heartbeat_failed(&why);
        // Said when it starts and when it has lasted; `goalcup status` shows it from then on.
        if failures == 1 || failures == crate::run::HEARTBEATS_FAILING_AFTER {
            crate::host_log::say(&format!(
                "heartbeat: {why} ({failures} in a row); it is sent again at the next beat"
            ));
        }
    };
    // Taken before the call, so the round trip makes the count end earlier, not later. (The
    // server's whole seconds can still leave it up to a second late.)
    let asked = goalcup_protocol::clock::ContinuousOrigin::now();
    match heartbeat(server, record, false, None, Some(vitals), HEARTBEAT_TIMEOUT) {
        Beat::Answer(Ok(answer)) => {
            vitals.heartbeat_ok();
            vitals.server_answered(crate::run::Channel::Heartbeat);
            LocalDeadline::heard(count, &answer, asked);
            if let Some(count) = count {
                vitals.set_time_left(count.time_left());
            }
            stop_reason(&answer)
        }
        Beat::Answer(Err(error)) => {
            if let Some(status) = error.refusal_status()
                && vitals.server_refused(crate::run::Channel::Heartbeat, status)
            {
                crate::host_log::say(&format!(
                    "heartbeat: {}",
                    crate::participant::problem_server_refuses(status)
                ));
            }
            failed(error.to_string());
            None
        }
        Beat::Unreadable(error) => {
            failed(format!("the chain could not be read ({error})"));
            None
        }
        // Nothing recorded yet: nothing to say, and nothing failed.
        Beat::Empty => None,
    }
}

/// Send a heartbeat every `policy.heartbeat` and watch the run's own deadline, until the run
/// must stop (said once on `stopping`, and `over` set) or the uploader is `done`. Each answer
/// that says how much time is left sets the count again ([`LocalDeadline`]).
#[allow(clippy::too_many_arguments)]
pub(super) fn watch_deadline(
    server: &ServerClient,
    record: &UploadRecord,
    policy: Policy,
    local_deadline: Option<LocalDeadline>,
    vitals: &RunSignals,
    stopping: &std::sync::mpsc::Sender<String>,
    over: &AtomicBool,
    done: &AtomicBool,
    ended: &EndedGate,
) {
    let mut local_deadline = local_deadline;
    if let Some(count) = &local_deadline {
        vitals.set_time_left(count.time_left());
    }
    let mut next_beat = Instant::now();
    let tick = policy.poll.min(Duration::from_secs(1));
    while !done.load(Ordering::SeqCst) {
        // The server's count, kept here, first: it needs no network, so a hanging server cannot
        // hold it.
        let local = local_deadline.filter(LocalDeadline::passed).map(|_| {
            format!(
                "the run's time is up by the server's count (its deadline was {})",
                record.deadline_at
            )
        });
        let reason = if local.is_some() {
            local
        } else if Instant::now() >= next_beat {
            next_beat = Instant::now() + policy.heartbeat;
            ordinary_beat(server, record, vitals, ended, &mut local_deadline)
        } else {
            None
        };
        if let Some(reason) = reason {
            over.store(true, Ordering::SeqCst);
            let _ = stopping.send(reason);
            return;
        }
        std::thread::sleep(tick);
    }
}

/// The run's countdown to its deadline, for when the server cannot say stop. The server is the
/// clock (WP11): the count is set from the `seconds_left` of the open or continue answer, and set
/// again from every heartbeat answer that carries it, whatever it says (the server's answer
/// wins). It runs on a clock that keeps running while the machine sleeps
/// ([`goalcup_protocol::clock::ContinuousOrigin`]), and never reads this machine's date, which
/// the participant can change. During an outage it goes on counting down from the last answer.
///
/// Only an older server, whose answers carry no `seconds_left`, leaves the count to the rules
/// it had before: the run's length from when it opened ([`LocalDeadline::of`]), and for a
/// continued segment `deadline_at` less this machine's date ([`LocalDeadline::until`]).
#[derive(Debug, Clone, Copy)]
pub struct LocalDeadline {
    /// When the count was last set: taken before the call whose answer set it, so a slow or
    /// retried answer makes the count end earlier. It is never more than a second late: the
    /// server counts its own clock in whole seconds.
    pub from: goalcup_protocol::clock::ContinuousOrigin,
    /// What was left of the run at `from`.
    pub left: Duration,
}

impl LocalDeadline {
    /// `seconds_left` as a server answered it, counted from `asked`, which was taken before the
    /// call.
    pub fn left_from(
        seconds_left: u64,
        asked: goalcup_protocol::clock::ContinuousOrigin,
    ) -> LocalDeadline {
        LocalDeadline {
            from: asked,
            left: Duration::from_secs(seconds_left),
        }
    }

    /// The count for an entry that `goalcup start` (or `continue`, `continued`) has just had
    /// opened: the answer's `seconds_left` from `asked`, taken before the call. An older server's
    /// answer has none: then the run's length ([`LocalDeadline::of`]), or for a continued
    /// segment what this machine's date says is left ([`LocalDeadline::until`]).
    pub fn at_start(
        record: &UploadRecord,
        seconds_left: Option<u64>,
        asked: goalcup_protocol::clock::ContinuousOrigin,
        continued: bool,
    ) -> Option<LocalDeadline> {
        match (seconds_left, continued) {
            (Some(seconds), _) => Some(LocalDeadline::left_from(seconds, asked)),
            (None, false) => LocalDeadline::of(record, asked),
            (None, true) => LocalDeadline::until(&record.deadline_at),
        }
    }

    /// An older server only: the run's length from the record's two server times, from `opened`.
    pub fn of(
        record: &UploadRecord,
        opened: goalcup_protocol::clock::ContinuousOrigin,
    ) -> Option<LocalDeadline> {
        let start = goalcup_protocol::control::parse_z(&record.official_start)?;
        let deadline = goalcup_protocol::control::parse_z(&record.deadline_at)?;
        Some(LocalDeadline {
            from: opened,
            left: Duration::from_secs(deadline.checked_sub(start)?),
        })
    }

    /// An older server only: what is left of a run that is already under way, its `deadline_at`
    /// less this machine's date now (nothing if it has passed), counted from now on the
    /// continuous clock. This is the one place the date is read, and only because such a server
    /// says nothing better.
    pub fn until(deadline_at: &str) -> Option<LocalDeadline> {
        LocalDeadline::until_at(deadline_at, crate::run::now_unix())
    }

    /// [`LocalDeadline::until`] with this machine's date given (`now_unix`, epoch seconds).
    fn until_at(deadline_at: &str, now_unix: u64) -> Option<LocalDeadline> {
        let deadline = goalcup_protocol::control::parse_z(deadline_at)?;
        Some(LocalDeadline {
            from: goalcup_protocol::clock::ContinuousOrigin::now(),
            left: Duration::from_secs(deadline.saturating_sub(now_unix)),
        })
    }

    /// A heartbeat answer came, to a call made at `asked`: its `seconds_left` is the count from
    /// then on. An answer without one (an older server) leaves the count as it is.
    fn heard(
        count: &mut Option<LocalDeadline>,
        answer: &HeartbeatAnswer,
        asked: goalcup_protocol::clock::ContinuousOrigin,
    ) {
        if let Some(seconds) = answer.seconds_left {
            *count = Some(LocalDeadline::left_from(seconds, asked));
        }
    }

    /// Whether the count has run out.
    fn passed(&self) -> bool {
        u128::from(self.from.elapsed_ns()) >= self.left.as_nanos()
    }

    /// The count as the run's `state.json` keeps it ([`crate::run::TimeLeft`]), for the commands
    /// that ask whether the run is over. Its start is read a moment after the clock, so it can
    /// only end earlier than this count, by that moment.
    pub fn time_left(&self) -> crate::run::TimeLeft {
        let now = goalcup_protocol::clock::continuous_ns();
        crate::run::TimeLeft::counted(
            self.left.as_secs(),
            now.saturating_sub(self.from.elapsed_ns()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::record_in;
    use super::*;

    // ---- the server is the clock (WP11)

    fn answer_with(seconds_left: Option<u64>) -> HeartbeatAnswer {
        HeartbeatAnswer {
            stop: false,
            deadline_at: "2026-11-14T00:00:00Z".into(),
            seconds_left,
            reason: None,
        }
    }

    /// Each answer that says how much is left sets the count again, also to more time than the
    /// count had (the server's answer wins). An older server's answer leaves it alone, and with
    /// no answer at all (an outage) it goes on counting down from the last one.
    #[test]
    fn each_answer_sets_the_count_again_and_an_outage_counts_on_from_the_last() {
        use goalcup_protocol::clock::ContinuousOrigin;
        let mut count = Some(LocalDeadline::left_from(3600, ContinuousOrigin::now()));
        assert!(!count.unwrap().passed());
        LocalDeadline::heard(&mut count, &answer_with(Some(0)), ContinuousOrigin::now());
        assert!(
            count.unwrap().passed(),
            "nothing left: the count has run out"
        );
        LocalDeadline::heard(
            &mut count,
            &answer_with(Some(7200)),
            ContinuousOrigin::now(),
        );
        assert!(
            !count.unwrap().passed(),
            "the next answer wins, with more time"
        );
        assert_eq!(count.unwrap().left, Duration::from_secs(7200));
        LocalDeadline::heard(&mut count, &answer_with(None), ContinuousOrigin::now());
        assert_eq!(
            count.unwrap().left,
            Duration::from_secs(7200),
            "an older server's answer changes nothing"
        );
        // A run whose record gave no count takes the first answer's.
        let mut none = None;
        LocalDeadline::heard(&mut none, &answer_with(Some(5)), ContinuousOrigin::now());
        assert_eq!(none.unwrap().left, Duration::from_secs(5));

        // The outage: no answer comes, and the count runs out on its own.
        let last = LocalDeadline {
            from: ContinuousOrigin::now(),
            left: Duration::from_millis(200),
        };
        assert!(!last.passed());
        std::thread::sleep(Duration::from_millis(300));
        assert!(last.passed(), "the count went on through the outage");
    }

    /// An entry's first count is the server's `seconds_left`, whatever this machine's date says,
    /// for a new run and a continued segment alike. Only an older server's answer, which has
    /// none, falls back to the run's length or (continued) to the date.
    #[test]
    fn the_first_count_is_the_servers_time_left_and_never_the_date() {
        use goalcup_protocol::clock::ContinuousOrigin;
        let record = UploadRecord {
            official_start: "2099-01-01T00:00:00Z".into(),
            deadline_at: "2099-01-08T00:00:00Z".into(),
            ..record_in(Path::new("/nowhere"))
        };
        let asked = ContinuousOrigin::now();
        for continued in [false, true] {
            let count = LocalDeadline::at_start(&record, Some(90), asked, continued).unwrap();
            assert_eq!(count.left, Duration::from_secs(90));
            assert_eq!(count.from, asked, "counted from when the server was asked");
        }
        let fresh = LocalDeadline::at_start(&record, None, asked, false).unwrap();
        assert_eq!(
            fresh.left,
            Duration::from_secs(168 * 3600),
            "the run's length"
        );
        // A continued segment on an older server reads the date (now, long before the deadline).
        let continued = LocalDeadline::at_start(&record, None, asked, true).unwrap();
        assert!(continued.left > Duration::ZERO);
    }

    /// The older server's rule for a continued segment, on a skewed date: a deadline the date
    /// says has passed gives nothing left, and one in the future is counted on the continuous
    /// clock from now. (A date set a day back gives a day more: why the server's count wins.)
    #[test]
    fn until_on_a_skewed_date_counts_what_the_date_says_on_the_continuous_clock() {
        let deadline = "2026-11-14T00:00:00Z";
        let at = goalcup_protocol::control::parse_z(deadline).unwrap();
        let past = LocalDeadline::until_at(deadline, at + 24 * 3600).unwrap();
        assert_eq!(past.left, Duration::ZERO);
        assert!(past.passed(), "a deadline in the past has run out at once");
        let future = LocalDeadline::until_at(deadline, at - 90).unwrap();
        assert_eq!(future.left, Duration::from_secs(90));
        assert!(
            future.from.elapsed_ns() < 5_000_000_000,
            "counted from now on the continuous clock"
        );
        assert!(!future.passed());
        let set_back = LocalDeadline::until_at(deadline, at - 90 - 24 * 3600).unwrap();
        assert_eq!(set_back.left, Duration::from_secs(90 + 24 * 3600));
    }
}
