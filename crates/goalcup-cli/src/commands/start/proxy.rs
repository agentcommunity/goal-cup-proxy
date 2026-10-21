//! The recording proxy of a capture run, in this process, and the entry's own signed status that
//! shows its producer admitted.

use crate::CliError;
use crate::commands::say;
use crate::home::RunDir;
use crate::proxy::api::{CapturePath, CapturePolicy};
use crate::proxy::connect::{
    ApprovedHost, DEFAULT_ADMISSION_TIMEOUT, DEFAULT_CONNECT_ATTEMPT_TIMEOUT, ProxyCredential,
};
use crate::proxy::ingest::{IngestSecret, IngestSink, SinkConfig, connect_sink};
use crate::proxy::tls::UpstreamRoots;
use crate::proxy::{MAX_CONNECTION_TASKS, ProxyConfig, ProxyHandle};
use crate::retry::retry_busy;
use goalcup_protocol::capture::CaptureSink;
use goalcup_protocol::control::{CaptureWorkspaceInfo, DaemonClient, SignedStatus};
use goalcup_protocol::harness::CaptureSpec;
use std::sync::Arc;
use std::time::Duration;

/// Bounds the recording proxy's start (the sender's admission of its first producer included), and
/// separately the read of the entry's own admission counts that follows it.
const CAPTURE_STARTUP: Duration = Duration::from_secs(5);

/// The soft open-file limit `goalcup start` asks for: a relayed connection holds two
/// descriptors, and the rest is room for the runtime, the sender and this process.
const OPEN_FILES_WANTED: u64 = 2 * MAX_CONNECTION_TASKS + 256;

/// Raise the soft open-file limit toward the hard one, far enough for the recording proxy's
/// connection bound: launchd starts a process at 256, and at two descriptors a connection the
/// recording proxy would stop accepting near 120.
#[cfg(unix)]
pub(super) fn raise_open_file_limit(out: &mut dyn std::io::Write) -> Result<(), CliError> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes the one struct it is handed and reads nothing else.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return Ok(());
    }
    let wanted = (OPEN_FILES_WANTED as libc::rlim_t).min(limit.rlim_max);
    if limit.rlim_cur >= wanted {
        return Ok(());
    }
    let raised = libc::rlimit {
        rlim_cur: wanted,
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `setrlimit` reads the one struct it is handed.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } != 0 {
        say(
            out,
            &format!(
                "  (the open-file limit stays at {}, so the recording proxy may refuse \
                 connections past about half of it)",
                limit.rlim_cur
            ),
        )?;
    }
    Ok(())
}

/// The recording proxy and its IPC sender, on one runtime thread in this process. When this
/// process dies they die with it, and the daemon reads the open producer as an unknown tail.
///
/// The fields drop in order: the recording proxy's tasks, then the sender's, then the runtime.
pub(super) struct Proxied {
    pub(super) proxy: ProxyHandle,
    pub(super) sink: Arc<IngestSink>,
    /// `http://goalcup:<credential>@127.0.0.1:<port>`, handed to the harness and nowhere else.
    pub(super) proxy_url: String,
    pub(super) runtime: tokio::runtime::Runtime,
}

