//! TLS termination toward the harness with per-launch, process-scoped CA and
//! leaf certificates, and validated upstream TLS.
//!
//! # What this module owns
//!
//! - [`ProxyCa`]: one CA per proxy launch. Its signing key is generated in memory and never
//!   leaves it: nothing here serializes, writes or logs it, and it is freed when the caller
//!   drops the `ProxyCa`. Only the CA **certificate** is written, by
//!   [`ProxyCa::write_certificate`], as a new owner-only (0600) file inside a directory the
//!   caller names (the harness's scoped home). That file is the process-scoped trust the
//!   harness is pointed at (`CODEX_CA_CERTIFICATE`, `NODE_EXTRA_CA_CERTS`); nothing here
//!   touches the operating system's trust store or any app-wide configuration. The file
//!   outlives the `ProxyCa`: without the key it can sign nothing, and removing it belongs to
//!   whoever owns the scoped home.
//! - The CA is constrained: `pathLen 0`, and a critical NameConstraints extension that permits
//!   exactly the approved (intercepted) host names, so even a leaked key could not mint a
//!   certificate a validating client accepts for any other name. It is valid from
//!   [`CA_CLOCK_SKEW`] before generation until [`CA_VALIDITY`] after it. Leaves share that
//!   window and carry the Authority Key Identifier (Python 3.13 rejects leaves without it),
//!   one DNS SAN and the serverAuth EKU.
//! - [`UpstreamRoots`]: the trust anchors upstream certificates are validated against. The
//!   production default is [`UpstreamRoots::webpki`] (the Mozilla set); a synthetic local test
//!   injects its own roots with [`UpstreamRoots::from_certificates`]. There is no
//!   verification bypass: every upstream handshake checks the chain and the host name.
//! - The ALPN policy: interception is HTTP/1.1 only, on both legs. The harness-facing server
//!   offers only `http/1.1`, so an `h2`-only client is refused with a `no_application_protocol`
//!   alert, a client offering `h2` and `http/1.1` gets `http/1.1`, and a client offering no
//!   ALPN at all (WebSocket-style) is accepted as HTTP/1.1. The upstream client offers only
//!   `http/1.1`. No HTTP/2 path exists.
//!
//! The CONNECT admission that uses these pieces is [`super::connect`].

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The one application protocol interception negotiates, on both TLS legs.
pub(crate) const HTTP11_ALPN: &[u8] = b"http/1.1";

/// The file name [`ProxyCa::write_certificate`] creates inside the caller's directory.
pub const CA_CERT_FILE_NAME: &str = "goalcup-proxy-ca.pem";

/// How long a per-launch CA (and every leaf it issues) stays valid after generation: the
/// 168-hour exercise plus a day. A launch that outlives it needs a new proxy, which mints a
/// new CA.
pub const CA_VALIDITY: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// How far before generation the validity window starts, so a slightly slow client clock
/// still accepts a fresh certificate.
pub(crate) const CA_CLOCK_SKEW: Duration = Duration::from_secs(60 * 60);

/// Why CA, leaf or trust-root preparation failed. Messages are fixed text: no host, path or
/// key material is ever formatted into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaError {
    #[error("a proxy CA needs at least one approved host")]
    NoApprovedHosts,
    #[error("an approved host is not a lowercase DNS name without port or trailing dot")]
    InvalidHost,
    #[error("the host is not one this CA was generated for")]
    HostNotPermitted,
    #[error("certificate or key generation failed")]
    Generate,
    #[error("the TLS configuration could not be built")]
    TlsConfig,
    #[error("an upstream trust root is not a usable certificate")]
    InvalidRoot,
    #[error("the CA certificate could not be written ({0:?})")]
    Write(std::io::ErrorKind),
}

/// A per-launch certificate authority for the approved hosts. See the module documentation
/// for its key lifetime and file semantics. `Debug` shows the permitted host names only.
pub struct ProxyCa {
    params: CertificateParams,
    key: KeyPair,
    cert_der: CertificateDer<'static>,
    cert_pem: String,
    hosts: BTreeSet<String>,
}

impl std::fmt::Debug for ProxyCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyCa")
            .field("hosts", &self.hosts)
            .finish_non_exhaustive()
    }
}

impl ProxyCa {
    /// Generates a fresh CA key and self-signed certificate whose NameConstraints permit
    /// exactly `approved_hosts`, each of which must pass [`is_config_host`]. Performs no I/O.
    pub fn generate(approved_hosts: &[String]) -> Result<Self, CaError> {
        if approved_hosts.is_empty() {
            return Err(CaError::NoApprovedHosts);
        }
        let mut hosts = BTreeSet::new();
        for host in approved_hosts {
            if !is_config_host(host) {
                return Err(CaError::InvalidHost);
            }
            hosts.insert(host.clone());
        }
        let mut params = CertificateParams::default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let epoch = rcgen::date_time_ymd(1970, 1, 1);
        params.not_before = epoch + now.saturating_sub(CA_CLOCK_SKEW);
        params.not_after = epoch + now + CA_VALIDITY;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Goal Cup per-launch proxy CA");
        name.push(DnType::OrganizationName, "Goal Cup");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.name_constraints = Some(NameConstraints {
            permitted_subtrees: hosts
                .iter()
                .map(|h| GeneralSubtree::DnsName(h.clone()))
                .collect(),
            excluded_subtrees: Vec::new(),
        });
        let key = KeyPair::generate().map_err(|_| CaError::Generate)?;
        let cert = params.self_signed(&key).map_err(|_| CaError::Generate)?;
        Ok(Self {
            cert_pem: cert.pem(),
            cert_der: cert.der().clone(),
            params,
            key,
            hosts,
        })
    }

