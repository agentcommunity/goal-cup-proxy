//! The one place a `busy` daemon is asked again.
//!
//! # Why it is not in the transport
//!
//! `busy` is the daemon holding an entry for the length of one asynchronous call. It is
//! raised on every mutating route while an entry is mid-pump, it is transient by
//! construction, and the same request a moment later is expected to succeed. Most mutable
//! calls use the bounded generic retry below. Selected entry-runtime calls (`/arm`, `/abandon`,
//! resume preview/CAS, and `/process/cancel`), the `/process` handoff, and terminal `/seal`
//! have deliberately dephased policies with separately stated end-to-end limits: entry calls
//! must span an upload hold, `/process` is made while a freshly launched child is waiting for
//! daemon ownership, and `/seal` has to preserve room for one slow, admitted manifest
//! operation.
//! `conflict` shares the `409` and means the opposite: already sealed, already started, a
//! lock still on disk. It is never retried, here or anywhere.
//!
//! It is not inside `HttpDaemonClient::call`, because a command is written against the
//! [`DaemonClient`](goalcup_protocol::control::DaemonClient) **trait**, and in every command
//! test it is driven by the in-process `MockDaemonClient`. A retry that only the HTTP client
//! has is a retry the mock does not have: `busy_once("resume")` would hand the command an
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
//! retry_busy(|| client.arm(entry_id))?;
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
//! slow one — and the terminal `/seal` route is kept out of this *generic* helper entirely.
//! [`retry_busy_seal`] gives it a distinct reservation. When the budget is spent the `busy`
//! surfaces: the last one the daemon actually said, with its own words in it.

use goalcup_protocol::control::{
    CaptureSealError, ControlError, FINISH_COMMIT_RESERVE, ProcessRegistrationError,
};
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

/// How many times a selected entry-runtime call is asked before its final `busy` is reported.
///
/// These calls all reject `busy` before performing their operation. Nine bounded asks span
/// one ordinary daemon upload hold without widening retry semantics for unrelated routes.
pub const ENTRY_BUSY_ATTEMPTS: usize = 9;

/// The maximum nominal sleep after the first selected entry-runtime `busy`: 12,050 ms.
pub const ENTRY_BUSY_MAX_SLEEP: Duration = Duration::from_millis(12_050);

/// Explicit scheduler allowance around the interruptible entry-runtime waits.
pub const ENTRY_BUSY_SCHEDULER_HEADROOM: Duration = Duration::from_millis(950);

/// Planned entry-runtime sleeps plus bounded scheduler headroom.
pub const ENTRY_BUSY_RETRY_ENVELOPE: Duration = Duration::from_secs(13);

/// The wall-clock reservation for a selected entry-runtime call.
///
/// One current ordinary call, one next ordinary call, and the 13-second retry envelope make
/// a strict 73-second ceiling. A `busy` is expected to answer immediately; reserving two full
/// transport windows keeps a late current response from starting another request that cannot
/// finish inside the stated bound.
pub const ENTRY_BUSY_BUDGET: Duration = Duration::new(
    crate::daemon::CALL_TIMEOUT.as_secs() * 2 + ENTRY_BUSY_RETRY_ENVELOPE.as_secs(),
    ENTRY_BUSY_RETRY_ENVELOPE.subsec_nanos(),
);

/// Fixed entry-runtime waits, producing offsets 0, 100, 275, 550, 1,050, 2,050, 4,050,
/// 8,050, and 12,050 ms.
const ENTRY_BUSY_BACKOFFS: [Duration; ENTRY_BUSY_ATTEMPTS - 1] = [
    Duration::from_millis(100),
    Duration::from_millis(175),
    Duration::from_millis(275),
    Duration::from_millis(500),
    Duration::from_millis(1_000),
    Duration::from_millis(2_000),
    Duration::from_millis(4_000),
    Duration::from_millis(4_000),
];

/// Retries only a definite [`ControlError::Busy`] from selected entry-runtime routes.
///
/// The daemon emits `busy` only before borrowing the entry runtime, so arm, abandon, resume
/// preview/CAS, and process-handoff cancellation can repeat that definite pre-operation
/// refusal. `Unreachable`, timeout, malformed, conflict, and internal responses are returned
/// once. Each wait uses the shared 20 ms interruption slices, and a retry is made only where
/// its wait and one full ordinary transport window fit [`ENTRY_BUSY_BUDGET`].
pub fn retry_busy_entry<T>(
    call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let started = Instant::now();
    retry_busy_entry_with(|| started.elapsed(), dephased_backoff, call)
}

/// Testable clock and wait seam for [`retry_busy_entry`].
fn retry_busy_entry_with<T>(
    mut elapsed: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration) -> bool,
    mut call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let mut said = String::new();
    for attempt in 0..ENTRY_BUSY_ATTEMPTS {
        match call() {
            Err(ControlError::Busy(message)) => {
                said = message;
                let Some(delay) = ENTRY_BUSY_BACKOFFS.get(attempt) else {
                    break;
                };
                if elapsed()
                    .saturating_add(*delay)
                    .saturating_add(crate::daemon::CALL_TIMEOUT)
                    > ENTRY_BUSY_BUDGET
                {
                    break;
                }
                if !wait(*delay) {
                    break;
                }
                if elapsed().saturating_add(crate::daemon::CALL_TIMEOUT) > ENTRY_BUSY_BUDGET {
                    break;
                }
            }
            answered => return answered,
        }
    }
    Err(ControlError::Busy(said))
}

