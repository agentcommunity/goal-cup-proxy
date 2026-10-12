//! The one place a `busy` daemon is asked again.
//!
//! # Why it is not in the transport
//!
//! `busy` is the daemon holding an entry for the length of one asynchronous call. It is
//! raised on every mutating route while an entry is mid-pump, it is transient by
//! construction, and the same request a moment later is expected to succeed. Most mutable
//! calls use the bounded generic retry below. The entry-runtime `/abandon` and the `/process`
//! handoff share one dephased schedule with separately stated end-to-end limits: the abandon
//! must span an upload hold, and `/process` is made while a freshly launched child is waiting
//! for daemon ownership. The capture seal has its own retry against the local-finish budget.
//! `conflict` shares the `409` and means the opposite: already sealed, already started, a
//! lock still on disk. It is never retried, here or anywhere.
//!
//! It is not inside `HttpDaemonClient::call`, because a command is written against the
//! [`DaemonClient`](goalcup_protocol::control::DaemonClient) **trait**, and in every command
//! test it is driven by the in-process `MockDaemonClient`. A retry that only the HTTP client
//! has is a retry the mock does not have: `busy_once("abandon")` would hand the command an
//! `Err(Busy)`, the command would stop at the first refusal, and the retry rule could not be
//! tested anywhere the commands are actually tested. The transport is not where the decision
//! belongs either way; whether a refusal is worth asking again is a question about the
//! *call*, not about the socket.
//!
//! So it is here, above the trait, and both clients behave the same underneath it:
//!
//! ```no_run
//! # use goalcup_cli::retry::retry_busy;
//! # fn f(client: &dyn goalcup_protocol::control::DaemonClient, entry_id: &str)
//! #     -> Result<(), goalcup_protocol::control::ControlError> {
//! retry_busy(|| client.abandon(entry_id, "stopping early"))?;
//! # Ok(()) }
//! ```
//!
//! # The two bounds, and why there are two
//!
//! [`BUSY_ATTEMPTS`] bounds the asking: three, not "until it works", because a daemon that
//! answers `busy` forever has a stuck pump and a client that asked forever would hang a
//! participant's terminal with nothing on screen to say why.
//!
//! [`BUSY_BUDGET`] bounds the *waiting*, and it is the bound the attempt count alone does not
//! give. Attempts times a read deadline is the real worst case, and with the seal's
//! five-minute deadline that arithmetic comes to a quarter of an hour of held terminal. The
//! budget is written in ordinary calls — twice [`crate::daemon::CALL_TIMEOUT`], never the
//! slow one — and the capture seal is kept out of this *generic* helper entirely:
//! [`retry_capture_seal`] works against the finish budget instead. When the budget is spent
//! the `busy` surfaces: the last one the daemon actually said, with its own words in it.

use goalcup_protocol::control::{CaptureSealError, ControlError, FINISH_COMMIT_RESERVE};
use std::time::{Duration, Instant};

/// How many times one call is made before a `busy` daemon is reported as busy.
pub const BUSY_ATTEMPTS: usize = 3;

/// The wait between two of those attempts. One pump cycle, roughly.
pub const BUSY_BACKOFF: Duration = Duration::from_millis(200);

/// How often that wait looks at [`crate::arming::interrupted`].
///
/// A participant who pressed Ctrl-C while a busy daemon was being asked again is waiting on
/// a sleep this file chose, and the honest thing to do with the rest of it is to stop
/// taking it. Twenty milliseconds is below what a terminal notices and above what a poll
/// costs.
const BACKOFF_SLICE: Duration = Duration::from_millis(20);

/// The wall clock the whole of a retried call is allowed, start to refusal.
///
/// Two ordinary calls, and the constant is written out of [`crate::daemon::CALL_TIMEOUT`]
/// rather than beside it so the two cannot drift. A retried call that has already spent
/// this long is a call whose participant has been waiting a minute for a terminal to say
/// something, and the honest thing to say is the `busy` the daemon gave.
pub const BUSY_BUDGET: Duration = Duration::from_secs(crate::daemon::CALL_TIMEOUT.as_secs() * 2);

