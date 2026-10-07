//! CONNECT admission and SNI peeking without consuming bytes.
//!
//! [`admit`] takes one accepted loopback connection and settles it as exactly one of:
//!
//! - [`Admission::Intercepted`]: the harness asked for an approved host, authenticated to this
//!   proxy, and sent a TLS ClientHello whose SNI names that same host. Upstream TLS to the
//!   host was validated first; then the harness-facing handshake completed with a leaf from
//!   the per-launch CA. The caller gets the real stream pair and safe facts; it builds
//!   the `RelayContext` (connection ID, emitter) itself.
//! - [`Admission::Tunneled`]: an authentication host, a host that is not approved, or an
//!   approved host whose ClientHello is absent, unparsable or names another server. The
//!   tunnel ran to completion inside `admit`, relaying every byte unchanged — including the
//!   bytes read while inspecting the ClientHello. Nothing about its content is returned.
//! - [`Admission::Failed`]: a fixed [`FailureClass`] and, once the CONNECT target parsed and
//!   the caller authenticated, the target host.
//!
//! This module emits no capture frames and takes no sink: whatever it reads before a
//! decision is either replayed into the chosen path or discarded with the connection. A caller
//! that must act when a tunnel is decided, not when it ends, passes a hook to
//! [`admit_observed`].
//!
//! # Credentials
//!
//! The proxy credential authorizes this local listener only. It arrives as
//! `Proxy-Authorization: Basic base64("goalcup:<secret>")` on the CONNECT request — what a
//! client sends for the proxy URL [`ProxyCredential::proxy_url`] returns — and is compared in
//! constant time. The CONNECT head is never forwarded upstream, so the credential never
//! reaches a provider. It is never logged, hashed or retained: `Debug` impls redact it, and no
//! outcome or error formats the request line, the target text, a header or the credential.
//!
//! # Bounds
//!
//! The CONNECT head is read into at most [`MAX_CONNECT_HEAD_BYTES`]; the ClientHello
//! inspection buffers at most [`MAX_CLIENT_HELLO_PEEK_BYTES`]. Everything before the decision
//! (head, inspection, upstream connect, both handshakes) runs under one admission deadline.
//! A tunnel, once decided, has no deadline: dropping the `admit` future (for example on the
//! caller's shutdown) closes exactly the two sockets it owns and nothing else.

use super::api::{AsyncIo, CapturePolicy, RelayPair};
use super::tls::{CaError, ProxyCa, UpstreamRoots, alpn_is_http11, is_config_host};
use goalcup_protocol::constant_time::constant_time_eq;
use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::net::TcpStream;

/// The user name in the proxy credential. The secret is the password.
pub const PROXY_USERNAME: &str = "goalcup";

/// The largest CONNECT request head (request line, headers and the blank line) read before
/// the request is refused with `431`.
pub const MAX_CONNECT_HEAD_BYTES: usize = 8 * 1024;

/// The most bytes buffered while inspecting a ClientHello. A ClientHello that does not
/// complete within it is tunnelled unchanged rather than intercepted.
pub const MAX_CLIENT_HELLO_PEEK_BYTES: usize = 32 * 1024;

/// The default bound on everything before a tunnel/intercept decision is settled.
pub const DEFAULT_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);

/// The default bound on one upstream TCP connect attempt (one resolved address).
pub const DEFAULT_CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// The secret that authorizes a client to this proxy: 32 lowercase hex characters. `Debug`
/// never shows it; there is no `Clone`, so each copy is an explicit
/// [`ProxyCredential::expose_secret`] call.
pub struct ProxyCredential {
    secret: String,
}

impl std::fmt::Debug for ProxyCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyCredential(<redacted>)")
    }
}

impl ProxyCredential {
    /// A fresh random credential (128 bits from the thread CSPRNG).
    pub fn generate() -> Self {
        Self {
            secret: format!("{:032x}", rand::random::<u128>()),
        }
    }

    /// Adopts a secret the caller already holds. It must be exactly 32 lowercase hex
    /// characters.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_secret(secret: String) -> Result<Self, AdmissionConfigError> {
        if secret.len() == 32
            && secret
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self { secret })
        } else {
            Err(AdmissionConfigError::InvalidCredential)
        }
    }

    /// The secret itself, for the harness's process-scoped proxy environment. Every copy is
    /// an explicit call.
    pub fn expose_secret(&self) -> &str {
        &self.secret
    }

    /// `http://goalcup:<secret>@<listener>`: the process-scoped `HTTPS_PROXY` value for a
    /// harness. The returned string contains the secret.
    pub fn proxy_url(&self, listener: std::net::SocketAddr) -> String {
        format!("http://{PROXY_USERNAME}:{}@{listener}", self.secret)
    }

    /// Whether a `Proxy-Authorization` value is `Basic` (any case) with exactly this
    /// credential. The token comparison takes the same time wherever it differs.
    fn accepts(&self, value: &[u8]) -> bool {
        let Some(space) = value.iter().position(|&b| b == b' ') else {
            return false;
        };
        let (scheme, token) = (&value[..space], trim_ows(&value[space + 1..]));
        let expected = base64_standard(format!("{PROXY_USERNAME}:{}", self.secret).as_bytes());
        scheme.eq_ignore_ascii_case(b"basic") & constant_time_eq(token, expected.as_bytes())
    }
}