/// How many times `/process` is asked before its final `busy` is reported.
///
/// This is intentionally independent from [`BUSY_ATTEMPTS`]. The handoff is a fixed,
/// immutable request made immediately after a child was launched, so it gets nine bounded
/// tries spanning one ordinary upload hold rather than changing every other control route.
pub const PROCESS_BUSY_ATTEMPTS: usize = 9;

/// The maximum nominal sleep after the first `/process` `busy`.
///
/// The fixed waits total 12,050 ms. This is the planned sleep, not the whole retry envelope:
/// real scheduling around the interruptible slices has a separate bounded allowance below.
pub const PROCESS_BUSY_MAX_SLEEP: Duration = Duration::from_millis(12_050);

/// Explicit allowance for ordinary scheduler delay around the `/process` waits.
///
/// The schedule remains fixed and deterministic. This 950 ms is reservation headroom, never
/// another retry delay.
pub const PROCESS_BUSY_SCHEDULER_HEADROOM: Duration = Duration::from_millis(950);

/// The `/process` retry envelope: planned sleeps plus bounded scheduler headroom.
pub const PROCESS_BUSY_RETRY_ENVELOPE: Duration = Duration::from_secs(13);

/// The wall-clock ceiling for a real `/process` handoff, start to final refusal.
///
/// Nine [`crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT`] windows plus the 13-second
/// [`PROCESS_BUSY_RETRY_ENVELOPE`] is 31 seconds. The envelope includes the fixed
/// [`PROCESS_BUSY_MAX_SLEEP`] and [`PROCESS_BUSY_SCHEDULER_HEADROOM`]. These equalities are
/// pinned in deterministic tests below so a route deadline or schedule change also makes the
/// participant-visible bound an intentional choice.
pub const PROCESS_BUSY_BUDGET: Duration = Duration::new(
    crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT.as_secs() * PROCESS_BUSY_ATTEMPTS as u64
        + PROCESS_BUSY_RETRY_ENVELOPE.as_secs(),
    PROCESS_BUSY_RETRY_ENVELOPE.subsec_nanos(),
);

/// The waits for the `/process` handoff route are the entry-runtime waits: offsets 0, 100,
/// 275, 550, 1,050, 2,050, 4,050, 8,050, and 12,050 ms, so one daemon upload may occupy the
/// entry for its ordinary ten-second network timeout and the immutable handoff still gets an
/// idle-window attempt without randomized behavior.
const PROCESS_BUSY_BACKOFFS: [Duration; PROCESS_BUSY_ATTEMPTS - 1] = ENTRY_BUSY_BACKOFFS;

/// The terminal outcome of a process retry.  The distinction is security relevant: once
/// any attempt was transport/post-Prepare ambiguous, a later definite refusal cannot prove
/// the daemon did not accept the original immutable body.
#[derive(Debug)]
pub struct ProcessRetryFailure {
    pub error: ControlError,
    pub may_have_been_admitted: bool,
    /// The local interrupt flag ended a backoff before another request was made. This is
    /// distinct from spending the nine-attempt/budget envelope even though both preserve
    /// the last wire error and its admission classification.
    pub interrupted: bool,
}

/// Retries a pre-admission [`ControlError::Busy`], a post-Prepare
/// [`ControlError::ProcessAdmissionBusy`], or an ambiguous [`ControlError::Unreachable`]
/// from the `/process` handoff, and keeps the ambiguity history for the CLI lifecycle: a
/// caller must not kill or cancel a child that an earlier unreachable request may already
/// have handed to the daemon.
///
/// It makes at most [`PROCESS_BUSY_ATTEMPTS`] calls using the fixed waits in
/// [`PROCESS_BUSY_BACKOFFS`], whose total is [`PROCESS_BUSY_MAX_SLEEP`]. Every other result
/// is returned immediately, including a `conflict` that shares HTTP 409 with `busy`, and a
/// malformed or internal response.  `Unreachable` is the narrow exception because durable
/// `Prepared`/`Registered` makes this exact route request idempotent: it can mean the daemon
/// accepted the body and the `204` was lost. The closure owns no request construction:
/// callers capture one immutable
/// [`ProcessRegistration`](goalcup_protocol::process::ProcessRegistration) and submit that
/// same document on each attempt. Before it takes a wait for another attempt, the helper
/// reserves both that wait and the next route's named transport window inside
/// [`PROCESS_BUSY_BUDGET`]; a spent budget surfaces the last retryable refusal without making
/// a further request.
pub fn retry_busy_process_classified<T>(
    call: impl FnMut() -> Result<T, ProcessRegistrationError>,
) -> Result<T, ProcessRetryFailure> {
    let started = Instant::now();
    retry_busy_process_classified_with(|| started.elapsed(), dephased_backoff, call)
}

