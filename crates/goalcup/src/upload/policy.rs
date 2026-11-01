//! When the uploader sends and how a failure waits (PRD-06 sections 4, 5 and 9). Pure rules on
//! boot time: every function takes `now` and reads no clock. Waits with retries, never limits.

use crate::pipeline::Mono;
use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// The uploader's timings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// A round every this long, from the start of the last one.
    pub round: Duration,
    /// A round at once when this many compressed bytes wait.
    pub trigger_bytes: u64,
    /// The first wait after a failure; it doubles per failure in a row.
    pub backoff_base: Duration,
    /// No wait is longer, `Retry-After` included.
    pub backoff_cap: Duration,
    /// A heartbeat every this long, and at once on a recording state change.
    pub beat: Duration,
    /// A `stuck` entry reads the position again this often.
    pub stuck_reread: Duration,
    /// `refused` is said after this many refusals in a row.
    pub refused_after: u32,
    /// How long sending goes on after the seal.
    pub wrap_up: Duration,
    /// The first interval is a random share of the period, so runs started together spread.
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
    /// The default; a `test-hooks` build reads `GOALCUP_TEST_UPLOAD_POLICY` as
    /// `round_ms,trigger_bytes,backoff_ms,beat_ms` (cap 60 times the base, no spread).
    pub fn from_env() -> Policy {
        #[cfg(feature = "test-hooks")]
        if let Some([round, trigger, backoff, beat]) = std::env::var("GOALCUP_TEST_UPLOAD_POLICY")
            .ok()
            .and_then(|s| {
                s.split(',')
                    .map(|v| v.trim().parse().ok())
                    .collect::<Option<Vec<u64>>>()
            })
            .and_then(|n| <[u64; 4]>::try_from(n).ok())
        {
            let ms = Duration::from_millis;
            return Policy {
                round: ms(round),
                trigger_bytes: trigger,
                backoff_base: ms(backoff),
                backoff_cap: ms(backoff.saturating_mul(60)),
                beat: ms(beat),
                spread_first: false,
                ..Policy::default()
            };
        }
        Policy::default()
    }
}

/// `now` plus `d`.
pub fn after(now: Mono, d: Duration) -> Mono {
    let n = d.as_nanos().try_into().unwrap_or(u64::MAX);
    Mono(now.0.saturating_add(n))
}

/// The wait after `failures` failures in a row:
/// `max(min(cap, base * 2^(failures - 1)) * jitter, min(retry_after, cap))`.
pub fn backoff(p: &Policy, failures: u32, retry_after: Option<Duration>, jitter: f64) -> Duration {
    let exp = failures.saturating_sub(1).min(31);
    let nominal = p.backoff_base.saturating_mul(1 << exp).min(p.backoff_cap);
    let asked = retry_after.unwrap_or_default().min(p.backoff_cap);
    nominal.mul_f64(jitter.clamp(JITTER.0, JITTER.1)).max(asked)
}

/// A uniform draw in `[0, 1)` from std's random hasher keys (no rand crate).
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
        Backoff {
            policy: *p,
            failures: 0,
            next_try: None,
        }
    }

    /// One more failure at `now`; returns the wait.
    pub fn fail(&mut self, now: Mono, retry_after: Option<Duration>) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = backoff(&self.policy, self.failures, retry_after, jitter());
        self.next_try = Some(after(now, wait));
        wait
    }

    /// Progress resets the failures, so a flaky network that still moves never escalates.
    pub fn progress(&mut self) {
        self.failures = 0;
        self.next_try = None;
    }

    pub fn due(&self, now: Mono) -> bool {
        self.next_try.is_none_or(|t| now >= t)
    }
}

/// A repeating timer: due at once, then a first interval (spread when asked), then every period.
/// The beat's runs every `Policy::beat`; a round's every `Policy::round`, and its caller also
/// starts a round at once when `trigger_bytes` wait.
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    period: Duration,
    spread_first: bool,
    next: Option<Mono>,
}

impl Timer {
    pub fn new(period: Duration, spread_first: bool) -> Timer {
        Timer {
            period,
            spread_first,
            next: None,
        }
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

/// The wrap-up after the seal: sending goes on until `until`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapUp {
    pub until: Mono,
}

impl WrapUp {
    pub fn starting(now: Mono, p: &Policy) -> WrapUp {
        WrapUp {
            until: after(now, p.wrap_up),
        }
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

impl Class {
    /// The word `upload.json` and `goalcup.log` use.
    pub fn word(self) -> &'static str {
        match self {
            Class::Network => "network",
            Class::ServerBusy => "server_busy",
            Class::Refused => "refused",
            Class::Stuck => "stuck",
            Class::Internal => "internal",
        }
    }
}

/// The class of an HTTP status; None for a success and for 409 (read the position again). Only
/// the server's own refusals are `refused`; any other answer (a portal, a middlebox) is `network`.
pub fn class_of_status(status: u16) -> Option<Class> {
    match status {
        200..=299 | 409 => None,
        429 | 500..=599 => Some(Class::ServerBusy),
        400 | 401 | 404 | 410 | 413 | 422 => Some(Class::Refused),
        _ => Some(Class::Network),
    }
}

/// Whether `goalcup upload`, with a person waiting, gives up on this status.
pub fn ends_manual_upload(status: u16) -> bool {
    matches!(status, 401 | 404 | 410)
}