/// One intercepted host and the capture policy of its profile. Hosts are matched exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedHost {
    pub host: String,
    pub policy: CapturePolicy,
}

/// Why an [`AdmissionConfig`] or [`ProxyCredential`] was refused. Fixed text only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionConfigError {
    #[error("a configured host is not a lowercase DNS name without port or trailing dot")]
    InvalidHost,
    #[error("a host is configured more than once")]
    DuplicateHost,
    #[error("a host is configured both as approved and as an authentication host")]
    AmbiguousHost,
    #[error("the proxy credential is not 32 lowercase hex characters")]
    InvalidCredential,
    #[error("TLS preparation failed: {0}")]
    Tls(#[from] CaError),
}

struct ApprovedEntry {
    policy: CapturePolicy,
    server: Arc<rustls::ServerConfig>,
}

/// Everything admission decides with: the approved hosts (each with its capture policy and
/// a prepared leaf), the authentication hosts, the proxy credential, the upstream TLS client
/// configuration and two timeouts. Immutable once built; share it with `Arc`.
///
/// It holds leaf keys but not the CA key: the CA key lives exactly as long as the caller's
/// [`ProxyCa`]. `Debug` lists hosts, policies and timeouts and redacts the credential.
pub struct AdmissionConfig {
    approved: BTreeMap<String, ApprovedEntry>,
    auth_hosts: BTreeSet<String>,
    credential: ProxyCredential,
    upstream: Arc<rustls::ClientConfig>,
    admission_timeout: Duration,
    connect_attempt_timeout: Duration,
}

impl std::fmt::Debug for AdmissionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let approved: BTreeMap<&str, &CapturePolicy> = self
            .approved
            .iter()
            .map(|(host, entry)| (host.as_str(), &entry.policy))
            .collect();
        f.debug_struct("AdmissionConfig")
            .field("approved", &approved)
            .field("auth_hosts", &self.auth_hosts)
            .field("credential", &self.credential)
            .field("admission_timeout", &self.admission_timeout)
            .field("connect_attempt_timeout", &self.connect_attempt_timeout)
            .finish_non_exhaustive()
    }
}

impl AdmissionConfig {
    /// Validates the host sets and prepares one harness-facing leaf per approved host from
    /// `ca`. Every host must pass [`is_config_host`]; a host may appear once, and never as
    /// both approved and authentication host. Each approved host must be one `ca` was
    /// generated for. Timeouts start at [`DEFAULT_ADMISSION_TIMEOUT`] and
    /// [`DEFAULT_CONNECT_ATTEMPT_TIMEOUT`]. Performs no I/O.
    pub fn new(
        approved: Vec<ApprovedHost>,
        auth_hosts: Vec<String>,
        credential: ProxyCredential,
        ca: &ProxyCa,
        upstream_roots: &UpstreamRoots,
    ) -> Result<Self, AdmissionConfigError> {
        let mut auth = BTreeSet::new();
        for host in auth_hosts {
            if !is_config_host(&host) {
                return Err(AdmissionConfigError::InvalidHost);
            }
            if !auth.insert(host) {
                return Err(AdmissionConfigError::DuplicateHost);
            }
        }
        let mut entries = BTreeMap::new();
        for ApprovedHost { host, policy } in approved {
            if !is_config_host(&host) {
                return Err(AdmissionConfigError::InvalidHost);
            }
            if auth.contains(&host) {
                return Err(AdmissionConfigError::AmbiguousHost);
            }
            if entries.contains_key(&host) {
                return Err(AdmissionConfigError::DuplicateHost);
            }
            let server = ca.server_config(&host)?;
            entries.insert(host, ApprovedEntry { policy, server });
        }
        Ok(Self {
            approved: entries,
            auth_hosts: auth,
            credential,
            upstream: upstream_roots.client_config()?,
            admission_timeout: DEFAULT_ADMISSION_TIMEOUT,
            connect_attempt_timeout: DEFAULT_CONNECT_ATTEMPT_TIMEOUT,
        })
    }

    /// Replaces both timeouts: `admission` bounds everything before the decision,
    /// `connect_attempt` bounds each upstream address tried.
    pub fn with_timeouts(mut self, admission: Duration, connect_attempt: Duration) -> Self {
        self.admission_timeout = admission;
        self.connect_attempt_timeout = connect_attempt;
        self
    }
}

/// How one accepted connection was settled.
#[derive(Debug)]
pub enum Admission {
    Intercepted(Intercepted),
    Tunneled(Tunneled),
    Failed(AdmissionFailure),
}