fn retry_busy_process_classified_with<T>(
    mut elapsed: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration) -> bool,
    mut call: impl FnMut() -> Result<T, ProcessRegistrationError>,
) -> Result<T, ProcessRetryFailure> {
    let mut may_have_been_admitted = false;
    let mut last = None;
    let mut interrupted = false;
    for attempt in 0..PROCESS_BUSY_ATTEMPTS {
        match call() {
            Err(response) => {
                let response_may_have_been_admitted = response.may_have_been_admitted();
                let error = response.into_error();
                match error {
                    error @ (ControlError::Busy(_)
                    | ControlError::ProcessAdmissionBusy(_)
                    | ControlError::Unreachable(_)) => {
                        may_have_been_admitted |= response_may_have_been_admitted;
                        last = Some(error);
                        let Some(delay) = PROCESS_BUSY_BACKOFFS.get(attempt) else {
                            break;
                        };
                        if elapsed()
                            .saturating_add(*delay)
                            .saturating_add(crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT)
                            > PROCESS_BUSY_BUDGET
                        {
                            break;
                        }
                        if !wait(*delay) {
                            interrupted = true;
                            break;
                        }
                        if elapsed().saturating_add(crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT)
                            > PROCESS_BUSY_BUDGET
                        {
                            break;
                        }
                    }
                    error => {
                        return Err(ProcessRetryFailure {
                            may_have_been_admitted: may_have_been_admitted
                                || response_may_have_been_admitted,
                            error,
                            interrupted: false,
                        });
                    }
                }
            }
            Ok(value) => return Ok(value),
        }
    }
    Err(ProcessRetryFailure {
        error: last.expect("a bounded process retry always remembers its last refusal"),
        may_have_been_admitted,
        interrupted,
    })
}

/// How many times the slow `/seal` call is asked before its final `busy` is reported.
///
/// This is intentionally independent from both [`BUSY_ATTEMPTS`] and
/// [`PROCESS_BUSY_ATTEMPTS`]. A seal is a terminal, admitted slow call; its retry policy only
/// rides out the short `busy` windows before that call starts, and never asks again after an
/// uncertain transport or a permanent refusal.
pub const SEAL_BUSY_ATTEMPTS: usize = 4;

/// The maximum nominal sleep after the first `/seal` `busy`: 100 + 175 + 275 ms.
pub const SEAL_BUSY_MAX_SLEEP: Duration = Duration::from_millis(550);

/// Explicit allowance for ordinary scheduler delay around the dephased `/seal` waits.
///
/// The 550 ms schedule is still fixed — it is what escapes every capture-cadence phase.
/// Real wall clocks also include scheduling between timer slices, response handling, and the
/// checks in this helper. This 450 ms is reservation headroom, never another retry delay.
pub const SEAL_BUSY_SCHEDULER_HEADROOM: Duration = Duration::from_millis(450);

/// The `/seal` retry envelope: its planned sleeps plus bounded scheduler headroom.
///
/// This is deliberately one second, not a claim that the policy sleeps for one second. The
/// schedule remains [`SEAL_BUSY_MAX_SLEEP`]; the remaining 450 ms makes the fourth ordinary
/// command-path attempt reachable while still refusing an extraordinary late wake.
pub const SEAL_BUSY_RETRY_ENVELOPE: Duration = Duration::from_secs(1);

/// The wall-clock reservation for a real `/seal` retry, start to final refusal.
///
/// A seal that the daemon admits can legitimately use its entire
/// [`crate::daemon::SLOW_CALL_TIMEOUT`], so one full slow transport window and the
/// one-second [`SEAL_BUSY_RETRY_ENVELOPE`] are reserved together. The planned dephased sleeps
/// still total only [`SEAL_BUSY_MAX_SLEEP`]; `busy` itself is immediate, and a later `busy`
/// only earns an attempt if a full slow seal can still begin inside this 301-second envelope.
pub const SEAL_BUSY_BUDGET: Duration = Duration::new(
    crate::daemon::SLOW_CALL_TIMEOUT.as_secs() + SEAL_BUSY_RETRY_ENVELOPE.as_secs(),
    SEAL_BUSY_RETRY_ENVELOPE.subsec_nanos(),
);

/// Deliberately non-periodic waits for the one `/seal` route.
///
/// These are the same fixed offsets as `/process`: 0, 100, 275, and 550 ms escape every
/// phase of the accelerated 200 ms capture cadence's 120 ms hold, without introducing
/// randomness into a participant's reproduction of a refusal.
const SEAL_BUSY_BACKOFFS: [Duration; SEAL_BUSY_ATTEMPTS - 1] = [
    Duration::from_millis(100),
    Duration::from_millis(175),
    Duration::from_millis(275),
];

/// Retries only a transient [`ControlError::Busy`] from the terminal `/seal` call.
///
/// It makes at most [`SEAL_BUSY_ATTEMPTS`] calls on the fixed schedule in
/// [`SEAL_BUSY_BACKOFFS`]. Every non-`Busy` answer returns immediately. Before and after each
/// interruptible wait, the policy reserves a full [`crate::daemon::SLOW_CALL_TIMEOUT`] inside
/// [`SEAL_BUSY_BUDGET`]. The 550 ms planned schedule has 450 ms explicit scheduler headroom;
/// a wake beyond that envelope surfaces the daemon's last refusal instead of beginning another
/// five-minute route outside its stated reservation.
pub fn retry_busy_seal<T>(
    call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let started = Instant::now();
    retry_busy_seal_with(|| started.elapsed(), dephased_backoff, call)
}

