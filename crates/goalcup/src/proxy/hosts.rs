//! The host decision and refusal learning (PRD-03 sections 2 to 4). The deciding name is the SNI,
//! else the CONNECT host; the answer comes from the one host list (seam S3), never from matching
//! copied here. Per boot, two tables in memory only, never persisted: the trusted set (pairs of
//! host and ClientHello fingerprint that completed a handshake with our leaf) and the refusal
//! table (pairs that sent an explicit certificate alert on it). A trusted pair is never learned; a
//! learned refusal expires after [`REFUSAL_EXPIRY`], so the next connection of the pair tries our
//! leaf again. Times are `Recorder::now()` values passed in (rule 9).

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
    /// When each pair was learned.
    refused: HashMap<Pair, Mono>,
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
    pub fn decide(&self, sni: Option<&str>, connect_host: &str) -> (String, Decision) {
        let name = sni.filter(|s| !s.is_empty()).unwrap_or(connect_host);
        (name.to_owned(), self.list.decide(name))
    }

    /// True while (host, fingerprint) is a learned refusal younger than [`REFUSAL_EXPIRY`] at
    /// `now`; an expired entry is removed, so the next connection of the pair tries our leaf again.
    pub fn refused(&self, now: Mono, host: &str, fp: &[u8; 32]) -> bool {
        let key = pair(host, fp);
        let mut tables = self.lock();
        match tables.refused.get(&key) {
            Some(&at) if fresh(at, now) => true,
            Some(_) => {
                tables.refused.remove(&key);
                false
            }
            None => false,
        }
    }

    /// A handshake with our leaf completed: the pair is trusted for the boot and never learned.
    pub fn trust(&self, host: &str, fp: &[u8; 32]) {
        let key = pair(host, fp);
        let mut tables = self.lock();
        tables.refused.remove(&key);
        tables.trusted.insert(key);
    }

    /// An explicit certificate alert on our leaf (`tls::Refusal::Alert`). A trusted pair is never
    /// learned: no note, false. Otherwise the pair is recorded at `now`, one `ca_refused` note is
    /// written (host and fingerprint prefix) and the answer is true. Expired entries go meanwhile.
    pub fn learn(&self, rec: &Recorder, now: Mono, host: &str, fp: &[u8; 32]) -> bool {
        let key = pair(host, fp);
        {
            let mut tables = self.lock();
            if tables.trusted.contains(&key) {
                return false;
            }
            tables.refused.retain(|_, at| fresh(*at, now));
            tables.refused.insert(key, now);
        }
        rec.event(json!({
            "ev": "note",
            "what": "ca_refused",
            "host": host,
            "fingerprint": fingerprint_hex16(fp),
        }));
        true
    }

    fn lock(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn pair(host: &str, fp: &[u8; 32]) -> Pair {
    (host.to_ascii_lowercase(), *fp)
}

/// Whether a refusal learned at `at` still holds at `now`.
fn fresh(at: Mono, now: Mono) -> bool {
    u128::from(now.ns().saturating_sub(at.ns())) < REFUSAL_EXPIRY.as_nanos()
}
