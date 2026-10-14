//! The uploader's budgets: when chunks go, how a failure waits, which answers end an upload at
//! all, and how long the wrap-up after the stop lasts.
//!
//! These are budgets with retries, not limits. One failed attempt never ends a run's upload:
//! every answer but `401`, `404` and `410` ([`ServerError::ends_upload`]) is asked again after
//! [`backoff`], and the wrap-up ([`WrapUp`]) is the only thing that bounds the asking.

use super::http::ServerError;
use goalcup_protocol::clock::ContinuousOrigin;
use std::time::{Duration, Instant};

/// How long one chunk upload may take.
const PUT_TIMEOUT: Duration = Duration::from_secs(600);

/// How long a run with no upload server has to seal after its stop. The seal is the only thing
/// such a run wraps up, so this is the whole of its budget.
pub const LOCAL_WRAP_UP: Duration = Duration::from_secs(30 * 60);

/// When chunks go, and how failures wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// The regular cadence: a chunk goes this often, whatever the chain's size.
    pub interval: Duration,
    /// Whether the first interval after the opening chunk is a random share of
    /// [`Policy::interval`] (see [`Cadence`]), so that runs started together do not all send on
    /// the same half hour. A test build's policy turns it off to stay deterministic.
    pub spread_first: bool,
    /// How often the chain is looked at.
    pub poll: Duration,
    /// A chunk goes before its interval once this much chain is waiting (about 25 MB
    /// compressed).
    pub eager_bytes: u64,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// How long after the stop `goalcup start` keeps sealing and sending (the wrap-up). What is
    /// still unsent then is left to `goalcup upload`.
    pub wrap_up: Duration,
    /// The longest one chunk upload may take.
    pub put_timeout: Duration,
    /// How often a heartbeat goes while the run records.
    pub heartbeat: Duration,
}

impl Policy {
    /// A test build's policy from `spec`: `interval,poll,backoff_min,backoff_max,finish`
    /// milliseconds, `eager_bytes`, and optionally the chunk upload timeout and the heartbeat
    /// interval in milliseconds, comma separated. `finish` is the wrap-up. A test policy has
    /// no random first interval. A shipped build has no such door.
    #[cfg(feature = "test-support")]
    pub fn from_test_spec(spec: &str) -> Option<Policy> {
        let n: Vec<u64> = spec
            .split(',')
            .map(|v| v.trim().parse().ok())
            .collect::<Option<_>>()?;
        let (interval, poll, backoff_min, backoff_max, finish, eager, put, beat) = match n[..] {
            [a, b, c, d, e, f] => (a, b, c, d, e, f, 600_000, 300_000),
            [a, b, c, d, e, f, g] => (a, b, c, d, e, f, g, 300_000),
            [a, b, c, d, e, f, g, h] => (a, b, c, d, e, f, g, h),
            _ => return None,
        };
        let ms = Duration::from_millis;
        Some(Policy {
            interval: ms(interval),
            spread_first: false,
            poll: ms(poll),
            eager_bytes: eager,
            backoff_min: ms(backoff_min),
            backoff_max: ms(backoff_max),
            wrap_up: ms(finish),
            put_timeout: ms(put),
            heartbeat: ms(beat),
        })
    }
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            interval: Duration::from_secs(30 * 60),
            spread_first: true,
            poll: Duration::from_secs(30),
            eager_bytes: 256 * 1024 * 1024,
            backoff_min: Duration::from_secs(60),
            backoff_max: Duration::from_secs(15 * 60),
            wrap_up: Duration::from_secs(60 * 60),
            put_timeout: PUT_TIMEOUT,
            heartbeat: Duration::from_secs(300),
        }
    }
}

/// The environment variable through which a test build shortens the wrap-up of a run without an
/// upload server: `<wrap_up_ms>[,<first_seal_retry_ms>]`. A shipped build neither has nor reads
/// it.
#[cfg(feature = "test-support")]
pub const TEST_LOCAL_WRAP_UP: &str = "GOALCUP_TEST_LOCAL_WRAP_UP";

/// How a run's wrap-up is shaped: its length, and (in a test build only) the first wait of the
/// seal's retries. [`WrapUpSpec::begin`] starts the clock at the moment the run stops.
#[derive(Debug, Clone, Copy)]
pub struct WrapUpSpec {
    length: Duration,
    #[cfg(feature = "test-support")]
    first_retry: Option<Duration>,
}