/// `call`, retried while it answers [`ControlError::Busy`] — at most [`BUSY_ATTEMPTS`]
/// times, [`BUSY_BACKOFF`] apart, within [`BUSY_BUDGET`] of wall clock.
///
/// Every other answer, success or refusal, is returned the first time it arrives. The branch
/// is on the `ControlError` and never on the `409` that `busy` and `conflict` share.
pub fn retry_busy<T>(call: impl FnMut() -> Result<T, ControlError>) -> Result<T, ControlError> {
    retry_busy_within(BUSY_BUDGET, call)
}

/// [`retry_busy`] with the budget named, which is the only thing a test can shorten.
///
/// A minute is not a thing a test suite may wait, and a budget that could only be exercised
/// by waiting one would be a bound nothing checks. This is that seam and it is the whole of
/// it: the attempt count, the backoff and the branch are the same code either way.
pub fn retry_busy_within<T>(
    budget: Duration,
    mut call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let started = Instant::now();
    // The daemon's own words for why it is busy, kept so the refusal a participant reads is
    // the last thing the daemon actually said rather than a sentence this file invented.
    let mut said = String::new();
    for attempt in 1..=BUSY_ATTEMPTS {
        match call() {
            Err(ControlError::Busy(message)) => {
                said = message;
                if attempt == BUSY_ATTEMPTS {
                    break;
                }
                // The sleep is part of the budget, so a wait that would end past it is not
                // taken at all: the refusal arrives at the budget, not a backoff after it.
                if started.elapsed().saturating_add(BUSY_BACKOFF) >= budget {
                    break;
                }
                // And a Ctrl-C ends it early: the `busy` the daemon last said surfaces now
                // rather than four hundred milliseconds after the participant asked this
                // process to stop.
                if !backoff() {
                    break;
                }
            }
            answered => return answered,
        }
    }
    Err(ControlError::Busy(said))
}

/// One [`BUSY_BACKOFF`], in slices, ending early where this process has been asked to stop.
///
/// `false` says the wait was cut short, which is a `busy` to surface rather than an attempt
/// to make.
fn backoff() -> bool {
    let until = Instant::now() + BUSY_BACKOFF;
    while Instant::now() < until {
        if crate::arming::interrupted() {
            return false;
        }
        std::thread::sleep(BACKOFF_SLICE);
    }
    true
}

/// The fixed, deliberately non-periodic waits of the scheduled policies: offsets 0, 100, 275,
/// 550, 1,050, 2,050, 4,050, 8,050 and 12,050 ms, nine asks in all. They span one daemon
/// upload hold of its ordinary ten-second network timeout and escape every phase of the
/// accelerated 200 ms capture cadence's 120 ms hold, without randomness in a participant's
/// reproduction of a refusal.
const DEPHASED_BACKOFFS: [Duration; 8] = [
    Duration::from_millis(100),
    Duration::from_millis(175),
    Duration::from_millis(275),
    Duration::from_millis(500),
    Duration::from_millis(1_000),
    Duration::from_millis(2_000),
    Duration::from_millis(4_000),
    Duration::from_millis(4_000),
];

/// The retry envelope of a scheduled policy: the 12,050 ms of planned sleep in
/// [`DEPHASED_BACKOFFS`] plus 950 ms of explicit scheduler headroom (never another delay).
const DEPHASED_RETRY_ENVELOPE: Duration = Duration::from_secs(13);

/// The wall-clock reservation for an entry-runtime call (`/abandon`), start to final refusal.
///
/// One current ordinary call, one next ordinary call, and the 13-second retry envelope make
/// a strict 73-second ceiling. A `busy` is expected to answer immediately; reserving two full
/// transport windows keeps a late current response from starting another request that cannot
/// finish inside the stated bound.
pub(crate) const ENTRY_BUSY_BUDGET: Duration = Duration::new(
    crate::daemon::CALL_TIMEOUT.as_secs() * 2 + DEPHASED_RETRY_ENVELOPE.as_secs(),
    DEPHASED_RETRY_ENVELOPE.subsec_nanos(),
);

