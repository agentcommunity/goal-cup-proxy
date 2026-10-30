//! TLS configs and handshake error classes (PRD-03 section 4 points 4 to 6).
//!
//! ALPN prefers HTTP/1.1 (`PreferH1`, the only mode in 1.0): upstream is offered a list built
//! from the client's own offer, and whatever upstream picks is the only protocol offered back to
//! the client, so both legs speak one protocol and bytes pass unchanged. Configs are built once
//! per offer and per chosen protocol, never per connection: three upstream client configs over
//! the Mozilla roots plus the OS store, and three harness-facing server configs sharing the run
//! CA's leaf resolver, which picks the leaf by SNI. A client that sends no SNI gets a config of its
//! own from [`Tls::server_for`], bound to the CONNECT host (section 2). The ring provider is named
//! on every builder (PRD-17 T8).

use crate::proxy::ca::{CaError, RunCa, os_roots, ring_provider};
use rustls::pki_types::CertificateDer;
use rustls::server::ResolvesServerCert;
use rustls::{AlertDescription, ClientConfig, Error, PeerMisbehaved, RootCertStore, ServerConfig};
use std::fmt;
use std::io;
use std::sync::Arc;

const H1: &[u8] = b"http/1.1";
const H2: &[u8] = b"h2";

/// What upstream is offered, from the client's ALPN list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// The client offers `http/1.1`, with or without `h2`: offer `http/1.1` only.
    Http1,
    /// The client offers `h2` and not `http/1.1`: offer `h2` only.
    H2,
    /// The client offers no ALPN: offer none, parse as HTTP/1.1.
    None,
    /// The client offers ALPN with neither: tunnel, GAP `protocol_other` (`alpn_other`).
    Other,
}

/// The `PreferH1` rule of section 4 point 4.
pub fn upstream_offer(client_alpn: &[Vec<u8>]) -> Offer {
    if client_alpn.is_empty() {
        Offer::None
    } else if client_alpn.iter().any(|p| p == H1) {
        Offer::Http1
    } else if client_alpn.iter().any(|p| p == H2) {
        Offer::H2
    } else {
        Offer::Other
    }
}

/// The six configs, built once per boot and shared.
pub struct Tls {
    ca: RunCa,
    up_h1: Arc<ClientConfig>,
    up_h2: Arc<ClientConfig>,
    up_none: Arc<ClientConfig>,
    down_h1: Arc<ServerConfig>,
    down_h2: Arc<ServerConfig>,
    down_none: Arc<ServerConfig>,
}

impl fmt::Debug for Tls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tls").finish_non_exhaustive()
    }
}

impl Tls {
    /// Upstream roots are `webpki-roots`, the OS store and `extra_roots` (a test hook's, or
    /// none). Leaves come from `ca`.
    pub fn new(ca: &RunCa, extra_roots: Vec<CertificateDer<'static>>) -> Tls {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        roots.add_parsable_certificates(os_roots());
        roots.add_parsable_certificates(extra_roots);
        let roots = Arc::new(roots);
        let client = |alpn: &[&[u8]]| {
            let mut config = ClientConfig::builder_with_provider(ring_provider())
                .with_safe_default_protocol_versions()
                .expect("the ring provider supports the default TLS versions")
                .with_root_certificates(roots.clone())
                .with_no_client_auth();
            config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
            Arc::new(config)
        };
        let resolver = ca.resolver();
        let server = |alpn: &[u8]| server_config(resolver.clone(), Some(alpn));
        Tls {
            ca: ca.clone(),
            up_h1: client(&[H1]),
            up_h2: client(&[H2]),
            up_none: client(&[]),
            down_h1: server(H1),
            down_h2: server(H2),
            down_none: server_config(resolver.clone(), None),
        }
    }

    /// The upstream config for `offer`; none for [`Offer::Other`], which tunnels.
    pub fn client(&self, offer: Offer) -> Option<Arc<ClientConfig>> {
        match offer {
            Offer::Http1 => Some(self.up_h1.clone()),
            Offer::H2 => Some(self.up_h2.clone()),
            Offer::None => Some(self.up_none.clone()),
            Offer::Other => None,
        }
    }

