//! The shared host list this build was made with (`connectors/model-hosts.toml`), the same file
//! the recorder compiles in. Honesty judges a host by it, whatever the record's label says.

use goalcup_format::shared::host_matches;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

const FILE: &str = include_str!("../../../connectors/model-hosts.toml");

/// The arrays honesty reads; the others (`pinned`, `expected`) name no kind. A missing array is
/// empty, as in the recorder's parser.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Lists {
    provider: Vec<String>,
    gateway: Vec<String>,
    vendor: Vec<String>,
    never: Vec<String>,
}

static LISTS: LazyLock<Lists> = LazyLock::new(|| {
    let mut l: Lists = toml::from_str(FILE).expect("connectors/model-hosts.toml parses");
    // The recorder lowercases `never` as it reads it (connector/hosts.rs `from_file`).
    l.never.iter_mut().for_each(|n| n.make_ascii_lowercase());
    l
});

/// goalcup's own server; the recorder's `OWN_SERVER` (crates/goalcup/src/connector/hosts.rs).
const OWN_SERVER: &str = "agentcommunity.org";

/// SHA-256 of the file's bytes, lowercase hex: the recorder's `hosts_sha256` for the built-in list.
pub fn sha256() -> &'static str {
    static HASH: LazyLock<String> = LazyLock::new(|| goalcup_format::hex(&Sha256::digest(FILE)));
    &HASH
}

/// `provider`, `gateway` or `vendor` when the file lists `host`; `None` for a `never` host, as the
/// recorder decides.
pub fn kind(host: &str) -> Option<&'static str> {
    if never(host) {
        return None;
    }
    let (l, host) = (&*LISTS, host.to_ascii_lowercase());
    let hit = |list: &[String]| list.iter().any(|p| host_matches(p, &host));
    [
        ("provider", &l.provider),
        ("gateway", &l.gateway),
        ("vendor", &l.vendor),
    ]
    .into_iter()
    .find_map(|(k, list)| hit(list).then_some(k))
}

/// Whether the recorder never decrypts `host`: a `never` host, or goalcup's own server (a
/// subdomain of it, or any name holding `goalcup`), the recorder's `decide` rule.
pub fn never(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let sub = |before: &str| before.is_empty() || before.ends_with('.');
    let own = host.contains("goalcup") || host.strip_suffix(OWN_SERVER).is_some_and(sub);
    own || LISTS.never.iter().any(|p| host_matches(p, &host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goalcups_own_server_is_never_as_the_recorder_decides() {
        for h in [
            "agentcommunity.org",
            "Up.AgentCommunity.org",
            "x.goalcup.dev",
        ] {
            assert!(never(h), "{h}");
            assert_eq!(kind(h), None, "{h}");
        }
        assert!(!never("notagentcommunity.org"));
    }
}