/// The wall-clock ceiling for a real `/process` handoff, start to final refusal.
///
/// Nine [`crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT`] windows plus the 13-second retry
/// envelope is 31 seconds. These equalities are pinned in deterministic tests below so a
/// route deadline or schedule change also makes the participant-visible bound an intentional
/// choice.
pub(crate) const PROCESS_BUSY_BUDGET: Duration = Duration::new(
    crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT.as_secs() * (DEPHASED_BACKOFFS.len() as u64 + 1)
        + DEPHASED_RETRY_ENVELOPE.as_secs(),
    DEPHASED_RETRY_ENVELOPE.subsec_nanos(),
);

/// One scheduled policy: its waits, its wall-clock budget, and the transport window every
/// further request must still fit inside that budget.
struct Schedule {
    backoffs: &'static [Duration],
    budget: Duration,
    call_window: Duration,
}

/// `/abandon`: nine asks over [`ENTRY_BUSY_BUDGET`], each with an ordinary call's window.
const ENTRY_SCHEDULE: Schedule = Schedule {
    backoffs: &DEPHASED_BACKOFFS,
    budget: ENTRY_BUSY_BUDGET,
    call_window: crate::daemon::CALL_TIMEOUT,
};

/// `/process`: nine asks over [`PROCESS_BUSY_BUDGET`], each with the handoff's short window.
const PROCESS_SCHEDULE: Schedule = Schedule {
    backoffs: &DEPHASED_BACKOFFS,
    budget: PROCESS_BUSY_BUDGET,
    call_window: crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
};

/// Retries only a definite [`ControlError::Busy`] from an entry-runtime route (`/abandon`).
///
/// The daemon emits `busy` only before borrowing the entry runtime, so the call can repeat
/// that definite pre-operation refusal. `Unreachable`, timeout, malformed, conflict, and
/// internal responses are returned once. Each wait uses the shared 20 ms interruption slices,
/// and a retry is made only where its wait and one full ordinary transport window fit
/// [`ENTRY_BUSY_BUDGET`].
pub(crate) fn retry_busy_entry<T>(
    call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let started = Instant::now();
    retry_scheduled(
        &ENTRY_SCHEDULE,
        |error| matches!(error, ControlError::Busy(_)),
        || started.elapsed(),
        dephased_backoff,
        call,
    )
}

/// Retries a pre-admission [`ControlError::Busy`], a post-Prepare
/// [`ControlError::ProcessAdmissionBusy`], or an ambiguous [`ControlError::Unreachable`] from
/// the `/process` handoff.
///
/// `Unreachable` is the narrow exception because durable `Prepared`/`Registered` makes this
/// exact route request idempotent: it can mean the daemon accepted the body and the `204` was
/// lost. Every other result is returned immediately, including a `conflict` that shares HTTP
/// 409 with `busy`, and a malformed or internal response. The closure owns no request
/// construction: callers capture one immutable
/// [`ProcessRegistration`](goalcup_protocol::process::ProcessRegistration) and submit that
/// same document on each attempt, within [`PROCESS_BUSY_BUDGET`].
pub(crate) fn retry_busy_process<T>(
    call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let started = Instant::now();
    retry_scheduled(
        &PROCESS_SCHEDULE,
        |error| {
            matches!(
                error,
                ControlError::Busy(_)
                    | ControlError::ProcessAdmissionBusy(_)
                    | ControlError::Unreachable(_)
            )
        },
        || started.elapsed(),
        dephased_backoff,
        call,
    )
}

