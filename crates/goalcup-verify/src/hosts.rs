//! The shared host list this build was made with (`connectors/model-hosts.toml`), the same file
//! the recorder compiles in. Honesty judges a host by it, whatever the record's label says.

use goalcup_format::shared::host_matches;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

const FILE: &str = include_str!("../../../connectors/model-hosts.toml");

/// The arrays honesty reads; the others (`pinned`, `expected`) name no kind.
#[derive(Deserialize)]
struct Lists {
    provider: Vec<String>,
    gateway: Vec<String>,
    vendor: Vec<String>,
    never: Vec<String>,
}

static LISTS: LazyLock<Lists> =
    LazyLock::new(|| toml::from_str(FILE).expect("connectors/model-hosts.toml parses"));

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

/// Whether `host` is a sign-in host the recorder never decrypts.
pub fn never(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    LISTS.never.iter().any(|p| host_matches(p, &host))
}