impl WrapUpSpec {
    /// The shape for a run: the policy's for a run on a server, [`LOCAL_WRAP_UP`] for one
    /// without. A test build reads [`TEST_LOCAL_WRAP_UP`] from `env` and applies it to a run
    /// without a server only; a malformed value is ignored.
    pub fn for_run(
        policy: Option<&Policy>,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        #[cfg(not(feature = "test-support"))]
        let _ = env;
        #[allow(unused_mut)]
        let mut spec = WrapUpSpec {
            length: wrap_up_length(policy),
            #[cfg(feature = "test-support")]
            first_retry: None,
        };
        #[cfg(feature = "test-support")]
        if policy.is_none()
            && let Some(value) = env.get(TEST_LOCAL_WRAP_UP)
        {
            let parsed: Option<Vec<u64>> =
                value.split(',').map(|v| v.trim().parse().ok()).collect();
            let ms = Duration::from_millis;
            match parsed.as_deref() {
                Some(&[wrap]) => spec.length = ms(wrap),
                Some(&[wrap, first]) => {
                    spec.length = ms(wrap);
                    spec.first_retry = Some(ms(first));
                }
                _ => {}
            }
        }
        spec
    }

    /// The wrap-up, beginning now.
    pub fn begin(self) -> WrapUp {
        let wrap = WrapUp::after(self.length);
        #[cfg(feature = "test-support")]
        let wrap = WrapUp {
            first_retry: self.first_retry,
            ..wrap
        };
        wrap
    }
}

/// How long the wrap-up of a run lasts: the policy's for a run opened on a server, and
/// [`LOCAL_WRAP_UP`] for a run with none.
pub fn wrap_up_length(policy: Option<&Policy>) -> Duration {
    policy.map_or(LOCAL_WRAP_UP, |policy| policy.wrap_up)
}

/// When the regular sends are due.
///
/// The first chunk goes when the run launches. The first timer interval after it is a random
/// share of [`Policy::interval`], so that 1,500 runs started together do not all send on the
/// same half hour; every interval after it is the full one.
#[derive(Debug, Clone, Copy)]
pub struct Cadence {
    interval: Duration,
    spread_first: bool,
    sent: bool,
}

impl Cadence {
    pub fn new(policy: &Policy) -> Cadence {
        Cadence {
            interval: policy.interval,
            spread_first: policy.spread_first,
            sent: false,
        }
    }

    /// When the next regular send is due, after one that was caught up at `now`.
    pub fn next_due(&mut self, now: Instant) -> Instant {
        use rand::Rng;
        self.next_due_with(now, rand::thread_rng().gen_range(0.0..1.0))
    }

    /// [`Cadence::next_due`] with the random draw (in `[0, 1)`) named, for a test.
    fn next_due_with(&mut self, now: Instant, draw: f64) -> Instant {
        let wait = if !self.sent && self.spread_first {
            self.interval.mul_f64(draw)
        } else {
            self.interval
        };
        self.sent = true;
        now + wait
    }
}

/// The wrap-up of one run: sealing and sending after the stop, until `length` has passed.
///
/// Measured on a clock that keeps counting while the machine sleeps
/// ([`ContinuousOrigin`]): a lid closed during the wrap-up uses it up, as it uses up the run's
/// own hours. `Instant` on macOS stops during sleep, so a bound held as an `Instant` would
/// outlast the hour by every second the machine slept.
#[derive(Debug, Clone, Copy)]
pub struct WrapUp {
    began: ContinuousOrigin,
    length: Duration,
    /// A test build's first seal-retry wait, in place of [`crate::retry::SEAL_RETRY_FIRST`].
    #[cfg(feature = "test-support")]
    first_retry: Option<Duration>,
}

impl WrapUp {
    /// A wrap-up of `length` that begins now: call it at the moment the run stops.
    pub fn after(length: Duration) -> WrapUp {
        WrapUp {
            began: ContinuousOrigin::now(),
            length,
            #[cfg(feature = "test-support")]
            first_retry: None,
        }
    }

    /// The first wait before the seal is asked again: [`crate::retry::SEAL_RETRY_FIRST`],
    /// except in a test build told otherwise ([`TEST_LOCAL_WRAP_UP`]).
    pub fn first_seal_retry(&self) -> Duration {
        #[cfg(feature = "test-support")]
        if let Some(first) = self.first_retry {
            return first;
        }
        crate::retry::SEAL_RETRY_FIRST
    }