/// An admitted interception: both TLS legs are up and negotiated HTTP/1.1 (or no ALPN).
/// `pair.client` is the harness-facing TLS stream, `pair.upstream` the validated
/// provider-facing one. `host` is the normalized CONNECT host (equal to the SNI), `policy`
/// its profile's capture policy, `alpn` the harness-facing negotiated protocol: `None` or
/// `Some("http/1.1")`.
#[derive(Debug)]
pub struct Intercepted {
    pub pair: RelayPair,
    pub host: String,
    pub policy: CapturePolicy,
    pub alpn: Option<String>,
}

/// A completed opaque tunnel: only its target host, why it was not intercepted, and how it
/// ended. No content, header or credential is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tunneled {
    pub host: String,
    pub reason: TunnelReason,
    pub end: TunnelEnd,
}

/// Why a connection was tunnelled rather than intercepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TunnelReason {
    /// The CONNECT host is an authentication host: always opaque.
    AuthHost,
    /// The CONNECT host is not an approved host.
    NotApproved,
    /// The ClientHello carried no server name.
    SniAbsent,
    /// The ClientHello named a server other than the CONNECT host.
    SniMismatch,
    /// The first bytes were not a TLS handshake record.
    NotTls,
    /// The ClientHello was malformed, ended early or exceeded the inspection bound.
    ClientHelloUnparsed,
}

/// How a tunnel ended. Byte counts include the replayed prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelEnd {
    Closed {
        client_to_upstream: u64,
        upstream_to_client: u64,
    },
    Error(std::io::ErrorKind),
}

/// A refused or failed admission. `host` is set only once the caller authenticated and the
/// CONNECT target parsed as a valid host.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("proxy admission failed: {class}")]
pub struct AdmissionFailure {
    pub class: FailureClass,
    pub host: Option<String>,
}

/// The fixed failure classes of admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum FailureClass {
    /// The CONNECT head is not a well-formed HTTP/1.x request head, or its target is not a
    /// `host:port` authority. Answered `400`.
    #[error("malformed request")]
    Malformed,
    /// The CONNECT head exceeded [`MAX_CONNECT_HEAD_BYTES`]. Answered `431`.
    #[error("request head too large")]
    HeadTooLarge,
    /// An authenticated request whose method is not `CONNECT`. Answered `405`.
    #[error("method is not CONNECT")]
    NotConnect,
    /// The proxy credential is missing, repeated or wrong. Answered `407`.
    #[error("proxy authentication required")]
    ProxyAuthRequired,
    /// The admission deadline passed before a decision.
    #[error("admission timed out")]
    Timeout,
    /// The client closed or failed before a decision.
    #[error("client connection failed")]
    ClientIo,
    /// No TCP connection to the target could be made. Answered `502` when still possible.
    #[error("upstream connect failed")]
    UpstreamConnect(ConnectFailureKind),
    /// The upstream TLS handshake or certificate validation failed.
    #[error("upstream TLS failed")]
    UpstreamTls,
    /// The harness-facing TLS handshake failed.
    #[error("client TLS failed")]
    ClientTls,
    /// A leg offered or selected an application protocol other than HTTP/1.1.
    #[error("ALPN incompatible with HTTP/1.1 interception")]
    AlpnIncompatible,
}

/// Fixed, non-secret source of an upstream connection failure. TCP kinds describe only the
/// last resolved address attempted; earlier attempts may have failed differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectFailureKind {
    Dns,
    NoAddresses,
    TcpRefused,
    TcpTimeout,
    TcpPermissionDenied,
    TcpOther,
}

impl ConnectFailureKind {
    fn tcp(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::ConnectionRefused => Self::TcpRefused,
            std::io::ErrorKind::TimedOut => Self::TcpTimeout,
            std::io::ErrorKind::PermissionDenied => Self::TcpPermissionDenied,
            _ => Self::TcpOther,
        }
    }
}

/// Settles one accepted connection. See the module documentation.
///
/// The returned future owns `inbound` (the accepted harness connection) and every socket it
/// opens. Everything up to the intercept/tunnel decision is bounded by the configured
/// admission timeout; a decided tunnel then runs until both directions close. Dropping the
/// future closes exactly the sockets it owns.
pub async fn admit<S: AsyncIo + 'static>(inbound: S, config: &AdmissionConfig) -> Admission {
    admit_observed(inbound, config, |_| {}).await
}

