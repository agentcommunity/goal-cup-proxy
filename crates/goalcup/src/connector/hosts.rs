//! The host list (PRD-11 section 4): the shared file, the participant's registered hosts and the
//! hosts a harness config names, plus `never` and `pinned`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ConnectorError;
use super::schema::from_toml;

/// goalcup's own server and its subdomains: never decrypted, whatever a list says.
pub(crate) const OWN_SERVER: &[&str] = &["agentcommunity.org", "*.agentcommunity.org"];

/// Where a host entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The shared host file (built in, or `--hosts`).
    Shared,
    /// `record-hosts.txt`, written by `goalcup setup --record-host`.
    Participant,
    /// A harness config file or a named variable (a hint).
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
    /// The name points at this computer: a hint then writes a `local_model` note.
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
    /// SHA-256 of the shared file as read, 64 hex.
    sha256: String,
}

/// The shared host file as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHosts {
    #[serde(default)]
    provider: Vec<String>,
    #[serde(default)]
    gateway: Vec<String>,
    #[serde(default)]
    vendor: Vec<String>,
    #[serde(default)]
    never: Vec<String>,
    #[serde(default)]
    pinned: Vec<String>,
}

impl HostList {
    /// Parses a shared host file (`connectors/model-hosts.toml` or a `--hosts` copy). `file` names
    /// it in errors.
    pub fn from_file(bytes: &[u8], file: &str) -> Result<HostList, ConnectorError> {
        let raw: RawHosts = from_toml(bytes, file)?;
        let checked = |array: &str, i: usize, name: &str| {
            check(name, Source::Shared).map_err(|_| ConnectorError::Invalid {
                file: file.to_string(),
                key: format!("{array}[{i}]"),
                reason: "not a host name or pattern",
            })
        };
        let pinned = (raw.pinned.iter().enumerate())
            .map(|(i, name)| checked("pinned", i, name).map(|h| h.name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut shared = Vec::new();
        for (array, kind, names) in [
            ("provider", Kind::Provider, &raw.provider),
            ("gateway", Kind::Gateway, &raw.gateway),
            ("vendor", Kind::Vendor, &raw.vendor),
        ] {
            for (i, name) in names.iter().enumerate() {
                let host = checked(array, i, name)?;
                shared.push(Host {
                    pinned: pinned.iter().any(|p| matches(p, &host.name)),
                    kind,
                    name: host.name,
                });
            }
        }
        for name in &pinned {
            if !shared.iter().any(|h| &h.name == name) {
                shared.push(Host {
                    name: name.clone(),
                    kind: Kind::Other,
                    pinned: true,
                });
            }
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

    /// Whether `name` is decrypted, and as what. The hosts slice fills this in; until then every
    /// name is a tunnel.
    pub fn decide(&self, _name: &str) -> Decision {
        Decision::NotListed
    }

    /// The wildcard-free suffix of each entry, for the run CA's name constraints. The hosts slice
    /// fills this in.
    pub fn ca_subtrees(&self) -> Vec<String> {
        Vec::new()
    }

    /// The three lists for the `start` event (PRD-01 section 11).
    pub fn lists(&self) -> serde_json::Value {
        serde_json::json!({
            "shared": self.shared,
            "participant": self.participant,
            "config": self.config,
        })
    }

    /// SHA-256 of the shared file as read, 64 hex.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Hosts named by a harness config or a named variable.
    pub(crate) fn add_config(&mut self, hosts: Vec<Host>) {
        for host in hosts {
            if !self.config.contains(&host) {
                self.config.push(host);
            }
        }
    }

    /// A name that is never decrypted.
    pub(crate) fn add_never(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        if !name.is_empty() && !self.never.contains(&name) {
            self.never.push(name);
        }
    }
}

/// Whether `name` matches the host `pattern`: the exact name, `*.a.b` (one or more labels before
/// `a.b`), or one `*` inside one label.
pub(crate) fn matches(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.")
        && !suffix.contains('*')
    {
        return name
            .strip_suffix(suffix)
            .is_some_and(|before| before.len() > 1 && before.ends_with('.'));
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

/// The one host check for every source. The hosts slice copies the full rules (PRD-17); until
/// then it lowercases and checks the label shape only.
pub fn check(name: &str, source: Source) -> Result<Host, Skipped> {
    let name = name.to_ascii_lowercase();
    let label_ok = |label: &str| {
        (1..=63).contains(&label.len())
            && label.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'-'
                    || (b == b'*' && source == Source::Shared)
            })
    };
    if name.len() > 253 || name.split('.').count() < 2 || !name.split('.').all(label_ok) {
        return Err(Skipped {
            reason: "not a host name".to_string(),
            local: false,
        });
    }
    Ok(Host {
        name,
        kind: Kind::Other,
        pinned: false,
    })
}
