//! The `goalcup start` connectivity preflight: before the entry, the proxy or the harness
//! exists, one bounded TCP connect to each provider host the profile intercepts.
//!
//! A run whose provider is unreachable from the start would otherwise record a harness that
//! never got an answer, as a valid sealed entry. The commonest cause measured on a
//! participant's machine is not the network but a per-application firewall (Little Snitch,
//! LuLu, endpoint security) holding the connections of a freshly built, unsigned `goalcup`
//! until someone answers its prompt; the refusal names this binary's path for that reason.
//!
//! The probe is the proxy's own upstream connect ([`super::connect`]): it resolves, cycles the
//! addresses and retries inside [`PREFLIGHT_BUDGET`], every host at once. It sends nothing over
//! the connection and closes it. A loopback host (`localhost`, a `.localhost` name, a loopback
//! literal) has no network path to check and is skipped. Nothing here is said to the harness,
//! which does not exist yet.

use super::connect::{ConnectFailureKind, DEFAULT_CONNECT_ATTEMPT_TIMEOUT, Retry, connect_host};
use std::path::PathBuf;
use std::time::Duration;

/// The whole preflight's bound, all hosts together.
pub(crate) const PREFLIGHT_BUDGET: Duration = Duration::from_secs(10);

/// The port every provider host is probed on.
pub(crate) const PREFLIGHT_PORT: u16 = 443;

/// A provider host the preflight could not reach. Its `Display` is the whole refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unreachable {
    pub host: String,
    pub port: u16,
    pub kind: ConnectFailureKind,
    pub budget: Duration,
    /// This process's executable, when it can be named.
    pub binary: Option<PathBuf>,
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let binary = match &self.binary {
            Some(path) => path.display().to_string(),
            None => "this goalcup binary".to_string(),
        };
        write!(
            f,
            "`goalcup start` refuses this run: the provider host {}:{} could not be reached \
             within {}s ({}), so the harness would get no answer. Nothing was created. If this \
             machine is online, a firewall such as Little Snitch may be holding this binary's \
             connections: allow {binary} to connect, then start again.",
            self.host,
            self.port,
            self.budget.as_secs(),
            describe(self.kind),
        )
    }
}

impl Unreachable {
    /// The host and what went wrong, without `goalcup start`'s refusal around it: for
    /// `goalcup prep`, which says the same thing in its own sentence.
    pub(crate) fn reason(&self) -> String {
        format!(
            "{}:{} could not be reached within {}s ({})",
            self.host,
            self.port,
            self.budget.as_secs(),
            describe(self.kind)
        )
    }
}

/// The failure subtype in words.
fn describe(kind: ConnectFailureKind) -> &'static str {
    match kind {
        ConnectFailureKind::Dns => "the DNS lookup failed",
        ConnectFailureKind::NoAddresses => "the DNS lookup returned no address",
        ConnectFailureKind::TcpRefused => "the connection was refused",
        ConnectFailureKind::TcpTimeout => "the TCP connect timed out",
        ConnectFailureKind::TcpPermissionDenied => "the connection was not permitted",
        ConnectFailureKind::TcpOther => "the TCP connect failed",
    }
}

/// Whether `host` needs no network to reach: `localhost`, a `.localhost` name (RFC 6761) or a
/// loopback address literal.
fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The preflight `goalcup start` runs: every non-loopback host on [`PREFLIGHT_PORT`] within
/// [`PREFLIGHT_BUDGET`]. Blocks the calling thread for at most the budget.
pub(crate) fn upstreams_reachable<'a>(
    hosts: impl IntoIterator<Item = &'a str>,
) -> Result<(), Unreachable> {
    let hosts: Vec<String> = hosts
        .into_iter()
        .filter(|host| !is_loopback_host(host))
        .map(str::to_owned)
        .collect();
    probe(hosts, PREFLIGHT_PORT, PREFLIGHT_BUDGET)
}

/// Connects to every host on `port` at once, within `budget`; the first host (in the given
/// order) that could not be reached is the answer.
fn probe(hosts: Vec<String>, port: u16, budget: Duration) -> Result<(), Unreachable> {
    if hosts.is_empty() {
        return Ok(());
    }
    let unreachable = |host: String, kind| Unreachable {
        host,
        port,
        kind,
        budget,
        binary: std::env::current_exe().ok(),
    };
    // A runtime of its own, gone before the proxy's is built. One that cannot be built is not
    // a reason to refuse here: the proxy's own start would refuse it a moment later.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return Ok(());
    };
    let attempt = DEFAULT_CONNECT_ATTEMPT_TIMEOUT.min(budget);
    let failed = runtime.block_on(async {
        let until = tokio::time::Instant::now() + budget;
        let mut probes = tokio::task::JoinSet::new();
        for (index, host) in hosts.iter().cloned().enumerate() {
            probes.spawn(async move {
                let reached = connect_host(&host, port, attempt, until, Retry::UntilDeadline).await;
                (index, reached.err())
            });
        }
        let mut failed: Vec<(usize, ConnectFailureKind)> = Vec::new();
        while let Some(joined) = probes.join_next().await {
            match joined {
                Ok((index, Some(kind))) => failed.push((index, kind)),
                Ok((_, None)) => {}
                // A probe cannot panic short of a bug; it reached nothing.
                Err(_) => {}
            }
        }
        failed.into_iter().min_by_key(|(index, _)| *index)
    });
    // A lookup still blocked in the resolver is abandoned, not waited for.
    runtime.shutdown_background();
    match failed {
        Some((index, kind)) => Err(unreachable(hosts[index].clone(), kind)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark_port() -> u16 {
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        reserved.local_addr().unwrap().port()
    }

    #[test]
    fn an_unreachable_host_is_named_with_its_subtype_and_this_binary() {
        let port = dark_port();
        let started = std::time::Instant::now();
        let refused = probe(vec!["127.0.0.1".into()], port, Duration::from_secs(1))
            .expect_err("nothing listens there");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(refused.host, "127.0.0.1");
        assert_eq!(refused.kind, ConnectFailureKind::TcpRefused);
        let said = refused.to_string();
        assert!(said.contains(&format!("127.0.0.1:{port}")), "{said}");
        assert!(said.contains("the connection was refused"), "{said}");
        assert!(said.contains("Little Snitch"), "{said}");
        assert!(said.contains("Nothing was created."), "{said}");
        let binary = std::env::current_exe().unwrap();
        assert!(said.contains(&binary.display().to_string()), "{said}");
    }

    #[test]
    fn the_first_unreachable_host_in_order_is_the_answer_and_reachable_ones_pass() {
        let listening = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listening.local_addr().unwrap().port();
        assert_eq!(
            probe(vec!["127.0.0.1".into()], port, Duration::from_secs(2)),
            Ok(())
        );
        let refused = probe(
            vec!["127.0.0.1".into(), "preflight-unreachable.invalid".into()],
            port,
            Duration::from_secs(2),
        )
        .expect_err("a name that cannot resolve");
        assert_eq!(refused.host, "preflight-unreachable.invalid");
        assert_eq!(refused.kind, ConnectFailureKind::Dns);
    }

    #[test]
    fn loopback_hosts_are_not_probed() {
        for host in ["localhost", "api.e00.localhost", "127.0.0.1", "::1"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["chatgpt.com", "api.anthropic.com", "localhost.example.com"] {
            assert!(!is_loopback_host(host), "{host}");
        }
        // Nothing listens on 443 here, so only a skip passes.
        assert_eq!(
            upstreams_reachable(["localhost", "api.e00.localhost"]),
            Ok(())
        );
    }
}