/// [`admit`], calling `on_tunnel` with the reason at the moment a tunnel is decided, before the
/// first byte of it is relayed. The tunnel runs inside this future until both directions close,
/// so a caller that must act when an uncaptured relay *begins* (the proxy chains it as a loss)
/// cannot wait for the return value: a task cancelled or panicking mid-tunnel never gets there.
pub async fn admit_observed<S: AsyncIo + 'static>(
    inbound: S,
    config: &AdmissionConfig,
    on_tunnel: impl FnOnce(TunnelReason),
) -> Admission {
    let mut host = None;
    let deadline = tokio::time::Instant::now() + config.admission_timeout;
    let decision = tokio::time::timeout_at(deadline, negotiate(inbound, config, &mut host)).await;
    match decision {
        Err(_elapsed) => Admission::Failed(AdmissionFailure {
            class: FailureClass::Timeout,
            host,
        }),
        Ok(Decision::Intercept(intercepted)) => Admission::Intercepted(intercepted),
        Ok(Decision::Failed(class)) => Admission::Failed(AdmissionFailure { class, host }),
        Ok(Decision::Tunnel {
            inbound,
            upstream,
            prefix,
            host,
            reason,
        }) => {
            on_tunnel(reason);
            Admission::Tunneled(Tunneled {
                end: run_tunnel(inbound, upstream, prefix).await,
                host,
                reason,
            })
        }
    }
}

/// What the bounded part of admission settled on.
enum Decision<S> {
    Intercept(Intercepted),
    Tunnel {
        inbound: S,
        upstream: TcpStream,
        prefix: Vec<u8>,
        host: String,
        reason: TunnelReason,
    },
    Failed(FailureClass),
}

/// A validated CONNECT target: a normalized host (lowercase, no trailing dot; an IPv6
/// literal without brackets) and a non-zero port.
struct Target {
    host: String,
    port: u16,
}

/// The bounded part of admission. Sets `host_out` once the caller authenticated and the
/// target parsed, so a timeout can still name it.
async fn negotiate<S: AsyncIo + 'static>(
    mut inbound: S,
    config: &AdmissionConfig,
    host_out: &mut Option<String>,
) -> Decision<S> {
    let (mut buf, head_len) = match read_connect_head(&mut inbound).await {
        Ok(read) => read,
        Err(class) => return refuse(&mut inbound, class).await,
    };
    let parsed = parse_connect_head(&buf[..head_len], &config.credential);
    // The head (with the credential) is dropped here; only the bytes after it are kept.
    let prefix = buf.split_off(head_len);
    drop(buf);
    let target = match parsed {
        Ok(target) => target,
        Err(class) => return refuse(&mut inbound, class).await,
    };
    *host_out = Some(target.host.clone());

    // An authentication host is opaque whatever else matches it.
    let auth_host = config.auth_hosts.contains(&target.host);
    let Some(entry) = config.approved.get(&target.host).filter(|_| !auth_host) else {
        let reason = if auth_host {
            TunnelReason::AuthHost
        } else {
            TunnelReason::NotApproved
        };
        let upstream = match connect_upstream(&target, config.connect_attempt_timeout).await {
            Ok(upstream) => upstream,
            Err(kind) => return refuse(&mut inbound, FailureClass::UpstreamConnect(kind)).await,
        };
        if write_established(&mut inbound).await.is_err() {
            return Decision::Failed(FailureClass::ClientIo);
        }
        return Decision::Tunnel {
            inbound,
            upstream,
            prefix,
            host: target.host,
            reason,
        };
    };

    // An approved host: the ClientHello decides, and it only comes after the `200`.
    if write_established(&mut inbound).await.is_err() {
        return Decision::Failed(FailureClass::ClientIo);
    }
    let (prefix, hello) = match peek_client_hello(&mut inbound, prefix).await {
        Ok(peeked) => peeked,
        Err(_) => return Decision::Failed(FailureClass::ClientIo),
    };
    let reason = match hello {
        HelloParse::Sni(Some(name)) if normalize_sni(&name) == target.host => {
            return intercept(inbound, prefix, target, entry, config).await;
        }
        HelloParse::Sni(Some(_)) => TunnelReason::SniMismatch,
        HelloParse::Sni(None) => TunnelReason::SniAbsent,
        HelloParse::NotTls => TunnelReason::NotTls,
        HelloParse::NeedMore | HelloParse::Malformed => TunnelReason::ClientHelloUnparsed,
    };
    match connect_upstream(&target, config.connect_attempt_timeout).await {
        Ok(upstream) => Decision::Tunnel {
            inbound,
            upstream,
            prefix,
            host: target.host,
            reason,
        },
        Err(kind) => Decision::Failed(FailureClass::UpstreamConnect(kind)),
    }
}

