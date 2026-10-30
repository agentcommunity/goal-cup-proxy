//! The host list (PRD-11 section 4): the shared file, the participant's registered hosts and the
//! hosts a harness config names, plus `never` and `pinned`.

use std::net::IpAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ConnectorError;
use super::schema::from_toml;

/// goalcup's own server; it and its subdomains are never decrypted, whatever a list says.
const OWN_SERVER: &str = "agentcommunity.org";

/// Where a host entry came from: the shared file (built in, or `--hosts`), `record-hosts.txt`
/// (`goalcup setup --record-host`), or a harness config file or named variable (a hint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Shared,
    Participant,
    Config,
}

/// What kind of service a host is: the shared file's array name, or `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Provider,
    Gateway,
    Vendor,
    Other,
}

/// The answer for one host name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Listed {
        source: Source,
        kind: Kind,
        pinned: bool,
    },
    Never,
    NotListed,
}

/// One entry of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    #[serde(rename = "host")]
    pub name: String,
    pub kind: Kind,
    pub pinned: bool,
}

/// Why a name was not taken. It never holds the name's value beyond what the reason says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub reason: String,
    /// The name points at this computer or a private network: a hint writes a `local_model` note.
    pub local: bool,
}

/// The three lists in force, with `never`, `pinned` and the shared file's hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostList {
    shared: Vec<Host>,
    participant: Vec<Host>,
    config: Vec<Host>,
    never: Vec<String>,
    pinned: Vec<String>,
    sha256: String,
}

/// The shared host file as written.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawHosts {
    provider: Vec<String>,
    gateway: Vec<String>,
    vendor: Vec<String>,
    never: Vec<String>,
    pinned: Vec<String>,
}

impl HostList {
    /// Parses the shared host file or a `--hosts` copy; a pinned name no array holds is `other`.
    pub fn from_file(bytes: &[u8], file: &str) -> Result<HostList, ConnectorError> {
        let raw: RawHosts = from_toml(bytes, file)?;
        let checked = |array: &str, i: usize, name: &str| {
            check(name, Source::Shared).map_err(|_| ConnectorError::Invalid {
                file: file.to_string(),
                key: format!("{array}[{i}]"),
                reason: "not a host name or pattern",
            })
        };
        let pins = (raw.pinned.iter().enumerate())
            .map(|(i, name)| checked("pinned", i, name))
            .collect::<Result<Vec<_>, _>>()?;
        let pinned: Vec<String> = pins.iter().map(|h| h.name.clone()).collect();
        let mut shared = Vec::new();
        for (array, kind, names) in [
            ("provider", Kind::Provider, &raw.provider),
            ("gateway", Kind::Gateway, &raw.gateway),
            ("vendor", Kind::Vendor, &raw.vendor),
        ] {
            for (i, name) in names.iter().enumerate() {
                let host = checked(array, i, name)?;
                add(&mut shared, &pinned, Host { kind, ..host });
            }
        }
        for host in pins {
            add(&mut shared, &pinned, host);
        }
        Ok(HostList {
            shared,
            participant: Vec::new(),
            config: Vec::new(),
            never: raw.never.iter().map(|n| n.to_ascii_lowercase()).collect(),
            pinned,
            sha256: hex::encode(Sha256::digest(bytes)),
        })
    }

    /// Whether `name` is decrypted, and as what. `never` and goalcup's own server win over all.
    pub fn decide(&self, name: &str) -> Decision {
        let name = name.to_ascii_lowercase();
        if own_server(&name) || self.never.iter().any(|p| matches(p, &name)) {
            return Decision::Never;
        }
        let pinned = self.pinned.iter().any(|p| matches(p, &name));
        for (source, list) in [
            (Source::Shared, &self.shared),
            (Source::Participant, &self.participant),
            (Source::Config, &self.config),
        ] {
            if let Some(&Host { kind, .. }) = list.iter().find(|h| matches(&h.name, &name)) {
                return Decision::Listed {
                    source,
                    kind,
                    pinned,
                };
            }
        }
        Decision::NotListed
    }

    /// The wildcard-free suffix of each entry for the run CA (`*.a.b` and `x*y.a.b` give `a.b`).
    pub fn ca_subtrees(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let all = [&self.shared, &self.participant, &self.config];
        for host in all.into_iter().flatten() {
            let labels: Vec<&str> = host.name.split('.').collect();
            let star = labels.iter().rposition(|l| l.contains('*'));
            let suffix = labels[star.map_or(0, |i| i + 1)..].join(".");
            if !suffix.is_empty() && !out.contains(&suffix) {
                out.push(suffix);
            }
        }
        out
    }

    /// The three lists for the `start` event (PRD-01 section 11).
    pub fn lists(&self) -> serde_json::Value {
        let (s, p, c) = (&self.shared, &self.participant, &self.config);
        serde_json::json!({ "shared": s, "participant": p, "config": c })
    }