impl Proxied {
    pub(super) fn start(
        info: &CaptureWorkspaceInfo,
        profile: &CaptureSpec,
        model_host: Option<&crate::model_host::ModelHost>,
        run: &RunDir,
        roots: UpstreamRoots,
        credential: ProxyCredential,
    ) -> Result<Proxied, CliError> {
        let refused = |what: &str, e: &dyn std::fmt::Display| {
            CliError::Refused(format!(
                "the recording proxy did not start ({what}: {e}), so the harness is not \
                 launched."
            ))
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| refused("runtime", &e))?;
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| refused("listener", &e))?;
        let address = listener.local_addr().map_err(|e| refused("listener", &e))?;
        let proxy_url = credential.proxy_url(address);
        let sink = {
            let _inside = runtime.enter();
            connect_sink(SinkConfig::new(
                info.ingest_addr,
                info.entry_id.clone(),
                IngestSecret::new(info.ingest_secret.clone()),
                info.producer_id.clone(),
                crate::finish_budget(),
            ))
            .map_err(|e| refused("sender", &e))?
        };
        let config = ProxyConfig {
            listener,
            ca_dir: run.ca.clone(),
            approved: approved_hosts(profile, model_host),
            auth_hosts: profile.auth_hosts.clone(),
            credential,
            upstream_roots: roots,
            producer: info.producer_id.clone(),
            sink: Arc::clone(&sink) as Arc<dyn CaptureSink>,
            startup_deadline: CAPTURE_STARTUP,
            // The stop is the finish's first step and is measured from its own call, which is
            // the finish deadline's start, so its bound is the drain share: it ends by
            // `drain_end`, leaving the Git read its floor and the commit reserve to the
            // daemon's seal. A connection the harness holds open at a deadline would otherwise
            // run the wait to the reserve and leave the Git read nothing (D56).
            shutdown_deadline: goalcup_protocol::control::proxy_drain_share(crate::finish_budget()),
            admission_timeout: DEFAULT_ADMISSION_TIMEOUT,
            connect_attempt_timeout: DEFAULT_CONNECT_ATTEMPT_TIMEOUT,
            max_connections: MAX_CONNECTION_TASKS as usize,
        };
        let proxy = runtime
            .block_on(crate::proxy::start(config))
            .map_err(|e| refused("proxy", &e))?;
        Ok(Proxied {
            proxy,
            sink,
            proxy_url,
            runtime,
        })
    }
}

/// The profile's hosts as the recording proxy's host rules: exact paths, then prefixes, each with
/// the host's protocol hint.
fn approved_hosts(
    profile: &CaptureSpec,
    model_host: Option<&crate::model_host::ModelHost>,
) -> Vec<ApprovedHost> {
    // The model service the harness's config names (D62) joins them; a host the profile lists
    // keeps the profile's reviewed rule.
    let model = model_host
        .filter(|m| !profile.hosts.iter().any(|h| h.host == m.host))
        .map(|m| m.approved());
    profile
        .hosts
        .iter()
        .map(|h| {
            let exact = h.evidence_paths.iter().map(|p| (p, false));
            let prefix = h.prefix_paths.iter().map(|p| (p, true));
            let paths = exact
                .chain(prefix)
                .map(|(path, prefix)| CapturePath {
                    path: path.clone(),
                    prefix,
                    protocol_hint: h.protocol_hint.clone(),
                })
                .collect();
            ApprovedHost {
                host: h.host.clone(),
                policy: CapturePolicy { paths },
            }
        })
        .chain(model)
        .collect()
}