    /// The CA certificate, PEM-encoded: what a harness is told to trust.
    #[cfg(any(test, feature = "test-support"))]
    pub fn certificate_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The CA certificate, DER-encoded.
    pub fn certificate_der(&self) -> &CertificateDer<'static> {
        &self.cert_der
    }

    /// Creates `dir/`[`CA_CERT_FILE_NAME`] containing only the PEM certificate, with mode
    /// 0600, and returns its path. The directory must already exist. An existing entry at
    /// that path — a file or a symlink — is refused rather than replaced or followed. Writes
    /// nothing anywhere else.
    pub fn write_certificate(&self, dir: &Path) -> Result<PathBuf, CaError> {
        let path = dir.join(CA_CERT_FILE_NAME);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| CaError::Write(e.kind()))?;
        file.write_all(self.cert_pem.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| CaError::Write(e.kind()))?;
        Ok(path)
    }

    /// A harness-facing TLS server configuration for `host`: a fresh leaf key and a leaf
    /// certificate signed by this CA, offering only `http/1.1` over ALPN.
    pub(crate) fn server_config(&self, host: &str) -> Result<Arc<ServerConfig>, CaError> {
        if !self.hosts.contains(host) {
            return Err(CaError::HostNotPermitted);
        }
        let mut params = CertificateParams::default();
        params.not_before = self.params.not_before;
        params.not_after = self.params.not_after;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.subject_alt_names = vec![SanType::DnsName(
            host.to_owned()
                .try_into()
                .map_err(|_| CaError::InvalidHost)?,
        )];
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate().map_err(|_| CaError::Generate)?;
        let issuer = Issuer::from_params(&self.params, &self.key);
        let leaf = params
            .signed_by(&key, &issuer)
            .map_err(|_| CaError::Generate)?;
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let mut config = ServerConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()
            .map_err(|_| CaError::TlsConfig)?
            .with_no_client_auth()
            .with_single_cert(vec![leaf.der().clone()], key_der)
            .map_err(|_| CaError::TlsConfig)?;
        config.alpn_protocols = vec![HTTP11_ALPN.to_vec()];
        Ok(Arc::new(config))
    }
}

/// The trust anchors upstream (provider-facing) certificates are validated against.
#[derive(Clone)]
pub struct UpstreamRoots(Arc<RootCertStore>);

impl std::fmt::Debug for UpstreamRoots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamRoots")
            .field("anchors", &self.0.len())
            .finish()
    }
}

impl UpstreamRoots {
    /// The production default: the Mozilla root set compiled into `webpki-roots`.
    pub fn webpki() -> Self {
        let mut store = RootCertStore::empty();
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Self(Arc::new(store))
    }

    /// Exactly the given roots, for a synthetic local upstream. Validation still checks the
    /// chain and the host name; this only changes which anchors are trusted.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_certificates(
        roots: impl IntoIterator<Item = CertificateDer<'static>>,
    ) -> Result<Self, CaError> {
        let mut store = RootCertStore::empty();
        for root in roots {
            store.add(root).map_err(|_| CaError::InvalidRoot)?;
        }
        if store.is_empty() {
            return Err(CaError::InvalidRoot);
        }
        Ok(Self(Arc::new(store)))
    }

    /// These roots and `extra` as well, for a synthetic local upstream beside the public ones.
    /// A test build's only; a build without `test-support` has none of it.
    #[cfg(feature = "test-support")]
    pub fn with_extra(
        self,
        extra: impl IntoIterator<Item = CertificateDer<'static>>,
    ) -> Result<Self, CaError> {
        let mut store = (*self.0).clone();
        for root in extra {
            store.add(root).map_err(|_| CaError::InvalidRoot)?;
        }
        Ok(Self(Arc::new(store)))
    }

    /// The provider-facing TLS client configuration: these roots, SNI on, `http/1.1` only.
    pub(crate) fn client_config(&self) -> Result<Arc<ClientConfig>, CaError> {
        let mut config = ClientConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()
            .map_err(|_| CaError::TlsConfig)?
            .with_root_certificates(self.0.clone())
            .with_no_client_auth();
        config.alpn_protocols = vec![HTTP11_ALPN.to_vec()];
        Ok(Arc::new(config))
    }
}

/// Whether a negotiated ALPN value is one interception accepts: none, or `http/1.1`.
pub(crate) fn alpn_is_http11(negotiated: Option<&[u8]>) -> bool {
    matches!(negotiated, None | Some(HTTP11_ALPN))
}

/// A configured host name: lowercase ASCII letters, digits and hyphens in dot-separated
/// labels of 1–63 bytes, not starting or ending with a hyphen, at most 253 bytes, with no
/// scheme, port, userinfo, wildcard or trailing dot. IP literals are not host names here.
pub fn is_config_host(host: &str) -> bool {
    goalcup_protocol::capture_policy::is_capture_host(host)
}

/// The ring provider, named explicitly so no process-default provider is consulted.
fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}
