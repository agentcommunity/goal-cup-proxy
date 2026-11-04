//! The host decision and refusal learning (PRD-03 sections 2 to 4). The deciding name is the SNI,
//! else the CONNECT host; the answer comes from the one host list (seam S3), never from matching
//! copied here. Per boot, in memory only, never persisted: the trusted set (pairs of host and
//! ClientHello fingerprint that completed a handshake with our leaf), the refusal table (pairs
//! that refused it) and the count of silent closes. A refusal is an explicit certificate alert, or
//! [`SILENT_STRIKES`] closes without an alert after our flight went out. A trusted pair is never
//! learned and its count is cleared. A first refusal expires after [`REFUSAL_EXPIRY`], so the next
//! connection of the pair tries our leaf again; a second one holds for the boot. Times are
//! `Recorder::now()` values passed in (rule 9).

use crate::connector::HostList;
use crate::connector::hosts::Decision;
use crate::pipeline::{Mono, Recorder};
use crate::proxy::connect::fingerprint_hex16;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// How long a learned refusal makes its pair a tunnel.
pub const REFUSAL_EXPIRY: Duration = Duration::from_secs(600);

/// How many closes without an alert after our flight teach a pair like one alert.
pub const SILENT_STRIKES: u32 = 3;

/// A host, lowercased, and a ClientHello fingerprint.
type Pair = (String, [u8; 32]);

/// The host list and the boot's two tables. One per boot, shared by every connection.
pub struct Hosts {
    list: HostList,
    tables: Mutex<Tables>,
}

#[derive(Default)]
struct Tables {
    trusted: HashSet<Pair>,
    /// When each pair was last learned (`None`: for the boot) and how many times.
    refused: HashMap<Pair, (Option<Mono>, u32)>,
    /// Closes without an alert since the pair was last learned or trusted.
    silent: HashMap<Pair, u32>,
}

impl Hosts {
    pub fn new(list: HostList) -> Hosts {
        Hosts {
            list,
            tables: Mutex::default(),
        }
    }

    /// The deciding name and its answer: the SNI when there is one, else the CONNECT host. A
    /// listed SNI behind any CONNECT host is decrypted; an unlisted SNI behind a listed one is not.
    pub fn decide<'a>(&self, sni: Option<&'a str>, connect_host: &'a str) -> (&'a str, Decision) {
        let name = sni.filter(|s| !s.is_empty()).unwrap_or(connect_host);
        (name, self.list.decide(name))
    }

    /// Whether `host` is in the `expected` array of the list.
    pub fn is_expected(&self, host: &str) -> bool {
        self.list.is_expected(host)
    }

    /// True while (host, fingerprint) is a learned refusal that holds at `now`: for the boot, or
    /// younger than [`REFUSAL_EXPIRY`].
    pub fn refused(&self, now: Mono, host: &str, fp: &[u8; 32]) -> bool {
        let tables = self.lock();
        matches!(tables.refused.get(&pair(host, fp)), Some((at, _)) if holds(*at, now))
    }

    /// A handshake with our leaf completed: the pair is trusted for the boot and never learned.
    pub fn trust(&self, host: &str, fp: &[u8; 32]) {
        let key = pair(host, fp);
        let mut tables = self.lock();
        tables.refused.remove(&key);
        tables.silent.remove(&key);
        tables.trusted.insert(key);
    }

    /// An explicit certificate alert on our leaf (`tls::Refusal::Alert`). A trusted pair is never
    /// learned: no note, false. Otherwise the answer is true and, unless the pair's refusal still
    /// holds (two handshakes in flight), it is recorded at `now` (the second time for the boot) with
    /// one `ca_refused` note (host and fingerprint prefix).
    pub fn learn(&self, rec: &Recorder, now: Mono, host: &str, fp: &[u8; 32]) -> bool {
        let key = pair(host, fp);
        {
            let mut tables = self.lock();
            if tables.trusted.contains(&key) {
                return false;
            }
            let (at, strikes) = tables.refused.entry(key).or_insert((Some(now), 0));
            if *strikes > 0 && holds(*at, now) {
                return true;
            }
            *strikes += 1;
            *at = (*strikes == 1).then_some(now);
        }
        rec.event(json!({
            "ev": "note",
            "what": "ca_refused",
            "host": host,
            "fingerprint": fingerprint_hex16(fp),
        }));
        true
    }

    /// A close without an alert after our flight went out (`tls::Refusal::NoAlert`). The
    /// [`SILENT_STRIKES`]th since the pair was last learned or trusted is [`Hosts::learn`]ed;
    /// before that, false.
    pub fn learn_silent(&self, rec: &Recorder, now: Mono, host: &str, fp: &[u8; 32]) -> bool {
        {
            let mut tables = self.lock();
            let count = tables.silent.entry(pair(host, fp)).or_default();
            *count += 1;
            if *count < SILENT_STRIKES {
                return false;
            }
            tables.silent.remove(&pair(host, fp));
        }
        self.learn(rec, now, host, fp)
    }

    fn lock(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn pair(host: &str, fp: &[u8; 32]) -> Pair {
    (host.to_ascii_lowercase(), *fp)
}

/// Whether a refusal learned at `at` (`None`: for the boot) still holds at `now`.
fn holds(at: Option<Mono>, now: Mono) -> bool {
    at.is_none_or(|at| u128::from(now.ns().saturating_sub(at.ns())) < REFUSAL_EXPIRY.as_nanos())
}