/// The testable core of the scheduled policies. `elapsed` is the call's wall clock; `wait`
/// returns false only when the local process has been interrupted, which surfaces the last
/// retryable refusal rather than making another request.
///
/// It makes at most one call more than there are waits. Before it takes a wait, it reserves
/// both that wait and the next request's transport window inside the budget; after the wait it
/// checks again, because `thread::sleep` can wake late. A spent budget surfaces the last
/// retryable refusal without making a further request.
fn retry_scheduled<T>(
    schedule: &Schedule,
    retryable: impl Fn(&ControlError) -> bool,
    mut elapsed: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration) -> bool,
    mut call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let mut attempt = 0;
    loop {
        let error = match call() {
            Err(error) if retryable(&error) => error,
            answered => return answered,
        };
        let Some(&delay) = schedule.backoffs.get(attempt) else {
            return Err(error);
        };
        if elapsed()
            .saturating_add(delay)
            .saturating_add(schedule.call_window)
            > schedule.budget
        {
            return Err(error);
        }
        if !wait(delay) {
            return Err(error);
        }
        if elapsed().saturating_add(schedule.call_window) > schedule.budget {
            return Err(error);
        }
        attempt += 1;
    }
}

/// How many times the capture seal is asked before its final `busy` is reported: one more
/// than [`SEAL_BUSY_BACKOFFS`].
const SEAL_BUSY_ATTEMPTS: usize = 4;

/// Deliberately non-periodic waits for the capture seal: the first three of
/// [`DEPHASED_BACKOFFS`]. Offsets 0, 100, 275, and 550 ms escape every phase of the
/// accelerated 200 ms capture cadence's 120 ms hold.
const SEAL_BUSY_BACKOFFS: [Duration; SEAL_BUSY_ATTEMPTS - 1] = [
    Duration::from_millis(100),
    Duration::from_millis(175),
    Duration::from_millis(275),
];

/// The capture seal's retry (CONTRACTS.md, H-10 4a ¶7): a dephased schedule against the one
/// local-finish budget.
///
/// Retries only [`CaptureSealError::NotAdmitted`] carrying `busy`, on [`SEAL_BUSY_BACKOFFS`],
/// and only while the wait still leaves [`FINISH_COMMIT_RESERVE`] of `deadline`. Every other
/// answer returns at once: `finish_deadline`, an ambiguous failure and an unauthorized or
/// refused connection are never replayed by this call. Every attempt is built from what remains
/// of `deadline` (the closure receives it), so a retried request states the truth.
pub fn retry_capture_seal<T>(
    deadline: Instant,
    call: impl FnMut(Duration) -> Result<T, CaptureSealError>,
) -> Result<T, CaptureSealError> {
    retry_capture_seal_with(
        || deadline.saturating_duration_since(Instant::now()),
        dephased_backoff,
        call,
    )
}

/// The testable core of [`retry_capture_seal`]. `remaining` is what is left of the budget;
/// `wait` returns false only when the local process has been interrupted.
fn retry_capture_seal_with<T>(
    mut remaining: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration) -> bool,
    mut call: impl FnMut(Duration) -> Result<T, CaptureSealError>,
) -> Result<T, CaptureSealError> {
    let mut said = String::new();
    for attempt in 0..SEAL_BUSY_ATTEMPTS {
        match call(remaining()) {
            Err(CaptureSealError::NotAdmitted(ControlError::Busy(message))) => {
                said = message;
                let Some(delay) = SEAL_BUSY_BACKOFFS.get(attempt) else {
                    break;
                };
                // The wait must leave the commit share: a retry that spends the reserve would
                // make the finish refuse itself.
                if remaining().saturating_sub(*delay) < FINISH_COMMIT_RESERVE {
                    break;
                }
                if !wait(*delay) {
                    break;
                }
                // `thread::sleep` can wake late; recheck so an overslept wait cannot start a
                // request inside the reserve.
                if remaining() < FINISH_COMMIT_RESERVE {
                    break;
                }
            }
            answered => return answered,
        }
    }
    Err(CaptureSealError::NotAdmitted(ControlError::Busy(said)))
}