/// The testable core of [`retry_busy_seal`]. `elapsed` is the seal's wall clock; `wait`
/// returns false only when the local process has been interrupted, which surfaces the last
/// daemon `busy` rather than making another terminal request.
fn retry_busy_seal_with<T>(
    mut elapsed: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration) -> bool,
    mut call: impl FnMut() -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let mut said = String::new();
    for attempt in 0..SEAL_BUSY_ATTEMPTS {
        match call() {
            Err(ControlError::Busy(message)) => {
                said = message;
                let Some(delay) = SEAL_BUSY_BACKOFFS.get(attempt) else {
                    break;
                };
                // The next terminal request must have room for both its wait and the full
                // admitted seal window. Equality permits the fourth offset at the exact
                // boundary; a greater sum would cross the stated reservation.
                if elapsed()
                    .saturating_add(*delay)
                    .saturating_add(crate::daemon::SLOW_CALL_TIMEOUT)
                    > SEAL_BUSY_BUDGET
                {
                    break;
                }
                if !wait(*delay) {
                    break;
                }
                // `thread::sleep` can wake late. Recheck after it so a scheduler delay
                // cannot turn an equality admission into a slow route outside the budget.
                if elapsed().saturating_add(crate::daemon::SLOW_CALL_TIMEOUT) > SEAL_BUSY_BUDGET {
                    break;
                }
            }
            answered => return answered,
        }
    }
    Err(ControlError::Busy(said))
}

/// The capture seal's retry (CONTRACTS.md, H-10 4a ¶7): [`retry_busy_seal`]'s dephased schedule,
/// but against the one local-finish budget instead of the legacy 301-second reservation.
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
mod entry_retry_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn an_entry_call_retries_busy_on_the_exact_non_periodic_schedule() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = retry_busy_entry_with(
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
                    Ok("armed")
                }
            },
        );

        assert!(matches!(result, Ok("armed")));
        assert_eq!(attempts, 9);
        assert_eq!(
            waits,
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
        let planned_wait = waits.into_iter().sum::<Duration>();
        assert_eq!(planned_wait, Duration::from_millis(12_050));
        assert_eq!(ENTRY_BUSY_MAX_SLEEP, planned_wait);
        assert_eq!(ENTRY_BUSY_SCHEDULER_HEADROOM, Duration::from_millis(950));
        assert_eq!(
            ENTRY_BUSY_RETRY_ENVELOPE,
            planned_wait + ENTRY_BUSY_SCHEDULER_HEADROOM
        );
        assert_eq!(
            ENTRY_BUSY_BUDGET,
            crate::daemon::CALL_TIMEOUT * 2 + ENTRY_BUSY_RETRY_ENVELOPE
        );
        assert_eq!(ENTRY_BUSY_BUDGET, Duration::from_secs(73));
    }

    #[test]
    fn an_entry_call_outlasts_one_ten_second_upload_hold() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut attempts = Vec::new();

        let result = retry_busy_entry_with(
            || elapsed.get(),
            |delay| {
                elapsed.set(elapsed.get() + delay);
                true
            },
            || {
                attempts.push(elapsed.get());
                if elapsed.get() < Duration::from_secs(10) {
                    Err(ControlError::Busy(
                        "the upload still holds the entry".into(),
                    ))
                } else {
                    Ok("armed")
                }
            },
        );

        assert!(matches!(result, Ok("armed")));
        assert_eq!(attempts.len(), 9);
        assert_eq!(attempts.last(), Some(&Duration::from_millis(12_050)));
    }

    #[test]
    fn an_entry_call_surfaces_the_ninth_busy_without_a_tenth_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_entry_with(
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
        assert_eq!(attempts, 9);
        assert_eq!(waits.len(), 8);
    }

    #[test]
    fn entry_attempts_escape_every_phase_of_a_200ms_120ms_hold() {
        for start_ms in 0..120_u64 {
            let elapsed_ms = Cell::new(0_u64);
            let attempts = Cell::new(0_usize);

            let result: Result<(), ControlError> = retry_busy_entry_with(
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
    fn an_entry_call_does_not_wait_without_room_for_the_next_transport_window() {
        let elapsed = Cell::new(
            Duration::from_secs(73) - Duration::from_millis(100) - crate::daemon::CALL_TIMEOUT
                + Duration::from_millis(1),
        );
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_entry_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Busy(
                    "the upload still holds the entry".into(),
                ))
            },
        );

        assert!(matches!(result, Err(ControlError::Busy(_))));
        assert_eq!(attempts, 1);
        assert!(waits.is_empty());
    }

    #[test]
    fn an_entry_call_rechecks_its_budget_after_an_overslept_wait() {
        let elapsed = Cell::new(
            Duration::from_secs(73) - Duration::from_millis(100) - crate::daemon::CALL_TIMEOUT,
        );
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_entry_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                elapsed.set(elapsed.get() + delay + Duration::from_millis(1));
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Busy(
                    "the upload still holds the entry".into(),
                ))
            },
        );

        assert!(matches!(result, Err(ControlError::Busy(_))));
        assert_eq!(attempts, 1);
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn an_interrupted_entry_wait_surfaces_busy_without_another_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_entry_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                false
            },
            || {
                attempts += 1;
                Err(ControlError::Busy(
                    "the upload still holds the entry".into(),
                ))
            },
        );

        assert!(matches!(result, Err(ControlError::Busy(_))));
        assert_eq!(attempts, 1);
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn every_non_busy_entry_result_is_one_shot() {
        for error in [
            ControlError::Unreachable("daemon absent".into()),
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
            ControlError::ProcessAdmissionUncertain("not an arm result".into()),
        ] {
            let expected = error.code();
            let mut error = Some(error);
            let mut attempts = 0;
            let mut waits = Vec::new();
            let result: Result<(), ControlError> = retry_busy_entry_with(
                || Duration::ZERO,
                |delay| {
                    waits.push(delay);
                    true
                },
                || {
                    attempts += 1;
                    Err(error.take().expect("the arm result is one-shot"))
                },
            );

            assert_eq!(result.expect_err("the refusal surfaces").code(), expected);
            assert_eq!(attempts, 1, "{expected} was retried");
            assert!(waits.is_empty(), "{expected} scheduled a wait");
        }
    }
}