    /// The harness-facing config offering exactly what upstream chose (`None`: no ALPN), for a
    /// ClientHello that carries SNI.
    pub fn server(&self, chosen_alpn: Option<&[u8]>) -> Arc<ServerConfig> {
        match chosen_alpn {
            Some(H1) => self.down_h1.clone(),
            Some(H2) => self.down_h2.clone(),
            _ => self.down_none.clone(),
        }
    }

    /// As [`Tls::server`], for a ClientHello without SNI: the leaf is for `connect_host`. Built
    /// per such connection, which is rare.
    pub fn server_for(
        &self,
        chosen_alpn: Option<&[u8]>,
        connect_host: &str,
    ) -> Result<Arc<ServerConfig>, CaError> {
        let alpn = chosen_alpn.filter(|p| *p == H1 || *p == H2);
        Ok(server_config(self.ca.resolver_for(connect_host)?, alpn))
    }
}

/// A harness-facing config offering `alpn` alone, or no ALPN.
fn server_config(resolver: Arc<dyn ResolvesServerCert>, alpn: Option<&[u8]>) -> Arc<ServerConfig> {
    let mut config = ServerConfig::builder_with_provider(ring_provider())
        .with_safe_default_protocol_versions()
        .expect("the ring provider supports the default TLS versions")
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = alpn.map(|p| vec![p.to_vec()]).unwrap_or_default();
    Arc::new(config)
}

/// How the harness ended a handshake on our leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// An explicit certificate alert, by its TLS name: the only case refusal learning uses.
    Alert(&'static str),
    /// EOF, reset, timeout or anything else: GAP `tls_refused` with tail `{"alert": false}`,
    /// nothing learned.
    NoAlert,
}

/// Classifies the error of the harness-facing handshake.
pub fn classify_client_handshake(err: &io::Error) -> Refusal {
    let name = match rustls_error(err) {
        Some(Error::AlertReceived(alert)) => match alert {
            AlertDescription::UnknownCA => "unknown_ca",
            AlertDescription::BadCertificate => "bad_certificate",
            AlertDescription::CertificateUnknown => "certificate_unknown",
            AlertDescription::UnsupportedCertificate => "unsupported_certificate",
            AlertDescription::AccessDenied => "access_denied",
            _ => return Refusal::NoAlert,
        },
        _ => return Refusal::NoAlert,
    };
    Refusal::Alert(name)
}

/// Why the upstream handshake failed. Every failure falls back to a working tunnel on a fresh
/// TCP connection (section 1 rule 3), never a dropped harness connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamFail {
    /// Upstream rejected the offered ALPN: a tunnel with `alpn_other`, GAP 14 `protocol_other`.
    AlpnRejected,
    /// Everything else: a certificate that does not validate, a client-certificate demand (TLS
    /// 1.3 `certificate_required`, or under TLS 1.2 usually `handshake_failure` or
    /// `bad_certificate` after our empty certificate), a reset, an EOF, a version or cipher
    /// mismatch. A tunnel with `upstream_untrusted`, GAP 13: the harness then meets the real
    /// server itself, so trust is never lowered.
    Untrusted,
}

/// Classifies the error of the upstream handshake.
pub fn classify_upstream_handshake(err: &io::Error) -> UpstreamFail {
    match rustls_error(err) {
        Some(
            Error::AlertReceived(AlertDescription::NoApplicationProtocol)
            | Error::PeerMisbehaved(PeerMisbehaved::SelectedUnofferedApplicationProtocol),
        ) => UpstreamFail::AlpnRejected,
        _ => UpstreamFail::Untrusted,
    }
}

/// The rustls error inside an I/O error, as tokio-rustls and rustls' own streams wrap it.
fn rustls_error(err: &io::Error) -> Option<&Error> {
    err.get_ref()?.downcast_ref::<Error>()
}
