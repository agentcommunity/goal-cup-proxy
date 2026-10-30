//! When the uploader sends and how a failure waits (PRD-06 sections 4, 5 and 9).
//!
//! Pure rules: every function takes `now` (boot time, `Recorder::now()`) and never reads a clock
//! itself, so a wall clock set back or forward never moves a timer, and a suspend counts. These are
//! waits with retries, never limits: no answer ends a run's upload. Only `goalcup upload` by hand
//! gives up ([`ends_manual_upload`]). HTTP timeouts belong to the client, not here.

use crate::pipeline::Mono;
use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// The uploader's timings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// A round every this long, counted from the start of the last round.
    pub round: Duration,
    /// A round at once when this many compressed bytes wait past the server's offset.
    pub trigger_bytes: u64,
    /// The first wait after a failure; it doubles per failure in a row.
    pub backoff_base: Duration,
    /// No wait is longer, `Retry-After` included.
    pub backoff_cap: Duration,
    /// A heartbeat every this long (and at once when the recording state changes).
    pub beat: Duration,
    /// A `stuck` entry reads the server's position again this often.
    pub stuck_reread: Duration,
    /// `refused` is said once after this many refusals in a row.
    pub refused_after: u32,
    /// How long the recorder keeps sending after the seal.
    pub wrap_up: Duration,
    /// Whether the first interval after the opening send is a random share of `round`, so runs
    /// started together do not send together.
    pub spread_first: bool,
}

/// The jitter factor's range: every wait is its nominal value times a draw from it.
pub const JITTER: (f64, f64) = (0.5, 1.0);

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            round: Duration::from_secs(5 * 60),
            trigger_bytes: 16 << 20,
            backoff_base: Duration::from_secs(2),
            backoff_cap: Duration::from_secs(120),
            beat: Duration::from_secs(5 * 60),
            stuck_reread: Duration::from_secs(2 * 60),
            refused_after: 3,
            wrap_up: Duration::from_secs(60 * 60),
            spread_first: true,
        }
    }
}

impl Policy {
    /// The policy of this build. A `test-hooks` build reads `GOALCUP_TEST_UPLOAD_POLICY`:
    /// `round_ms,trigger_bytes,backoff_ms,beat_ms`, where the cap is 60 times the backoff base,
    /// as 120 s is to 2 s, and the first interval is not spread. A malformed value is ignored.
    pub fn from_env() -> Policy {
        #[cfg(feature = "test-hooks")]
        if let Some(p) = test_policy() {
            return p;
        }
        Policy::default()
    }
}

#[cfg(feature = "test-hooks")]
fn test_policy() -> Option<Policy> {
    let spec = std::env::var("GOALCUP_TEST_UPLOAD_POLICY").ok()?;
    let n: Vec<u64> = spec
        .split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [round, trigger, backoff, beat] = n[..] else {
        return None;
    };
    let ms = Duration::from_millis;
    Some(Policy {
        round: ms(round),
        trigger_bytes: trigger,
        backoff_base: ms(backoff),
        backoff_cap: ms(backoff.saturating_mul(60)),
        beat: ms(beat),
        spread_first: false,
        ..Policy::default()
    })
}

/// `now` plus `d`, on boot time.
pub fn after(now: Mono, d: Duration) -> Mono {
    Mono(now.0.saturating_add(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)))
}

/// The wait after `failures` failures in a row:
/// `max(min(cap, base * 2^(failures - 1)) * jitter, min(retry_after, cap))`.
pub fn backoff(p: &Policy, failures: u32, retry_after: Option<Duration>, jitter: f64) -> Duration {
    let exp = failures.saturating_sub(1).min(31);
    let nominal = p.backoff_base.saturating_mul(1 << exp).min(p.backoff_cap);
    let asked = retry_after.unwrap_or_default().min(p.backoff_cap);
    nominal.mul_f64(jitter.clamp(JITTER.0, JITTER.1)).max(asked)
}

