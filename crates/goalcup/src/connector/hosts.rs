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
    /// Parses a shared host file (`connectors/model-hosts.toml` or a `--hosts` copy).
    pub fn from_file(bytes: &[u8]) -> Result<HostList, ConnectorError> {
        let file = "the host list";
        let text = std::str::from_utf8(bytes).map_err(|_| ConnectorError::Parse {
            file: file.to_string(),
            line: 0,
            message: "not UTF-8".to_string(),
        })?;
        let raw: RawHosts = from_toml(text, file)?;
        let pinned: Vec<String> = raw.pinned.iter().map(|p| p.to_ascii_lowercase()).collect();
        let mut shared = Vec::new();
        for (array, kind, names) in [
            ("provider", Kind::Provider, &raw.provider),
            ("gateway", Kind::Gateway, &raw.gateway),
            ("vendor", Kind::Vendor, &raw.vendor),
        ] {
            for (i, name) in names.iter().enumerate() {
                let host = check(name, Source::Shared).map_err(|_| ConnectorError::Invalid {
                    file: file.to_string(),
                    key: format!("{array}[{i}]"),
                    reason: "not a host name or pattern",
                })?;
                shared.push(Host {
                    pinned: pinned.contains(&host.name),
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
        let list = |hosts: &[Host]| {
            hosts
                .iter()
                .map(|h| serde_json::json!({"host": h.name, "kind": h.kind, "pinned": h.pinned}))
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "shared": list(&self.shared),
            "participant": list(&self.participant),
            "config": list(&self.config),
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

/// The one host check for every source. The hosts slice copies the full rules (PRD-17); until
/// then it lowercases and checks the label shape only.
pub fn check(name: &str, source: Source) -> Result<Host, Skipped> {
    let name = name.to_ascii_lowercase();
    let label_ok = |label: &str| {
        (1..=63).contains(&label.len())
            && label.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || (b == b'*' && source == Source::Shared)
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
