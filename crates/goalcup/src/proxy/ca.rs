//! The run CA (PRD-03 section 4 points 1 to 3, 5).
//!
//! The watchdog mints one CA per entry with [`RunCa::mint`], writes `run-ca.pem` and `bundle.pem`
//! with [`RunCa::write_files`] and sends [`RunCa::export`] on the first control line (seam S8).
//! Every recorder boot rebuilds the same signer with [`RunCa::import`], so a restarted recorder
//! issues leaves the harness already trusts. The key is never written here (its one file,
//! `rec/run-ca.key`, is PRD-02's), never logged and never shown by `Debug`.
//!
//! The CA has `pathLen 0` and critical name constraints from the host list's subtrees, so a leaked
//! key cannot sign an unrelated name. It is valid from one hour before minting for 8 days. Leaves
//! are minted per name on first use and cached for the boot; each carries one DNS SAN, the
//! serverAuth EKU and the Authority Key Identifier (Python 3.13 refuses a leaf without it).
//!
//! [`RunCa::mint`] takes the subtrees themselves (`HostList::ca_subtrees()`), not the host list,
//! so this module needs nothing of seam S3; the watchdog calls `mint(&hosts.ca_subtrees())`.
//! Certificate dates are wall time, as X.509 needs; no proxy timer reads them.
//!
//! Copied from `crates/goalcup-cli/src/proxy/tls.rs:100-215` (CA and leaves) and
//! `crates/goalcup-cli/src/ca_bundle.rs` (OS roots and the bundle).

use crate::watchdog::CaPem;
use base64::Engine as _;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the CA and its leaves stay valid: the 168-hour exercise plus a day (`tls.rs:55`).
pub const CA_VALIDITY: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// How far before minting the validity window starts, for a slightly slow client clock (`:59`).
pub const CA_CLOCK_SKEW: Duration = Duration::from_secs(60 * 60);

/// The CA certificate alone, in the entry folder.
pub const RUN_CA_FILE: &str = "run-ca.pem";

/// The OS roots plus the run CA, run CA first, in the entry folder.
pub const BUNDLE_FILE: &str = "bundle.pem";

/// The Linux distribution bundles, in the order tried (Debian and Ubuntu, Fedora and RHEL,
/// openSUSE, RHEL 7+, Alpine).
const LINUX_BUNDLES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// The macOS keychain that holds the system's trusted roots.
const MACOS_ROOTS_KEYCHAIN: &str = "/System/Library/Keychains/SystemRootCertificates.keychain";

/// Why the CA could not be minted, imported or used. Fixed text only: never a name, path or key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaError {
    #[error("the run CA needs at least one permitted subtree")]
    NoSubtrees,
    #[error("a subtree or leaf name is not a lowercase DNS name")]
    InvalidName,
    #[error("certificate or key generation failed")]
    Generate,
    #[error("the run CA PEM does not hold a certificate and a PKCS#8 key")]
    BadPem,
    #[error("the run CA key does not match its certificate")]
    KeyMismatch,
    #[error("the system trust store holds no root certificate")]
    NoSystemRoots,
}

/// The run CA: a signer, its certificate, and the leaves minted in this boot. Cloning shares
/// all three.
#[derive(Clone)]
pub struct RunCa {
    inner: Arc<Inner>,
}

struct Inner {
    issuer: Issuer<'static, KeyPair>,
    cert_der: CertificateDer<'static>,
    cert_pem: String,
    provider: Arc<CryptoProvider>,
    leaves: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

impl fmt::Debug for RunCa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunCa")
            .field("leaves", &lock(&self.inner.leaves).len())
            .finish_non_exhaustive()
    }
}

impl RunCa {
    /// A fresh CA whose name constraints permit `subtrees` (from `HostList::ca_subtrees()`) and
    /// their subdomains. Performs no I/O.
    pub fn mint(subtrees: &[String]) -> Result<RunCa, CaError> {
        let mut names = BTreeSet::new();
        for name in subtrees {
            if leaf_name(name).as_ref() != Some(name) {
                return Err(CaError::InvalidName);
            }
            names.insert(name.clone());
        }
        if names.is_empty() {
            return Err(CaError::NoSubtrees);
        }
        let mut params = ca_params();
        set_window(&mut params);
        params.name_constraints = Some(NameConstraints {
            permitted_subtrees: names.into_iter().map(GeneralSubtree::DnsName).collect(),
            excluded_subtrees: Vec::new(),
        });
        let key = KeyPair::generate().map_err(|_| CaError::Generate)?;
        let cert = params.self_signed(&key).map_err(|_| CaError::Generate)?;
        let cert_der = cert.der().clone();
        let cert_pem = pem_block(&cert_der);
        Ok(Self::from_parts(
            Issuer::new(ca_params(), key),
            cert_der,
            cert_pem,
        ))
    }