#[cfg(test)]
mod process_handoff_tests {
    use super::*;
    use std::cell::Cell;

    /// A refusal as a client that could not classify it reports it.
    fn refused(error: ControlError) -> ProcessRegistrationError {
        ProcessRegistrationError::from_unclassified(error)
    }

    #[test]
    fn a_process_handoff_retries_busy_on_the_dephased_schedule() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = retry_busy_process_classified_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                elapsed.set(elapsed.get() + delay);
                true
            },
            || {
                attempts += 1;
                if attempts < 9 {
                    Err(refused(ControlError::Busy(format!("pump {attempts}"))))
                } else {
                    Ok("registered")
                }
            },
        );

        assert!(matches!(result, Ok("registered")));
        assert_eq!(attempts, 9);
        assert_eq!(
            waits,
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
        let planned_wait = waits.iter().copied().sum::<Duration>();
        assert_eq!(
            planned_wait,
            Duration::from_millis(12_050),
            "the exact non-periodic schedule spans one ordinary 10 s upload hold"
        );
        assert_eq!(
            PROCESS_BUSY_MAX_SLEEP, planned_wait,
            "the nominal wait cap is the sum of the eight fixed waits"
        );
        assert_eq!(
            crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
            Duration::from_secs(2),
            "the handoff leaves the daemon time for durable registry and trace writes"
        );
        assert_eq!(
            PROCESS_BUSY_SCHEDULER_HEADROOM,
            Duration::from_millis(950),
            "the retry envelope reserves 950 ms of scheduler headroom"
        );
        assert_eq!(
            PROCESS_BUSY_RETRY_ENVELOPE,
            planned_wait + PROCESS_BUSY_SCHEDULER_HEADROOM,
            "the retry envelope is exactly planned sleep plus scheduler headroom"
        );
        assert_eq!(
            PROCESS_BUSY_BUDGET,
            crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT * 9 + PROCESS_BUSY_RETRY_ENVELOPE,
            "nine 2 s route windows and the 13 s retry envelope are the strict ceiling"
        );
        assert_eq!(
            PROCESS_BUSY_BUDGET,
            Duration::from_secs(31),
            "the participant-visible process handoff ceiling is 31 seconds"
        );
    }

    #[test]
    fn a_process_handoff_outlasts_one_ten_second_upload_hold() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut attempts = Vec::new();

        let result = retry_busy_process_classified_with(
            || elapsed.get(),
            |delay| {
                elapsed.set(elapsed.get() + delay);
                true
            },
            || {
                attempts.push(elapsed.get());
                if elapsed.get() < Duration::from_secs(10) {
                    Err(refused(ControlError::Busy(
                        "the upload still holds the entry".into(),
                    )))
                } else {
                    Ok("registered")
                }
            },
        );

        assert!(matches!(result, Ok("registered")));
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
            "the ninth ask is the first one after a 10 s upload hold"
        );
    }

    #[test]
    fn a_process_handoff_budget_exhausted_before_a_retry_surfaces_busy_without_waiting() {
        let elapsed = Cell::new(PROCESS_BUSY_BUDGET - Duration::from_millis(99));
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Busy(
                    "the handoff is still held".into(),
                )))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Busy(message), .. }) if message == "the handoff is still held")
        );
        assert_eq!(attempts, 1, "a wait past the handoff budget makes no retry");
        assert!(
            waits.is_empty(),
            "the budget is checked before the first wait"
        );
    }

    #[test]
    fn a_process_handoff_does_not_start_a_route_that_cannot_fit_its_budget() {
        let elapsed = Cell::new(
            PROCESS_BUSY_BUDGET
                - Duration::from_millis(100)
                - crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT
                + Duration::from_millis(1),
        );
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Busy(
                    "the handoff is still held".into(),
                )))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Busy(message), .. }) if message == "the handoff is still held")
        );
        assert_eq!(
            attempts, 1,
            "a remaining sleep is insufficient when the next route cannot also fit"
        );
        assert!(
            waits.is_empty(),
            "no retry begins without its transport window"
        );
    }

    #[test]
    fn a_process_handoff_rechecks_the_budget_after_an_overslept_wait() {
        let elapsed = Cell::new(
            PROCESS_BUSY_BUDGET
                - Duration::from_millis(100)
                - crate::daemon::PROCESS_HANDOFF_CALL_TIMEOUT,
        );
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                elapsed.set(elapsed.get() + delay + Duration::from_millis(1));
                true
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Busy(
                    "the handoff is still held".into(),
                )))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Busy(message), .. }) if message == "the handoff is still held")
        );
        assert_eq!(attempts, 1, "an overslept wait leaves no transport budget");
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn a_process_handoff_surfaces_the_ninth_busy_without_a_tenth_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Busy(format!("pump {attempts}"))))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Busy(message), .. }) if message == "pump 9")
        );
        assert_eq!(attempts, 9);
        assert_eq!(waits.len(), 8);
    }

    #[test]
    fn a_process_handoff_retries_an_ambiguous_transport_result_on_the_same_schedule() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = retry_busy_process_classified_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                match attempts {
                    1 => Err(refused(ControlError::Unreachable(
                        "the accepted response was lost".to_string(),
                    ))),
                    2 => Ok("registered"),
                    _ => unreachable!("the exact retry succeeded before a third call"),
                }
            },
        );

        assert!(matches!(result, Ok("registered")));
        assert_eq!(attempts, 2, "one ambiguous reply earns one exact replay");
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn a_later_conflict_cannot_disprove_an_earlier_ambiguous_process_response() {
        let mut attempts = 0;
        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified(|| {
            attempts += 1;
            match attempts {
                1 => Err(ProcessRegistrationError::from_unclassified(
                    ControlError::Unreachable("accepted response lost".into()),
                )),
                2 => Err(ProcessRegistrationError::from_unclassified(
                    ControlError::Conflict("a handle now stands".into()),
                )),
                _ => unreachable!("conflict is not retried"),
            }
        });
        let failure = result.expect_err("the later refusal is surfaced with its admission history");

        assert!(
            failure.may_have_been_admitted,
            "the first ambiguous response means the immutable body may already be durable"
        );
        assert!(matches!(failure.error, ControlError::Conflict(_)));
    }

    #[test]
    fn malformed_and_internal_process_replies_are_possibly_admitted_without_a_retry() {
        for code in ["malformed", "internal"] {
            let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified(|| {
                Err(ProcessRegistrationError::from_unclassified(match code {
                    "malformed" => {
                        ControlError::Malformed("duplicate control version header".into())
                    }
                    "internal" => ControlError::Internal("ambiguous daemon response".into()),
                    _ => unreachable!("the table names every process-response ambiguity"),
                }))
            });
            let failure = result.expect_err("the non-retryable response is surfaced");
            assert!(failure.may_have_been_admitted, "{}", failure.error.code());
        }
    }

    #[test]
    fn process_busy_classification_preserves_the_prepare_boundary() {
        let definite: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || Duration::ZERO,
            |_| true,
            || {
                Err(ProcessRegistrationError::from_unclassified(
                    ControlError::Busy("the entry is held before admission".into()),
                ))
            },
        );
        assert!(
            !definite.expect_err("busy exhausts").may_have_been_admitted,
            "ordinary busy proves every attempt was pre-mutation"
        );

        let prepared: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || Duration::ZERO,
            |_| true,
            || {
                Err(ProcessRegistrationError::from_unclassified(
                    ControlError::ProcessAdmissionBusy("Prepared is waiting".into()),
                ))
            },
        );
        assert!(
            prepared
                .expect_err("prepared busy exhausts")
                .may_have_been_admitted,
            "post-Prepare busy retains the immutable body and child"
        );
    }

    #[test]
    fn interrupted_process_busy_wait_keeps_the_same_admission_classification() {
        for (error, expected_ambiguous) in [
            (ControlError::Busy("held before admission".into()), false),
            (
                ControlError::ProcessAdmissionBusy("Prepared is waiting".into()),
                true,
            ),
        ] {
            let mut error = Some(error);
            let mut attempts = 0;
            let failure: ProcessRetryFailure = retry_busy_process_classified_with(
                || Duration::ZERO,
                |_| false,
                || {
                    attempts += 1;
                    Err::<(), _>(ProcessRegistrationError::from_unclassified(
                        error
                            .take()
                            .expect("the interrupt prevents a second attempt"),
                    ))
                },
            )
            .expect_err("the interrupted wait surfaces the last retryable result");
            assert_eq!(attempts, 1);
            assert_eq!(failure.may_have_been_admitted, expected_ambiguous);
            assert!(
                failure.interrupted,
                "the caller can distinguish Ctrl-C from exhaustion"
            );
        }
    }

    #[test]
    fn a_process_handoff_exhausts_ambiguous_transport_within_its_same_bound() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Unreachable(format!(
                    "daemon absent {attempts}"
                ))))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Unreachable(message), .. }) if message == "daemon absent 9")
        );
        assert_eq!(attempts, 9);
        assert_eq!(waits.len(), 8);
        assert_eq!(waits.into_iter().sum::<Duration>(), PROCESS_BUSY_MAX_SLEEP);
    }

    #[test]
    fn an_interrupted_process_handoff_wait_surfaces_busy_without_another_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                false
            },
            || {
                attempts += 1;
                Err(refused(ControlError::Busy(
                    "the upload still holds the entry".into(),
                )))
            },
        );

        assert!(
            matches!(result, Err(ProcessRetryFailure { error: ControlError::Busy(message), .. }) if message == "the upload still holds the entry")
        );
        assert_eq!(attempts, 1, "an interrupted wait makes no second request");
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn a_process_handoff_reports_permanent_refusals_once() {
        for error in [
            ControlError::Conflict("the handoff slot is occupied".into()),
            ControlError::ArmingTimeout {
                detail: "not a process handoff retry".into(),
            },
            ControlError::Malformed("an uncertain response".into()),
            ControlError::Internal("the daemon rejected the request".into()),
        ] {
            let expected_code = error.code();
            let mut error = Some(error);
            let mut waits = Vec::new();
            let mut attempts = 0;

            let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
                || Duration::ZERO,
                |delay| {
                    waits.push(delay);
                    true
                },
                || {
                    attempts += 1;
                    Err(refused(
                        error.take().expect("a non-Busy refusal is called once"),
                    ))
                },
            );

            assert_eq!(
                result
                    .expect_err("the non-Busy result surfaces")
                    .error
                    .code(),
                expected_code
            );
            assert_eq!(attempts, 1, "{expected_code} is not a transient Busy");
            assert!(waits.is_empty(), "{expected_code} schedules no retry");
        }
    }

    #[test]
    fn dephased_process_handoff_attempts_escape_every_phase_of_a_200ms_120ms_hold() {
        for start_ms in 0..120_u64 {
            let clock_ms = Cell::new(start_ms);
            let attempts = Cell::new(0_usize);

            let result: Result<(), ProcessRetryFailure> = retry_busy_process_classified_with(
                || Duration::from_millis(clock_ms.get()),
                |delay| {
                    clock_ms.set(clock_ms.get() + delay.as_millis() as u64);
                    true
                },
                || {
                    attempts.set(attempts.get() + 1);
                    if clock_ms.get() % 200 < 120 {
                        Err(refused(ControlError::Busy(
                            "the upload still holds the entry".into(),
                        )))
                    } else {
                        Ok(())
                    }
                },
            );

            assert!(
                result.is_ok(),
                "start offset {start_ms} ms stayed inside every simulated upload hold"
            );
            assert!(
                attempts.get() <= PROCESS_BUSY_ATTEMPTS,
                "start offset {start_ms} ms exceeded the bounded retry cap"
            );
            assert!(
                clock_ms.get() - start_ms <= PROCESS_BUSY_MAX_SLEEP.as_millis() as u64,
                "start offset {start_ms} ms exceeded the post-Busy sleep cap"
            );
        }
    }
}