    /// A bound that is already spent: one attempt is made and none is repeated.
    pub fn one_attempt() -> WrapUp {
        WrapUp::after(Duration::ZERO)
    }

    /// What is left of the wrap-up, sleep included in what has passed.
    pub fn remaining(&self) -> Duration {
        self.length
            .saturating_sub(Duration::from_nanos(self.began.elapsed_ns()))
    }
}

impl ServerError {
    /// Whether this answer ends the upload of the run: the token is not accepted (`401`), or
    /// the run is not there (`404`) or is closed (`410`). Nothing asking again can change
    /// that. Every other answer, a `400` or `413` that rejected a chunk included, is asked
    /// again after [`backoff`], for as long as the wrap-up (or the run) lasts: the server, the
    /// network or a rule being deployed may be different a minute from now.
    pub fn ends_upload(&self) -> bool {
        matches!(
            self,
            ServerError::Status {
                status: 401 | 404 | 410,
                ..
            }
        )
    }
}

/// The wait before asking again: from `min`, doubling per consecutive failure to `max`, plus
/// up to a quarter more at random so a fleet of tracers does not ask in step, and never
/// less than what the server asked for.
pub fn backoff(policy: &Policy, failures: u32, retry_after: Option<Duration>) -> Duration {
    use rand::Rng;
    let doubled = policy
        .backoff_min
        .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
        .min(policy.backoff_max);
    let jitter = doubled.mul_f64(rand::thread_rng().gen_range(0.0..0.25));
    (doubled + jitter).max(retry_after.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(status: u16) -> ServerError {
        ServerError::Status {
            status,
            code: String::new(),
            message: String::new(),
            retry_after: None,
        }
    }

    #[test]
    fn only_401_404_410_stop_the_uploader() {
        for ends in [401, 404, 410] {
            assert!(status(ends).ends_upload(), "{ends}");
        }
        for retried in [400, 402, 403, 409, 413, 422, 429, 500, 502, 503, 504] {
            assert!(!status(retried).ends_upload(), "{retried}");
        }
        for retried in [
            ServerError::Transport("refused".into()),
            ServerError::Malformed("not json".into()),
            ServerError::Tls("untrusted".into()),
        ] {
            assert!(!retried.ends_upload(), "{retried}");
        }
    }

    #[test]
    fn the_wrap_up_stops_one_hour_after_the_stop() {
        let one_hour = Duration::from_secs(3600);
        assert_eq!(Policy::default().wrap_up, one_hour);
        assert_eq!(wrap_up_length(Some(&Policy::default())), one_hour);
        // A run with no server has only the seal to wrap up, and half an hour for it.
        assert_eq!(wrap_up_length(None), Duration::from_secs(1800));

        let wrap = WrapUp::after(one_hour);
        let left = wrap.remaining();
        assert!(
            left <= one_hour && left > one_hour - Duration::from_secs(5),
            "{left:?}"
        );
        assert!(wrap.remaining() <= left, "it only runs down");
        assert!(WrapUp::one_attempt().remaining().is_zero());
    }

    #[test]
    fn the_uploader_sends_every_thirty_minutes() {
        let policy = Policy::default();
        assert_eq!(policy.interval, Duration::from_secs(30 * 60));

        let now = Instant::now();
        let mut cadence = Cadence::new(&policy);
        // The first interval after the opening chunk is a random share of the thirty minutes.
        assert_eq!(cadence.next_due_with(now, 0.0), now);
        assert_eq!(
            cadence.next_due_with(now, 0.5),
            now + Duration::from_secs(30 * 60),
            "and every one after it is the whole of them"
        );
        let mut cadence = Cadence::new(&policy);
        assert_eq!(
            cadence.next_due_with(now, 0.5),
            now + Duration::from_secs(15 * 60)
        );
        assert_eq!(
            cadence.next_due_with(now, 0.0),
            now + Duration::from_secs(30 * 60)
        );
    }

    #[test]
    fn the_first_interval_is_spread_over_the_first_thirty_minutes() {
        let policy = Policy::default();
        let now = Instant::now();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..50 {
            let due = Cadence::new(&policy).next_due(now);
            assert!(due >= now && due < now + policy.interval);
            seen.insert(due);
        }
        assert!(seen.len() > 1, "fifty runs did not all pick one moment");

        // A policy that is not spread sends on the full interval from the start.
        let fixed = Policy {
            spread_first: false,
            ..policy
        };
        assert_eq!(Cadence::new(&fixed).next_due(now), now + fixed.interval);
    }
}