    /// The certificate and key as PEM, for the first control line. The caller never logs it.
    pub fn export(&self) -> CaPem {
        CaPem {
            cert: self.inner.cert_pem.clone(),
            key: self.inner.issuer.key().serialize_pem(),
        }
    }

    /// The CA a previous boot exported. rcgen cannot read the certificate back without its
    /// x509-parser feature, so the signer is rebuilt from the same fixed subject and the imported
    /// key: leaves then carry the issuer name and Authority Key Identifier of the exported
    /// certificate, which is kept as is.
    pub fn import(pem: &CaPem) -> Result<RunCa, CaError> {
        let der = certificates(pem.cert.as_bytes())
            .into_iter()
            .next()
            .ok_or(CaError::BadPem)?;
        let key = KeyPair::from_pem(&pem.key).map_err(|_| CaError::BadPem)?;
        let public = key.public_key_raw();
        if public.is_empty() || !der.windows(public.len()).any(|w| w == public) {
            return Err(CaError::KeyMismatch);
        }
        let cert_der = CertificateDer::from(der);
        let cert_pem = pem_block(&cert_der);
        Ok(Self::from_parts(
            Issuer::new(ca_params(), key),
            cert_der,
            cert_pem,
        ))
    }

    fn from_parts(
        issuer: Issuer<'static, KeyPair>,
        cert_der: CertificateDer<'static>,
        cert_pem: String,
    ) -> RunCa {
        RunCa {
            inner: Arc::new(Inner {
                issuer,
                cert_der,
                cert_pem,
                provider: ring_provider(),
                leaves: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The CA certificate, DER.
    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.inner.cert_der
    }

    /// Writes `run-ca.pem` (the certificate alone) and `bundle.pem` (the run CA, then the OS
    /// roots, certificates only) into `entry_dir`, each a new 0600 file. An existing file or
    /// link at either path is refused, never followed. The bundle is built before either file is
    /// written, so a failed OS roots read leaves no file behind. The key is not written here.
    pub fn write_files(&self, entry_dir: &Path) -> io::Result<()> {
        let bundle = system_roots_source()
            .and_then(|source| bundle_pem(&source, &self.inner.cert_der))
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        write_new(&entry_dir.join(RUN_CA_FILE), self.inner.cert_pem.as_bytes())?;
        write_new(&entry_dir.join(BUNDLE_FILE), bundle.as_bytes())
    }

    /// The leaf for `name`, minted on first use and kept for the boot. No cap: a pattern entry
    /// can match many names.
    pub fn leaf(&self, name: &str) -> Result<Arc<CertifiedKey>, CaError> {
        let name = leaf_name(name).ok_or(CaError::InvalidName)?;
        let mut leaves = lock(&self.inner.leaves);
        if let Some(leaf) = leaves.get(&name) {
            return Ok(leaf.clone());
        }
        let leaf = Arc::new(self.inner.mint_leaf(&name)?);
        leaves.insert(name, leaf.clone());
        Ok(leaf)
    }

    /// The harness-facing certificate source shared by every connection: the leaf for the
    /// client's SNI. A ClientHello without SNI gets none from it; for that case the connection
    /// layer uses [`RunCa::resolver_for`] with the CONNECT host (section 2).
    pub fn resolver(&self) -> Arc<dyn ResolvesServerCert> {
        Arc::new(LeafResolver(self.clone()))
    }

    /// A certificate source bound to `name`: its leaf whatever the ClientHello says. For a client
    /// that sent no SNI, where the CONNECT host decides.
    pub fn resolver_for(&self, name: &str) -> Result<Arc<dyn ResolvesServerCert>, CaError> {
        Ok(Arc::new(FixedLeaf(self.leaf(name)?)))
    }
}

impl Inner {
    fn mint_leaf(&self, name: &str) -> Result<CertifiedKey, CaError> {
        let mut params = CertificateParams::default();
        set_window(&mut params);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        params.distinguished_name = dn;
        params.subject_alt_names = vec![SanType::DnsName(
            name.to_owned()
                .try_into()
                .map_err(|_| CaError::InvalidName)?,
        )];
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate().map_err(|_| CaError::Generate)?;
        let leaf = params
            .signed_by(&key, &self.issuer)
            .map_err(|_| CaError::Generate)?;
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing = self
            .provider
            .key_provider
            .load_private_key(key_der)
            .map_err(|_| CaError::Generate)?;
        Ok(CertifiedKey::new(vec![leaf.der().clone()], signing))
    }
}

#[derive(Debug)]
struct LeafResolver(RunCa);

impl ResolvesServerCert for LeafResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.0.leaf(hello.server_name()?).ok()
    }
}

#[derive(Debug)]
struct FixedLeaf(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedLeaf {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// The ring provider, named so rustls never consults a process default (PRD-17 T8).
pub(crate) fn ring_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The CA's fixed subject and usages. Mint and import both build the signer from this, so the
/// issuer name and key identifier of a leaf match the exported certificate.
fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "goalcup run CA");
    dn.push(DnType::OrganizationName, "Goal Cup");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params
}

/// One hour back, 8 days ahead, in wall time.
fn set_window(params: &mut CertificateParams) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let epoch = rcgen::date_time_ymd(1970, 1, 1);
    params.not_before = epoch + now.saturating_sub(CA_CLOCK_SKEW);
    params.not_after = epoch + now + CA_VALIDITY;
}