/// One dephased route wait, without extending the nominal delay at its last slice.
fn dephased_backoff(delay: Duration) -> bool {
    let until = Instant::now() + delay;
    loop {
        if crate::arming::interrupted() {
            return false;
        }
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(BACKOFF_SLICE));
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    use std::cell::Cell;

    fn entry<T>(
        elapsed: impl FnMut() -> Duration,
        wait: impl FnMut(Duration) -> bool,
        call: impl FnMut() -> Result<T, ControlError>,
    ) -> Result<T, ControlError> {
        retry_scheduled(
            &ENTRY_SCHEDULE,
            |error| matches!(error, ControlError::Busy(_)),
            elapsed,
            wait,
            call,
        )
    }

    fn process<T>(
        elapsed: impl FnMut() -> Duration,
        wait: impl FnMut(Duration) -> bool,
        call: impl FnMut() -> Result<T, ControlError>,
    ) -> Result<T, ControlError> {
        retry_scheduled(
            &PROCESS_SCHEDULE,
            |error| {
                matches!(
                    error,
                    ControlError::Busy(_)
                        | ControlError::ProcessAdmissionBusy(_)
                        | ControlError::Unreachable(_)
                )
            },
            elapsed,
            wait,
            call,
        )
    }

    #[test]
    fn the_schedule_is_exact_and_its_budgets_are_pinned() {
        assert_eq!(
            DEPHASED_BACKOFFS.to_vec(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(175),
                Duration::from_millis(275),
                Duration::from_millis(500),
                Duration::from_millis(1_000),
                Duration::from_millis(2_000),
                Duration::from_millis(4_000),
                Duration::from_millis(4_000),
            ]
        );
        let planned_wait = DEPHASED_BACKOFFS.iter().copied().sum::<Duration>();
        assert_eq!(planned_wait, Duration::from_millis(12_050));
        assert_eq!(
            DEPHASED_RETRY_ENVELOPE,
            planned_wait + Duration::from_millis(950),
            "the retry envelope is planned sleep plus 950 ms of scheduler headroom"
        );
        assert_eq!(
            ENTRY_BUSY_BUDGET,
            crate::daemon::CALL_TIMEOUT * 2 + DEPHASED_RETRY_ENVELOPE
        );
        assert_eq!(ENTRY_BUSY_BUDGET, Duration::from_secs(73));
        assert_eq!(
            crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
            Duration::from_secs(2),
            "the handoff leaves the daemon time for durable registry and trace writes"
        );
        assert_eq!(
            PROCESS_BUSY_BUDGET,
            crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT * 9 + DEPHASED_RETRY_ENVELOPE,
            "nine 2 s route windows and the 13 s retry envelope are the strict ceiling"
        );
        assert_eq!(PROCESS_BUSY_BUDGET, Duration::from_secs(31));
        assert_eq!(SEAL_BUSY_BACKOFFS, DEPHASED_BACKOFFS[..3]);
    }

    #[test]
    fn an_entry_call_retries_busy_on_the_exact_non_periodic_schedule() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = entry(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                elapsed.set(elapsed.get() + delay);
                true
            },
            || {
                attempts += 1;
                if attempts < 9 {
                    Err(ControlError::Busy(format!("pump {attempts}")))
                } else {
                    Ok("abandoned")
                }
            },
        );

        assert!(matches!(result, Ok("abandoned")));
        assert_eq!(attempts, 9);
        assert_eq!(waits, DEPHASED_BACKOFFS.to_vec());
    }

    #[test]
    fn both_schedules_outlast_one_ten_second_upload_hold() {
        for (name, retried) in [("entry", false), ("process", true)] {
            let elapsed = Cell::new(Duration::ZERO);
            let mut attempts = Vec::new();
            let held = || {
                if elapsed.get() < Duration::from_secs(10) {
                    Err(ControlError::Busy(
                        "the upload still holds the entry".into(),
                    ))
                } else {
                    Ok("answered")
                }
            };
            let wait = |delay| {
                elapsed.set(elapsed.get() + delay);
                true
            };
            let mut call = || {
                attempts.push(elapsed.get());
                held()
            };
            let result = if retried {
                process(|| elapsed.get(), wait, &mut call)
            } else {
                entry(|| elapsed.get(), wait, &mut call)
            };

            assert!(matches!(result, Ok("answered")), "{name}");
            assert_eq!(
                attempts,
                vec![
                    Duration::ZERO,
                    Duration::from_millis(100),
                    Duration::from_millis(275),
                    Duration::from_millis(550),
                    Duration::from_millis(1_050),
                    Duration::from_millis(2_050),
                    Duration::from_millis(4_050),
                    Duration::from_millis(8_050),
                    Duration::from_millis(12_050),
                ],
                "{name}: the ninth ask is the first one after a 10 s upload hold"
            );
        }
    }

    #[test]
    fn the_ninth_retryable_refusal_surfaces_without_a_tenth_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;
        let result: Result<(), ControlError> = entry(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Busy(format!("pump {attempts}")))
            },
        );
        assert!(matches!(result, Err(ControlError::Busy(message)) if message == "pump 9"));
        assert_eq!((attempts, waits.len()), (9, 8));

        let mut waits = Vec::new();
        let mut attempts = 0;
        let result: Result<(), ControlError> = process(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Unreachable(format!(
                    "daemon absent {attempts}"
                )))
            },
        );
        assert!(
            matches!(result, Err(ControlError::Unreachable(message)) if message == "daemon absent 9")
        );
        assert_eq!(attempts, 9);
        assert_eq!(waits, DEPHASED_BACKOFFS.to_vec());
    }

    #[test]
    fn attempts_escape_every_phase_of_a_200ms_120ms_hold() {
        for start_ms in 0..120_u64 {
            let elapsed_ms = Cell::new(0_u64);
            let attempts = Cell::new(0_usize);

            let result: Result<(), ControlError> = entry(
                || Duration::from_millis(elapsed_ms.get()),
                |delay| {
                    elapsed_ms.set(elapsed_ms.get() + delay.as_millis() as u64);
                    true
                },
                || {
                    attempts.set(attempts.get() + 1);
                    if (start_ms + elapsed_ms.get()) % 200 < 120 {
                        Err(ControlError::Busy(
                            "the upload still holds the entry".into(),
                        ))
                    } else {
                        Ok(())
                    }
                },
            );

            assert!(result.is_ok(), "start phase {start_ms} ms stayed held");
            assert!(attempts.get() <= 9);
            assert!(elapsed_ms.get() <= 12_050);
        }
    }

    #[test]
    fn no_wait_is_taken_without_room_for_the_next_transport_window() {
        for (name, budget, window) in [
            ("entry", ENTRY_BUSY_BUDGET, crate::daemon::CALL_TIMEOUT),
            (
                "process",
                PROCESS_BUSY_BUDGET,
                crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
            ),
        ] {
            let elapsed = budget - Duration::from_millis(100) - window + Duration::from_millis(1);
            let mut waits = Vec::new();
            let mut attempts = 0;
            let wait = |delay| {
                waits.push(delay);
                true
            };
            let call = || {
                attempts += 1;
                Err::<(), _>(ControlError::Busy("held".into()))
            };
            let result = if name == "entry" {
                entry(|| elapsed, wait, call)
            } else {
                process(|| elapsed, wait, call)
            };

            assert!(matches!(result, Err(ControlError::Busy(_))), "{name}");
            assert_eq!(attempts, 1, "{name}");
            assert!(
                waits.is_empty(),
                "{name}: no retry begins without its window"
            );
        }
    }

    #[test]
    fn the_budget_is_rechecked_after_an_overslept_wait() {
        for (name, budget, window) in [
            ("entry", ENTRY_BUSY_BUDGET, crate::daemon::CALL_TIMEOUT),
            (
                "process",
                PROCESS_BUSY_BUDGET,
                crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
            ),
        ] {
            let elapsed = Cell::new(budget - Duration::from_millis(100) - window);
            let mut waits = Vec::new();
            let mut attempts = 0;
            let wait = |delay| {
                waits.push(delay);
                elapsed.set(elapsed.get() + delay + Duration::from_millis(1));
                true
            };
            let call = || {
                attempts += 1;
                Err::<(), _>(ControlError::Busy("held".into()))
            };
            let result = if name == "entry" {
                entry(|| elapsed.get(), wait, call)
            } else {
                process(|| elapsed.get(), wait, call)
            };

            assert!(matches!(result, Err(ControlError::Busy(_))), "{name}");
            assert_eq!(attempts, 1, "{name}");
            assert_eq!(waits, vec![Duration::from_millis(100)], "{name}");
        }
    }

    #[test]
    fn an_interrupted_wait_surfaces_the_refusal_without_another_attempt() {
        for retried in [
            ControlError::Busy("held".into()),
            ControlError::ProcessAdmissionBusy("Prepared is waiting".into()),
        ] {
            let expected = retried.code();
            let mut retried = Some(retried);
            let mut waits = Vec::new();
            let mut attempts = 0;
            let result: Result<(), ControlError> = process(
                || Duration::ZERO,
                |delay| {
                    waits.push(delay);
                    false
                },
                || {
                    attempts += 1;
                    Err(retried
                        .take()
                        .expect("the interrupt prevents a second attempt"))
                },
            );
            assert_eq!(result.expect_err("surfaced").code(), expected);
            assert_eq!(attempts, 1);
            assert_eq!(waits, vec![Duration::from_millis(100)]);
        }
    }

    #[test]
    fn every_other_result_is_one_shot() {
        let both_one_shot = || {
            vec![
                ControlError::NotFound("entry absent".into()),
                ControlError::Conflict("entry changed".into()),
                ControlError::Unauthorized("wrong bearer".into()),
                ControlError::ArmingTimeout {
                    detail: "the server refused the gate".into(),
                },
                ControlError::PreflightFailed("server gate failed".into()),
                ControlError::UnsupportedConnector("unknown harness".into()),
                ControlError::Malformed("bad reply".into()),
                ControlError::Internal("daemon failed".into()),
                ControlError::Unsupported("wrong platform".into()),
                ControlError::ProcessAdmissionUncertain("Prepared was durable".into()),
            ]
        };
        let cases = [
            ControlError::Unreachable("daemon absent".into()),
            ControlError::ProcessAdmissionBusy("not an entry-runtime refusal".into()),
        ]
        .into_iter()
        .chain(both_one_shot())
        .map(|error| (false, error))
        .chain(both_one_shot().into_iter().map(|error| (true, error)));
        for (is_process, error) in cases {
            let expected = error.code();
            let mut error = Some(error);
            let mut attempts = 0;
            let mut waits = Vec::new();
            let wait = |delay| {
                waits.push(delay);
                true
            };
            let call = || {
                attempts += 1;
                Err::<(), _>(error.take().expect("the result is one-shot"))
            };
            let result = if is_process {
                process(|| Duration::ZERO, wait, call)
            } else {
                entry(|| Duration::ZERO, wait, call)
            };

            assert_eq!(result.expect_err("the refusal surfaces").code(), expected);
            assert_eq!(attempts, 1, "{expected} was retried");
            assert!(waits.is_empty(), "{expected} scheduled a wait");
        }
    }
}