/// Validates upstream TLS first, then completes the harness-facing handshake over the
/// replayed ClientHello bytes.
async fn intercept<S: AsyncIo + 'static>(
    inbound: S,
    prefix: Vec<u8>,
    target: Target,
    entry: &ApprovedEntry,
    config: &AdmissionConfig,
) -> Decision<S> {
    let tcp = match connect_upstream(&target, config.connect_attempt_timeout).await {
        Ok(tcp) => tcp,
        Err(kind) => return Decision::Failed(FailureClass::UpstreamConnect(kind)),
    };
    let Ok(name) = rustls::pki_types::ServerName::try_from(target.host.clone()) else {
        return Decision::Failed(FailureClass::UpstreamTls);
    };
    let upstream = match tokio_rustls::TlsConnector::from(config.upstream.clone())
        .connect(name, tcp)
        .await
    {
        Ok(upstream) => upstream,
        Err(e) => return Decision::Failed(tls_failure(&e, FailureClass::UpstreamTls)),
    };
    if !alpn_is_http11(upstream.get_ref().1.alpn_protocol()) {
        return Decision::Failed(FailureClass::AlpnIncompatible);
    }
    let harness = match tokio_rustls::TlsAcceptor::from(entry.server.clone())
        .accept(PrefixedIo::new(prefix, inbound))
        .await
    {
        Ok(harness) => harness,
        Err(e) => return Decision::Failed(tls_failure(&e, FailureClass::ClientTls)),
    };
    let alpn = harness.get_ref().1.alpn_protocol();
    if !alpn_is_http11(alpn) {
        return Decision::Failed(FailureClass::AlpnIncompatible);
    }
    let alpn = alpn.map(|protocol| String::from_utf8_lossy(protocol).into_owned());
    Decision::Intercept(Intercepted {
        pair: RelayPair {
            client: Box::new(harness),
            upstream: Box::new(upstream),
        },
        host: target.host,
        policy: entry.policy.clone(),
        alpn,
    })
}

/// A handshake error's class: an ALPN refusal in either direction is
/// [`FailureClass::AlpnIncompatible`], anything else `otherwise`.
fn tls_failure(error: &std::io::Error, otherwise: FailureClass) -> FailureClass {
    let alpn_refused = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|e| {
            matches!(
                e,
                rustls::Error::NoApplicationProtocol
                    | rustls::Error::AlertReceived(rustls::AlertDescription::NoApplicationProtocol)
            )
        });
    if alpn_refused {
        FailureClass::AlpnIncompatible
    } else {
        otherwise
    }
}

/// Relays a decided tunnel until both directions close, starting with the bytes already
/// read from the client.
async fn run_tunnel<S: AsyncIo>(
    mut inbound: S,
    mut upstream: TcpStream,
    prefix: Vec<u8>,
) -> TunnelEnd {
    let relayed = async {
        upstream.write_all(&prefix).await?;
        tokio::io::copy_bidirectional(&mut inbound, &mut upstream).await
    }
    .await;
    match relayed {
        Ok((client_to_upstream, upstream_to_client)) => TunnelEnd::Closed {
            client_to_upstream: client_to_upstream + prefix.len() as u64,
            upstream_to_client,
        },
        Err(e) => TunnelEnd::Error(e.kind()),
    }
}