/// `name` lowercased with one trailing dot dropped, if it is a DNS-style name: labels of letters,
/// digits, `-` and `_`, 1 to 63 bytes, not starting or ending with `-`, at most 253 bytes in all.
/// No wildcard or port. An IPv4 literal passes (a CONNECT host may be one); [`leaf_name`] refuses
/// it. The one name rule of the proxy: the CONNECT head, the SNI and the leaves all use it.
pub(crate) fn dns_name(name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name);
    let valid = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        });
    valid.then(|| name.to_owned())
}

/// A name a leaf or a subtree may carry: [`dns_name`], and not an IP literal.
fn leaf_name(name: &str) -> Option<String> {
    dns_name(name).filter(|n| n.parse::<std::net::IpAddr>().is_err())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A new 0600 file. `create_new` (O_EXCL) fails on any existing entry, a symlink included, so a
/// link is never followed.
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The OS trust store's roots as PEM text: the macOS System Roots keychain through
/// `/usr/bin/security`, else the first Linux bundle in [`LINUX_BUNDLES`] holding a certificate.
fn system_roots_source() -> Result<Vec<u8>, CaError> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/bin/security")
            .args(["find-certificate", "-a", "-p", MACOS_ROOTS_KEYCHAIN])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|_| CaError::NoSystemRoots)?;
        return if out.status.success() && !certificates(&out.stdout).is_empty() {
            Ok(out.stdout)
        } else {
            Err(CaError::NoSystemRoots)
        };
    }
    LINUX_BUNDLES
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .find(|text| !certificates(text).is_empty())
        .ok_or(CaError::NoSystemRoots)
}

/// The OS roots, the same read as the bundle; empty when the store cannot be read.
pub(crate) fn os_roots() -> Vec<CertificateDer<'static>> {
    system_roots_source()
        .map(|source| certificates(&source))
        .unwrap_or_default()
        .into_iter()
        .map(CertificateDer::from)
        .collect()
}

/// The DER of every `CERTIFICATE` PEM block in `pem` that parses as a trust anchor, in order.
/// Every other block (a private key, a `TRUSTED CERTIFICATE`, a CRL) and every line outside a
/// block is dropped.
pub fn certificates(pem: &[u8]) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(pem);
    let mut out = Vec::new();
    let mut body: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        match &mut body {
            None if line == "-----BEGIN CERTIFICATE-----" => body = Some(String::new()),
            None => {}
            Some(b64) if line == "-----END CERTIFICATE-----" => {
                if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                    && rustls::RootCertStore::empty()
                        .add(CertificateDer::from(der.as_slice()))
                        .is_ok()
                {
                    out.push(der);
                }
                body = None;
            }
            // Another BEGIN inside a block ends it unread.
            Some(_) if line.starts_with("-----") => body = None,
            Some(b64) => b64.push_str(line),
        }
    }
    out
}

/// One certificate as a PEM block, 64 base64 characters a line.
fn pem_block(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// The bundle text: the run CA, then every root certificate in `roots_source`. An error when the
/// source holds no root, since the file would then trust the run CA alone and break every
/// connection that does not go through the proxy.
fn bundle_pem(roots_source: &[u8], ca_der: &[u8]) -> Result<String, CaError> {
    let roots = certificates(roots_source);
    if roots.is_empty() {
        return Err(CaError::NoSystemRoots);
    }
    let mut out = pem_block(ca_der);
    for der in &roots {
        out.push_str(&pem_block(der));
    }
    Ok(out)
}