#[cfg(test)]
mod capture_seal_retry_tests {
    use super::*;
    use goalcup_protocol::control::{CaptureSealError, FINISH_COMMIT_RESERVE};
    use std::cell::Cell;

    fn busy() -> CaptureSealError {
        CaptureSealError::NotAdmitted(ControlError::Busy("held".into()))
    }

    #[test]
    fn capture_seal_busy_retry_needs_commit_reserve() {
        // Plenty left: all four attempts on the fixed dephased schedule.
        let left_ms = Cell::new(10_000_u64);
        let mut waits = Vec::new();
        let mut attempts = 0;
        let result: Result<(), CaptureSealError> = retry_capture_seal_with(
            || Duration::from_millis(left_ms.get()),
            |delay| {
                waits.push(delay);
                left_ms.set(left_ms.get() - delay.as_millis() as u64);
                true
            },
            |_| {
                attempts += 1;
                Err(busy())
            },
        );
        assert!(matches!(
            result,
            Err(CaptureSealError::NotAdmitted(ControlError::Busy(_)))
        ));
        assert_eq!(attempts, SEAL_BUSY_ATTEMPTS);
        assert_eq!(waits, SEAL_BUSY_BACKOFFS.to_vec());

        // Not enough left to wait and still leave the commit share: the busy surfaces at once,
        // never a wait that spends the reserve and never the old 300-second reservation.
        let reserve = FINISH_COMMIT_RESERVE.as_millis() as u64;
        let left_ms = Cell::new(reserve + 99);
        let mut attempts = 0;
        let mut waited = false;
        let result: Result<(), CaptureSealError> = retry_capture_seal_with(
            || Duration::from_millis(left_ms.get()),
            |_| {
                waited = true;
                true
            },
            |_| {
                attempts += 1;
                Err(busy())
            },
        );
        assert!(matches!(
            result,
            Err(CaptureSealError::NotAdmitted(ControlError::Busy(_)))
        ));
        assert_eq!((attempts, waited), (1, false));

        // Exactly the first delay plus the reserve: the wait is taken.
        let left_ms = Cell::new(reserve + 100);
        let mut attempts = 0;
        let _: Result<(), CaptureSealError> = retry_capture_seal_with(
            || Duration::from_millis(left_ms.get()),
            |delay| {
                left_ms.set(left_ms.get() - delay.as_millis() as u64);
                true
            },
            |_| {
                attempts += 1;
                Err(busy())
            },
        );
        assert_eq!(attempts, 2, "the second attempt fits, the third does not");
    }