/// Wait, within [`CAPTURE_STARTUP`], for the entry's own status to show the recording proxy's
/// producer admitted: one producer consumed and none refused. A started proxy is not that proof.
pub(super) fn admitted(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<(), CliError> {
    let until = std::time::Instant::now() + CAPTURE_STARTUP;
    loop {
        let document = verified_status(client, info)?;
        match (
            document.payload.producers_consumed,
            document.payload.admissions_refused,
        ) {
            (Some(1), Some(0)) => return Ok(()),
            (_, Some(refused)) if refused > 0 => {
                return Err(CliError::Refused(format!(
                    "the tracker did not admit the recording proxy's capture producer ({refused} \
                     refused), so nothing would be recorded and the harness is not launched."
                )));
            }
            _ => {}
        }
        if std::time::Instant::now() >= until {
            return Err(CliError::Refused(format!(
                "the tracker did not show the recording proxy's capture producer admitted within \
                 {}s, so the harness is not launched.",
                CAPTURE_STARTUP.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The one `proxy admission:` line: `accepted` and `intercepted` always, every other counter only
/// when it is not zero, as `name=count` words in a fixed order. A run in which nothing went wrong
/// is two numbers; one in which something did names it. The completed tunnels
/// (`tunneled_auth_host`, to a connector's authentication hosts, and `tunneled_not_approved`, to
/// any host it does not list) are named too: they are opaque by design and leave no record in
/// the chain, so this line is the only place that says they happened. A tunnel still open at the
/// stop is not counted. `cut` is the intercepted connections the stop itself cut (their streams
/// end `Interrupted`, as for a peer that dropped TLS, which this tells apart).
pub(super) fn proxy_admission_line(
    h: &crate::proxy::ProxyHealth,
    a: &crate::proxy::AdmissionFailureCounts,
) -> String {
    let always = [("accepted", h.accepted), ("intercepted", h.intercepted)];
    let when_nonzero = [
        ("tunneled_auth_host", h.tunneled_auth_host),
        ("tunneled_not_approved", h.tunneled_not_approved),
        ("cut", h.cut_intercepted),
        ("fail_open", h.tunneled_fail_open),
        ("auth_refused", h.admission_refused_auth),
        ("failed", h.admission_failed),
        ("malformed", a.malformed),
        ("head_too_large", a.head_too_large),
        ("not_connect", a.not_connect),
        ("timeout", a.timeout),
        ("client_io", a.client_io),
        ("upstream_connect", a.upstream_connect),
        ("upstream_connect_dns", a.upstream_connect_dns),
        (
            "upstream_connect_no_addresses",
            a.upstream_connect_no_addresses,
        ),
        (
            "upstream_connect_tcp_last_refused",
            a.upstream_connect_tcp_last_refused,
        ),
        (
            "upstream_connect_tcp_last_timeout",
            a.upstream_connect_tcp_last_timeout,
        ),
        (
            "upstream_connect_tcp_last_permission_denied",
            a.upstream_connect_tcp_last_permission_denied,
        ),
        (
            "upstream_connect_tcp_last_other",
            a.upstream_connect_tcp_last_other,
        ),
        ("upstream_tls", a.upstream_tls),
        ("client_tls", a.client_tls),
        ("alpn_incompatible", a.alpn_incompatible),
        ("upstream_outages", h.upstream_outages),
    ];
    let mut line = String::from("proxy admission:");
    for (name, count) in always
        .into_iter()
        .chain(when_nonzero.into_iter().filter(|(_, count)| *count != 0))
    {
        line.push_str(&format!(" {name}={count}"));
    }
    line
}

/// Whether the entry's own signed status shows it sealed. The read waits for the entry's run
/// lock, which a finish holds from its first stage to its commit, so a seal in flight is
/// answered after it commits.
pub(super) fn entry_is_sealed(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<bool, CliError> {
    Ok(verified_status(client, info)?.payload.sealed_at.is_some())
}

/// The entry's own status, a busy tracker waited out, and only if it is signed by the key the
/// entry's creation named and is about this entry.
fn verified_status(
    client: &(dyn DaemonClient + Sync),
    info: &CaptureWorkspaceInfo,
) -> Result<SignedStatus, CliError> {
    let document = retry_busy(|| client.status(&info.entry_id))?;
    if document.verify_with(&info.run_public_key).is_err()
        || document.payload.entry_id != info.entry_id
    {
        return Err(CliError::Internal(format!(
            "the status of entry {} is not signed by the key its creation named",
            info.entry_id
        )));
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's completed tunnels are opaque and chained nowhere, and a connection the stop cut
    /// has no marker of its own in the chain, so the `proxy admission:` line names both (a real
    /// Codex run: 26 accepted, 23 intercepted, the rest tunnels). A run with none keeps its two
    /// numbers.
    #[test]
    fn the_admission_line_accounts_for_the_tunnels() {
        let none = crate::proxy::AdmissionFailureCounts::default();
        let mut health = crate::proxy::ProxyHealth {
            accepted: 26,
            intercepted: 23,
            ..Default::default()
        };
        assert_eq!(
            proxy_admission_line(&health, &none),
            "proxy admission: accepted=26 intercepted=23"
        );
        health.tunneled_auth_host = 1;
        health.tunneled_not_approved = 2;
        health.cut_intercepted = 1;
        assert_eq!(
            proxy_admission_line(&health, &none),
            "proxy admission: accepted=26 intercepted=23 tunneled_auth_host=1 tunneled_not_approved=2 cut=1"
        );
    }
}