#[cfg(test)]
mod seal_retry_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_seal_retries_busy_on_the_dephased_schedule() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = retry_busy_seal_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                if attempts < SEAL_BUSY_ATTEMPTS {
                    Err(ControlError::Busy(format!("pump {attempts}")))
                } else {
                    Ok("sealed")
                }
            },
        );

        assert!(matches!(result, Ok("sealed")));
        assert_eq!(attempts, SEAL_BUSY_ATTEMPTS);
        assert_eq!(
            waits,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(175),
                Duration::from_millis(275),
            ]
        );
        assert_eq!(
            waits.into_iter().sum::<Duration>(),
            SEAL_BUSY_MAX_SLEEP,
            "the complete seal retry window is bounded at 550 ms"
        );
        assert_eq!(
            SEAL_BUSY_BUDGET,
            crate::daemon::SLOW_CALL_TIMEOUT + SEAL_BUSY_RETRY_ENVELOPE,
            "the whole reservation is one admitted slow seal and its one-second retry envelope"
        );
        assert_eq!(
            SEAL_BUSY_RETRY_ENVELOPE,
            SEAL_BUSY_MAX_SLEEP + SEAL_BUSY_SCHEDULER_HEADROOM,
            "the one-second reservation leaves 450 ms beyond the 550 ms planned waits"
        );
    }

    #[test]
    fn a_seal_succeeds_after_one_busy() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result = retry_busy_seal_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(ControlError::Busy("the pump is mid-cycle".into()))
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_ok());
        assert_eq!(attempts, 2);
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn a_seal_allows_the_fourth_attempt_on_its_reservation_boundary() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
            || elapsed.get(),
            |delay| {
                elapsed.set(elapsed.get() + delay);
                true
            },
            || {
                attempts += 1;
                match attempts {
                    1 | 2 => Err(ControlError::Busy("the seal is still held".into())),
                    3 => {
                        // The third Busy will wait 275 ms. With 725 ms already elapsed,
                        // that wait and a whole slow call land exactly at the 301 s
                        // reservation boundary, so the fourth call is still admitted.
                        elapsed.set(
                            SEAL_BUSY_BUDGET
                                - Duration::from_millis(275)
                                - crate::daemon::SLOW_CALL_TIMEOUT,
                        );
                        Err(ControlError::Busy("the seal is still held".into()))
                    }
                    4 => Ok(()),
                    _ => unreachable!("the bounded helper never asks a fifth time"),
                }
            },
        );

        assert!(result.is_ok());
        assert_eq!(attempts, SEAL_BUSY_ATTEMPTS);
    }

    #[test]
    fn a_seal_surfaces_the_fourth_busy_without_a_fifth_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
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

        assert!(matches!(result, Err(ControlError::Busy(message)) if message == "pump 4"));
        assert_eq!(attempts, SEAL_BUSY_ATTEMPTS);
        assert_eq!(waits.len(), SEAL_BUSY_ATTEMPTS - 1);
    }

    #[test]
    fn a_seal_reports_every_non_busy_refusal_once() {
        for error in [
            ControlError::Conflict("already sealed".into()),
            ControlError::Unreachable("the daemon went away".into()),
            ControlError::ArmingTimeout {
                detail: "a timeout is not a Busy".into(),
            },
            ControlError::Malformed("an uncertain response".into()),
            ControlError::Internal("the daemon failed".into()),
        ] {
            let expected_code = error.code();
            let mut error = Some(error);
            let mut waits = Vec::new();
            let mut attempts = 0;

            let result: Result<(), ControlError> = retry_busy_seal_with(
                || Duration::ZERO,
                |delay| {
                    waits.push(delay);
                    true
                },
                || {
                    attempts += 1;
                    Err(error.take().expect("a non-Busy refusal is called once"))
                },
            );

            assert_eq!(
                result.expect_err("the non-Busy result surfaces").code(),
                expected_code
            );
            assert_eq!(attempts, 1, "{expected_code} is not a transient Busy");
            assert!(waits.is_empty(), "{expected_code} schedules no retry");
        }
    }

    #[test]
    fn a_seal_does_not_start_a_route_that_cannot_fit_its_budget() {
        let elapsed = Cell::new(
            SEAL_BUSY_BUDGET - Duration::from_millis(100) - crate::daemon::SLOW_CALL_TIMEOUT
                + Duration::from_millis(1),
        );
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Busy("the seal is still held".into()))
            },
        );

        assert!(
            matches!(result, Err(ControlError::Busy(message)) if message == "the seal is still held")
        );
        assert_eq!(
            attempts, 1,
            "the next slow call cannot cross the reservation"
        );
        assert!(
            waits.is_empty(),
            "no wait is taken without room for the next slow call"
        );
    }

    #[test]
    fn a_seal_reservation_absorbs_a_bounded_late_wake() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
            || elapsed.get(),
            |delay| {
                elapsed.set(elapsed.get() + delay + Duration::from_millis(500));
                true
            },
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(ControlError::Busy("the seal is still held".into()))
                } else {
                    Ok(())
                }
            },
        );

        assert!(
            result.is_ok(),
            "the one-second envelope admits this late wake"
        );
        assert_eq!(attempts, 2, "the next slow call still fits the reservation");
    }

    #[test]
    fn a_seal_rechecks_the_budget_after_an_overslept_wait() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
            || elapsed.get(),
            |delay| {
                waits.push(delay);
                elapsed.set(SEAL_BUSY_RETRY_ENVELOPE + Duration::from_millis(1));
                true
            },
            || {
                attempts += 1;
                Err(ControlError::Busy("the seal is still held".into()))
            },
        );

        assert!(
            matches!(result, Err(ControlError::Busy(message)) if message == "the seal is still held")
        );
        assert_eq!(
            attempts, 1,
            "an overslept wait leaves no slow-call reservation"
        );
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn an_interrupted_seal_wait_surfaces_busy_without_another_attempt() {
        let mut waits = Vec::new();
        let mut attempts = 0;

        let result: Result<(), ControlError> = retry_busy_seal_with(
            || Duration::ZERO,
            |delay| {
                waits.push(delay);
                false
            },
            || {
                attempts += 1;
                Err(ControlError::Busy("the seal is still held".into()))
            },
        );

        assert!(
            matches!(result, Err(ControlError::Busy(message)) if message == "the seal is still held")
        );
        assert_eq!(
            attempts, 1,
            "an interrupted wait does not make another seal request"
        );
        assert_eq!(waits, vec![Duration::from_millis(100)]);
    }

    #[test]
    fn dephased_seal_attempts_escape_every_phase_of_a_200ms_120ms_hold() {
        for start_ms in 0..120_u64 {
            let elapsed_ms = Cell::new(0_u64);
            let attempts = Cell::new(0_usize);

            let result: Result<(), ControlError> = retry_busy_seal_with(
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

            assert!(
                result.is_ok(),
                "start offset {start_ms} ms stayed inside every simulated upload hold"
            );
            assert!(
                attempts.get() <= SEAL_BUSY_ATTEMPTS,
                "start offset {start_ms} ms exceeded the bounded retry cap"
            );
            assert!(
                elapsed_ms.get() <= SEAL_BUSY_MAX_SLEEP.as_millis() as u64,
                "start offset {start_ms} ms exceeded the post-Busy sleep cap"
            );
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