    #[test]
    fn every_capture_seal_answer_but_not_admitted_busy_returns_at_once() {
        for (name, answer) in [
            (
                "finish_deadline",
                Err(CaptureSealError::MaybeAdmitted(
                    ControlError::FinishDeadline("local_finish".into()),
                )),
            ),
            (
                "unauthorized",
                Err(CaptureSealError::NotAdmitted(ControlError::Unauthorized(
                    "x".into(),
                ))),
            ),
            (
                "maybe busy",
                Err(CaptureSealError::MaybeAdmitted(ControlError::Busy(
                    "x".into(),
                ))),
            ),
            ("ok", Ok(())),
        ] {
            let attempts = Cell::new(0);
            let mut answer = Some(answer);
            let result = retry_capture_seal_with(
                || Duration::from_secs(10),
                |_| panic!("{name} must not wait"),
                |_| {
                    attempts.set(attempts.get() + 1);
                    answer.take().expect("one call")
                },
            );
            assert_eq!(attempts.get(), 1, "{name} is asked once");
            assert_eq!(result.is_ok(), name == "ok", "{name}");
        }
    }

    #[test]
    fn each_attempt_is_built_from_what_remains_of_the_budget() {
        let left_ms = Cell::new(9_000_u64);
        let mut seen = Vec::new();
        let _: Result<(), CaptureSealError> = retry_capture_seal_with(
            || Duration::from_millis(left_ms.get()),
            |delay| {
                left_ms.set(left_ms.get() - delay.as_millis() as u64);
                true
            },
            |remaining| {
                seen.push(remaining.as_millis() as u64);
                Err(busy())
            },
        );
        assert_eq!(seen, vec![9_000, 8_900, 8_725, 8_450]);
    }

    #[test]
    fn an_interrupted_capture_seal_wait_surfaces_busy_without_another_attempt() {
        let mut attempts = 0;
        let result: Result<(), CaptureSealError> = retry_capture_seal_with(
            || Duration::from_secs(10),
            |_| false,
            |_| {
                attempts += 1;
                Err(busy())
            },
        );
        assert!(matches!(
            result,
            Err(CaptureSealError::NotAdmitted(ControlError::Busy(_)))
        ));
        assert_eq!(attempts, 1);
    }
}