/// A uniform draw in `[0, 1)` from std's random hasher keys, fresh on every call (no rand crate).
fn draw() -> f64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u8(0);
    (h.finish() >> 11) as f64 / (1u64 << 53) as f64
}

/// A jitter factor in [`JITTER`].
pub fn jitter() -> f64 {
    JITTER.0 + (JITTER.1 - JITTER.0) * draw()
}

/// Failures in a row of one kind of call, and when to try again.
#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    policy: Policy,
    pub failures: u32,
    pub next_try: Option<Mono>,
}

impl Backoff {
    pub fn new(p: &Policy) -> Backoff {
        Backoff { policy: *p, failures: 0, next_try: None }
    }

    /// One more failure at `now`; returns the wait.
    pub fn fail(&mut self, now: Mono, retry_after: Option<Duration>) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = backoff(&self.policy, self.failures, retry_after, jitter());
        self.next_try = Some(after(now, wait));
        wait
    }

    /// The server's offset moved forward, or the entry is caught up: back to no failures, so a
    /// flaky network that still makes progress never escalates (run 1's lag bug).
    pub fn progress(&mut self) {
        self.failures = 0;
        self.next_try = None;
    }

    pub fn due(&self, now: Mono) -> bool {
        self.next_try.is_none_or(|t| now >= t)
    }
}

/// A repeating timer on boot time: due at once at first, then the first interval is a random
/// share of `period` (when spread), then every `period` from the last start.
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    period: Duration,
    spread_first: bool,
    next: Option<Mono>,
}

impl Timer {
    /// The heartbeat's timer.
    pub fn beat(p: &Policy) -> Timer {
        Timer { period: p.beat, spread_first: p.spread_first, next: None }
    }

    pub fn due(&self, now: Mono) -> bool {
        self.next.is_none_or(|t| now >= t)
    }

    /// The timed work began at `now`.
    pub fn started(&mut self, now: Mono) {
        let wait = match self.next {
            None if self.spread_first => self.period.mul_f64(draw()),
            _ => self.period,
        };
        self.next = Some(after(now, wait));
    }
}

/// When a round is due: the timer of section 4, or at once when the size trigger is reached.
#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    timer: Timer,
    trigger_bytes: u64,
}

impl Cadence {
    pub fn new(p: &Policy) -> Cadence {
        Cadence {
            timer: Timer { period: p.round, spread_first: p.spread_first, next: None },
            trigger_bytes: p.trigger_bytes,
        }
    }

    /// Whether a round is due. A round with nothing waiting sends nothing.
    pub fn due(&self, now: Mono, waiting_bytes: u64) -> bool {
        waiting_bytes >= self.trigger_bytes || self.timer.due(now)
    }

    /// A round began at `now`.
    pub fn started(&mut self, now: Mono) {
        self.timer.started(now);
    }
}

/// The wrap-up after the seal: sending goes on until `until`, on boot time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapUp {
    pub until: Mono,
}

impl WrapUp {
    pub fn starting(now: Mono, p: &Policy) -> WrapUp {
        WrapUp { until: after(now, p.wrap_up) }
    }

    pub fn over(&self, now: Mono) -> bool {
        now >= self.until
    }
}

/// Why a call failed, as `upload.json` `last_error.class` says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Network,
    ServerBusy,
    Refused,
    Stuck,
    Internal,
}

/// The class of an HTTP status. None for a success and for 409, which reads the position again.
/// An answer that is neither the server's success nor its error (a portal's redirect) is
/// `network`: the call did not reach the server.
pub fn class_of_status(status: u16) -> Option<Class> {
    match status {
        200..=299 | 409 => None,
        429 | 500..=599 => Some(Class::ServerBusy),
        400..=499 => Some(Class::Refused),
        _ => Some(Class::Network),
    }
}

/// Whether `goalcup upload`, with a person waiting, gives up on this status and says why.
pub fn ends_manual_upload(status: u16) -> bool {
    matches!(status, 401 | 404 | 410)
}