/// Answers a refusal with its fixed response, if it has one, and settles as failed.
async fn refuse<S: AsyncIo>(inbound: &mut S, class: FailureClass) -> Decision<S> {
    let response: Option<&[u8]> = match class {
        FailureClass::Malformed => Some(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
        FailureClass::HeadTooLarge => Some(
            b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
        FailureClass::NotConnect => Some(
            b"HTTP/1.1 405 Method Not Allowed\r\nAllow: CONNECT\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
        FailureClass::ProxyAuthRequired => Some(
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goalcup\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
        FailureClass::UpstreamConnect(_) => Some(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
        _ => None,
    };
    if let Some(response) = response
        && inbound.write_all(response).await.is_ok()
    {
        let _ = inbound.shutdown().await;
    }
    Decision::Failed(class)
}

async fn write_established<S: AsyncIo>(inbound: &mut S) -> std::io::Result<()> {
    inbound
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    inbound.flush().await
}

/// Reads until the first blank line, into at most [`MAX_CONNECT_HEAD_BYTES`]. Returns the
/// buffer (head plus any bytes already read after it) and the head's length.
async fn read_connect_head<S: AsyncIo>(inbound: &mut S) -> Result<(Vec<u8>, usize), FailureClass> {
    let mut buf = vec![0u8; MAX_CONNECT_HEAD_BYTES];
    let mut filled = 0;
    loop {
        if filled == buf.len() {
            return Err(FailureClass::HeadTooLarge);
        }
        let n = inbound
            .read(&mut buf[filled..])
            .await
            .map_err(|_| FailureClass::ClientIo)?;
        if n == 0 {
            return Err(FailureClass::ClientIo);
        }
        let search_from = filled.saturating_sub(3);
        filled += n;
        if let Some(at) = buf[search_from..filled]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            buf.truncate(filled);
            return Ok((buf, search_from + at + 4));
        }
    }
}

/// Parses a complete head (ending in the blank line): structure first, then the
/// credential, then the method, then the target. Never echoes any of it.
fn parse_connect_head(head: &[u8], credential: &ProxyCredential) -> Result<Target, FailureClass> {
    // Each line keeps its own CRLF; the blank line's CRLF is dropped.
    let body = &head[..head.len() - 2];
    let mut lines = body
        .split_inclusive(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r\n"));
    let request_line = lines.next().flatten().ok_or(FailureClass::Malformed)?;
    let mut parts = request_line.split(|&b| b == b' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(FailureClass::Malformed);
    };
    if method.is_empty() || target.is_empty() || !matches!(version, b"HTTP/1.1" | b"HTTP/1.0") {
        return Err(FailureClass::Malformed);
    }
    let mut authorization = None;
    let mut authorizations = 0;
    for line in lines {
        let line = line.ok_or(FailureClass::Malformed)?;
        let colon = line
            .iter()
            .position(|&b| b == b':')
            .ok_or(FailureClass::Malformed)?;
        let (name, value) = (&line[..colon], trim_ows(&line[colon + 1..]));
        if name.is_empty()
            || !name.iter().all(|&b| is_token_byte(b))
            || value.iter().any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f)
        {
            return Err(FailureClass::Malformed);
        }
        if name.eq_ignore_ascii_case(b"proxy-authorization") {
            authorizations += 1;
            authorization = Some(value);
        }
    }
    match authorization {
        Some(value) if authorizations == 1 && credential.accepts(value) => {}
        _ => return Err(FailureClass::ProxyAuthRequired),
    }
    if method != b"CONNECT" {
        return Err(FailureClass::NotConnect);
    }
    parse_authority(target).ok_or(FailureClass::Malformed)
}

/// RFC 4648 base64 with padding: the encoding clients use for Basic credentials.
fn base64_standard(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);
        let sextet = |shift: u32| char::from(ALPHABET[(n >> shift) as usize & 63]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

fn trim_ows(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t')
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}

/// RFC 9110 `tchar`.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// `host:port` or `[ipv6]:port`, nothing else: no userinfo, path, query or fragment.
fn parse_authority(target: &[u8]) -> Option<Target> {
    let target = std::str::from_utf8(target).ok()?;
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (literal, port) = rest.split_once("]:")?;
        (
            literal.parse::<std::net::Ipv6Addr>().ok()?.to_string(),
            port,
        )
    } else {
        let (host, port) = target.rsplit_once(':')?;
        (normalize_connect_host(host)?, port)
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    (port != 0).then_some(Target { host, port })
}

/// Lowercases a CONNECT host and drops one trailing dot. Accepts DNS-style names (letters,
/// digits, hyphen, underscore) and IPv4 literals; refuses anything else.
fn normalize_connect_host(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        });
    valid.then(|| host.to_owned())
}

/// A ClientHello server name as compared with the CONNECT host.
fn normalize_sni(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    name.strip_suffix('.').map(str::to_owned).unwrap_or(name)
}

/// Resolves the target and tries each address in turn, IPv4 first, each attempt bounded by
/// `attempt` so one dead route cannot stall admission.
async fn connect_upstream(
    target: &Target,
    attempt: Duration,
) -> Result<TcpStream, ConnectFailureKind> {
    let mut addresses: Vec<std::net::SocketAddr> =
        tokio::net::lookup_host((target.host.as_str(), target.port))
            .await
            .map_err(|_| ConnectFailureKind::Dns)?
            .collect();
    addresses.sort_by_key(|address| address.is_ipv6());
    connect_addresses(addresses, attempt).await
}

async fn connect_addresses(
    addresses: Vec<std::net::SocketAddr>,
    attempt: Duration,
) -> Result<TcpStream, ConnectFailureKind> {
    let mut last = ConnectFailureKind::NoAddresses;
    for address in addresses {
        match tokio::time::timeout(attempt, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(e)) => last = ConnectFailureKind::tcp(e.kind()),
            Err(_elapsed) => last = ConnectFailureKind::TcpTimeout,
        }
    }
    Err(last)
}

/// Reads until the ClientHello is complete (or cannot be), into at most
/// [`MAX_CLIENT_HELLO_PEEK_BYTES`]. Returns every byte read — `prefix` included — so the
/// chosen path can replay them.
async fn peek_client_hello<S: AsyncIo>(
    inbound: &mut S,
    mut buf: Vec<u8>,
) -> std::io::Result<(Vec<u8>, HelloParse)> {
    let mut chunk = [0u8; 4096];
    loop {
        match parse_client_hello(&buf) {
            HelloParse::NeedMore if buf.len() < MAX_CLIENT_HELLO_PEEK_BYTES => {}
            parsed => return Ok((buf, parsed)),
        }
        let room = (MAX_CLIENT_HELLO_PEEK_BYTES - buf.len()).min(chunk.len());
        let n = inbound.read(&mut chunk[..room]).await?;
        if n == 0 {
            return Ok((buf, HelloParse::NeedMore));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// What the bytes so far say about the ClientHello.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HelloParse {
    /// Not enough bytes yet (or, once returned from the peek, never enough).
    NeedMore,
    /// The first record is not a TLS handshake record.
    NotTls,
    /// A TLS handshake that is not a well-formed ClientHello.
    Malformed,
    /// A complete ClientHello, with its `host_name` server name if it carries one.
    Sni(Option<String>),
}

const TLS_HANDSHAKE: u8 = 22;
const TLS_MAX_RECORD: usize = 16 * 1024;
const CLIENT_HELLO: u8 = 1;
const EXT_SERVER_NAME: u16 = 0;

/// Reassembles the handshake stream from consecutive handshake records and parses the
/// ClientHello once it is complete. Pure: callable on any prefix.
fn parse_client_hello(bytes: &[u8]) -> HelloParse {
    let mut handshake = Vec::new();
    let mut offset = 0;
    loop {
        let Some(header) = bytes.get(offset..offset + 5) else {
            if offset == 0 && bytes.first().is_some_and(|&b| b != TLS_HANDSHAKE) {
                return HelloParse::NotTls;
            }
            return HelloParse::NeedMore;
        };
        if header[0] != TLS_HANDSHAKE || header[1] != 3 {
            return if offset == 0 {
                HelloParse::NotTls
            } else {
                HelloParse::Malformed
            };
        }
        let len = usize::from(u16::from_be_bytes([header[3], header[4]]));
        if len == 0 || len > TLS_MAX_RECORD {
            return HelloParse::Malformed;
        }
        let Some(fragment) = bytes.get(offset + 5..offset + 5 + len) else {
            return HelloParse::NeedMore;
        };
        handshake.extend_from_slice(fragment);
        offset += 5 + len;
        if handshake.len() >= 4 {
            if handshake[0] != CLIENT_HELLO {
                return HelloParse::Malformed;
            }
            let body = usize::from(handshake[1]) << 16
                | usize::from(handshake[2]) << 8
                | usize::from(handshake[3]);
            if body > MAX_CLIENT_HELLO_PEEK_BYTES {
                return HelloParse::Malformed;
            }
            if let Some(body) = handshake.get(4..4 + body) {
                return parse_client_hello_body(body).unwrap_or(HelloParse::Malformed);
            }
        }
    }
}

/// The ClientHello body: version, random, session ID, cipher suites, compression methods,
/// then (optionally) extensions, the `server_name` one parsed per RFC 6066.
fn parse_client_hello_body(body: &[u8]) -> Option<HelloParse> {
    let mut r = Reader(body);
    r.take(2 + 32)?;
    let session = r.u8()?;
    if session > 32 {
        return None;
    }
    r.take(usize::from(session))?;
    let suites = r.u16()?;
    if suites == 0 || suites % 2 != 0 {
        return None;
    }
    r.take(usize::from(suites))?;
    let compression = r.u8()?;
    if compression == 0 {
        return None;
    }
    r.take(usize::from(compression))?;
    if r.0.is_empty() {
        return Some(HelloParse::Sni(None));
    }
    let extensions = usize::from(r.u16()?);
    if extensions != r.0.len() {
        return None;
    }
    let mut server_name = None;
    let mut seen_server_name = false;
    while !r.0.is_empty() {
        let kind = r.u16()?;
        let len = r.u16()?;
        let data = r.take(usize::from(len))?;
        if kind == EXT_SERVER_NAME {
            if seen_server_name {
                return None;
            }
            seen_server_name = true;
            server_name = parse_server_name(data)?;
        }
    }
    Some(HelloParse::Sni(server_name))
}

/// A `server_name` extension body: one `host_name` at most, non-empty ASCII.
fn parse_server_name(data: &[u8]) -> Option<Option<String>> {
    let mut r = Reader(data);
    let list = usize::from(r.u16()?);
    if list == 0 || list != r.0.len() {
        return None;
    }
    let mut host = None;
    while !r.0.is_empty() {
        let kind = r.u8()?;
        let len = r.u16()?;
        let name = r.take(usize::from(len))?;
        if kind == 0 {
            if host.is_some() || name.is_empty() || !name.is_ascii() {
                return None;
            }
            host = Some(String::from_utf8(name.to_vec()).ok()?);
        }
    }
    Some(host)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
}

/// A client stream that first yields the bytes admission already read, then the socket.
/// Writes go straight to the socket.
struct PrefixedIo<S> {
    prefix: Vec<u8>,
    read: usize,
    inner: S,
}

impl<S> PrefixedIo<S> {
    fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix,
            read: 0,
            inner,
        }
    }
}

impl<S: AsyncIo> AsyncRead for PrefixedIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if this.read < this.prefix.len() {
            let n = (this.prefix.len() - this.read).min(buf.remaining());
            buf.put_slice(&this.prefix[this.read..this.read + n]);
            this.read += n;
            if this.read == this.prefix.len() {
                this.prefix = Vec::new();
                this.read = 0;
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncIo> AsyncWrite for PrefixedIo<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_resolution_and_terminal_tcp_errors_keep_fixed_kinds() {
        assert!(matches!(
            connect_addresses(Vec::new(), Duration::from_millis(1)).await,
            Err(ConnectFailureKind::NoAddresses)
        ));
        assert_eq!(
            ConnectFailureKind::tcp(std::io::ErrorKind::ConnectionRefused),
            ConnectFailureKind::TcpRefused
        );
        assert_eq!(
            ConnectFailureKind::tcp(std::io::ErrorKind::TimedOut),
            ConnectFailureKind::TcpTimeout
        );
        assert_eq!(
            ConnectFailureKind::tcp(std::io::ErrorKind::PermissionDenied),
            ConnectFailureKind::TcpPermissionDenied
        );
        assert_eq!(
            ConnectFailureKind::tcp(std::io::ErrorKind::Other),
            ConnectFailureKind::TcpOther
        );
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_standard(input.as_bytes()), expected);
        }
    }

    /// A minimal well-formed ClientHello body with the given extensions block (or none).
    fn hello_body(extensions: Option<Vec<u8>>) -> Vec<u8> {
        let mut body = vec![3, 3];
        body.extend([7u8; 32]);
        body.push(0);
        body.extend([0, 2, 0x13, 0x01]);
        body.extend([1, 0]);
        if let Some(extensions) = extensions {
            body.extend((extensions.len() as u16).to_be_bytes());
            body.extend(extensions);
        }
        body
    }

    fn server_name_ext(names: &[(u8, &[u8])]) -> Vec<u8> {
        let mut list = Vec::new();
        for (kind, name) in names {
            list.push(*kind);
            list.extend((name.len() as u16).to_be_bytes());
            list.extend(*name);
        }
        let mut data = (list.len() as u16).to_be_bytes().to_vec();
        data.extend(list);
        let mut ext = EXT_SERVER_NAME.to_be_bytes().to_vec();
        ext.extend((data.len() as u16).to_be_bytes());
        ext.extend(data);
        ext
    }

    fn record(body: &[u8]) -> Vec<u8> {
        let mut handshake = vec![CLIENT_HELLO];
        handshake.extend(&(body.len() as u32).to_be_bytes()[1..]);
        handshake.extend(body);
        let mut out = vec![TLS_HANDSHAKE, 3, 1];
        out.extend((handshake.len() as u16).to_be_bytes());
        out.extend(handshake);
        out
    }

    #[test]
    fn client_hello_parse_covers_each_verdict() {
        let named = record(&hello_body(Some(server_name_ext(&[(0, b"Api.Test.")]))));
        assert_eq!(
            parse_client_hello(&named),
            HelloParse::Sni(Some("Api.Test.".into()))
        );
        assert_eq!(normalize_sni("Api.Test."), "api.test");
        for cut in 0..named.len() {
            assert_eq!(
                parse_client_hello(&named[..cut]),
                HelloParse::NeedMore,
                "cut at {cut}"
            );
        }
        assert_eq!(
            parse_client_hello(&record(&hello_body(None))),
            HelloParse::Sni(None)
        );
        assert_eq!(
            parse_client_hello(b"GET / HTTP/1.1\r\n"),
            HelloParse::NotTls
        );
        assert_eq!(
            parse_client_hello(&[0x17, 3, 3, 0, 1, 0]),
            HelloParse::NotTls
        );
        let twice = server_name_ext(&[(0, b"a.test"), (0, b"b.test")]);
        assert_eq!(
            parse_client_hello(&record(&hello_body(Some(twice)))),
            HelloParse::Malformed
        );
        let mut duplicate_ext = server_name_ext(&[(0, b"a.test")]);
        duplicate_ext.extend(server_name_ext(&[(0, b"a.test")]));
        assert_eq!(
            parse_client_hello(&record(&hello_body(Some(duplicate_ext)))),
            HelloParse::Malformed
        );
        let mut short = hello_body(Some(server_name_ext(&[(0, b"a.test")])));
        short.pop();
        assert_eq!(parse_client_hello(&record(&short)), HelloParse::Malformed);
        // A declared ClientHello larger than the inspection bound is refused up front.
        let mut huge = vec![TLS_HANDSHAKE, 3, 1, 0, 4, CLIENT_HELLO];
        huge.extend(&((MAX_CLIENT_HELLO_PEEK_BYTES as u32 + 1).to_be_bytes()[1..]));
        assert_eq!(parse_client_hello(&huge), HelloParse::Malformed);
        // A second record that is not handshake, mid-ClientHello.
        let mut split = named[..5].to_vec();
        split[4] = 10;
        split.extend(&named[5..15]);
        split.extend([0x17, 3, 3, 0, 1, 0]);
        assert_eq!(parse_client_hello(&split), HelloParse::Malformed);
    }

    #[test]
    fn authority_accepts_only_host_and_port() {
        let ok = |t: &str| parse_authority(t.as_bytes()).map(|t| (t.host, t.port));
        assert_eq!(ok("API.Test.:443"), Some(("api.test".into(), 443)));
        assert_eq!(ok("127.0.0.1:8443"), Some(("127.0.0.1".into(), 8443)));
        assert_eq!(ok("[::1]:443"), Some(("::1".into(), 443)));
        for bad in [
            "api.test",
            "api.test:",
            "api.test:0",
            "api.test:65536",
            "api.test:+443",
            "u:p@api.test:443",
            "api.test:443/x",
            "api.test/x:443",
            "[::1]",
            "::1:443",
            "[::1%lo0]:443",
            "a..test:443",
            ":443",
            "api test:443",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }
}