    /// SHA-256 of the shared file as read, 64 hex.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Hosts named by a harness config or a named variable.
    pub(crate) fn add_config(&mut self, hosts: Vec<Host>) {
        for host in hosts {
            add(&mut self.config, &self.pinned, host);
        }
    }

    /// The participant's hosts from `$GOALCUP_HOME/record-hosts.txt` (PRD-10), one per line. A
    /// missing file adds nothing; a line that fails the check (a `#` line too) is never logged. A
    /// `never` host is left out with the log line returned (section 4).
    pub(crate) fn add_participant(&mut self, goalcup_home: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(goalcup_home.join("record-hosts.txt"));
        let text = text.unwrap_or_default();
        let hosts = text.lines().filter_map(|l| check(l.trim(), Source::Participant).ok());
        let mut log = Vec::new();
        for host in hosts {
            if self.decide(&host.name) == Decision::Never {
                log.push(format!("record-hosts.txt: {} is never decrypted", host.name));
            } else {
                add(&mut self.participant, &self.pinned, host);
            }
        }
        log
    }

    /// A name that is never decrypted.
    pub(crate) fn add_never(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        if !name.is_empty() && !self.never.contains(&name) {
            self.never.push(name);
        }
    }
}

/// Adds `host` to `list` once by name, `pinned` when a pinned pattern matches it.
fn add(list: &mut Vec<Host>, pinned: &[String], mut host: Host) {
    host.pinned = pinned.iter().any(|p| matches(p, &host.name));
    if !list.iter().any(|h| h.name == host.name) {
        list.push(host);
    }
}

/// Whether `name` matches `pattern`: exact, `*.a.b` (labels before `a.b`), or one `*` in a label.
pub(crate) fn matches(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.").filter(|s| !s.contains('*')) {
        let labels_before = |b: &str| b.len() > 1 && b.ends_with('.');
        return name.strip_suffix(suffix).is_some_and(labels_before);
    }
    let (p, n): (Vec<&str>, Vec<&str>) = (pattern.split('.').collect(), name.split('.').collect());
    p.len() == n.len()
        && p.iter().zip(&n).all(|(p, n)| match p.split_once('*') {
            Some((head, tail)) => {
                n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail)
            }
            None => p == n,
        })
}

/// goalcup's own server or a subdomain, or any name holding `goalcup` (PRD-17 N5).
fn own_server(name: &str) -> bool {
    let sub = |before: &str| before.is_empty() || before.ends_with('.');
    name.contains("goalcup") || name.strip_suffix(OWN_SERVER).is_some_and(sub)
}

/// Whether every address of a name is loopback or unspecified after `to_canonical` (N2, N6).
pub fn resolves_only_here(addrs: &[IpAddr]) -> bool {
    let here = |ip: &IpAddr| ip.to_canonical().is_loopback() || ip.to_canonical().is_unspecified();
    !addrs.is_empty() && addrs.iter().all(here)
}

/// This computer or a private network; a mapped public address is neither.
fn is_local(ip: IpAddr) -> bool {
    resolves_only_here(&[ip])
        || match ip.to_canonical() {
            IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
        }
}

/// The one host check for every source (PRD-17 section 2.7). Patterns: `Shared` any, `Participant`
/// one leading `*.`, `Config` none. The reason never repeats the input.
pub fn check(name: &str, source: Source) -> Result<Host, Skipped> {
    let skip = |reason: &str, local| {
        let reason = reason.to_string();
        Err(Skipped { reason, local })
    };
    if name.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\') {
        return skip("holds a backslash, a space or a control byte", false);
    }
    let name = name.to_ascii_lowercase();
    if let Ok(ip) = name.parse::<IpAddr>() {
        return skip("an address, not a host name", is_local(ip));
    }
    if name == "localhost" || name.ends_with(".localhost") || name == "localhost.localdomain" {
        return skip("a name of this computer", true);
    }
    let last = name.rsplit('.').next().unwrap_or_default();
    if last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") {
        return skip("an address spelled as a name", false);
    }
    if own_server(&name) {
        return skip("goalcup's own server", false);
    }
    let labels: Vec<&str> = name.split('.').collect();
    let byte_ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'*';
    let label_ok = |l: &&str| (1..=63).contains(&l.len()) && l.bytes().all(byte_ok);
    if name.len() > 253 || labels.len() < 2 || !labels.iter().all(label_ok) {
        return skip("not a host name", false);
    }
    let star = labels.iter().position(|l| l.contains('*'));
    let shape_ok = match (source, star) {
        (_, None) | (Source::Shared, Some(_)) => true,
        (Source::Participant, Some(i)) => i == 0 && labels[0] == "*",
        (Source::Config, Some(_)) => false,
    };
    // One `*`, two fixed labels after it: a CA subtree is never a bare top-level name.
    if !shape_ok || name.matches('*').count() > 1 || star.is_some_and(|i| labels.len() - i < 3) {
        return skip("not an allowed host pattern", false);
    }
    let (kind, pinned) = (Kind::Other, false);
    Ok(Host { name, kind, pinned })
}
