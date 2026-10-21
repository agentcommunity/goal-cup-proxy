//! The certificate file a shared CA variable (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) is handed:
//! the system's trusted roots plus the run's proxy CA, built for each run from the OS trust store.
//!
//! A shared variable replaces the roots of whatever reads it, the harness's tool children
//! included. The proxy CA alone would break every TLS connection that does not go through the
//! recording proxy. So the file starts with the proxy CA and then holds the roots the machine
//! already trusts, read when the run starts:
//!
//! * macOS: the System Roots keychain, exported by `/usr/bin/security find-certificate -a -p`;
//! * Linux: the first distribution bundle in [`LINUX_BUNDLES`] that holds a certificate.
//!
//! Only `CERTIFICATE` PEM blocks that parse as an X.509 trust anchor are kept, and each is
//! encoded again from its DER, so a private key or any other block in the source never reaches
//! the file. The file is written `create_new` with mode 0600 into the run's private `ca/`
//! directory, next to the proxy CA.
//!
//! A harness's own CA variable, in either mode, is handed the proxy CA alone and never this file
//! ([`handed_file`]). A replace-mode one so fails closed: a connection that skips the recording
//! proxy does not verify (D41).

use crate::CliError;
use base64::Engine as _;
use goalcup_protocol::capture_policy::SHARED_CA_ENV;
use goalcup_protocol::harness::CaptureSpec;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Which file the launch points a `capture.ca.env` variable at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandedFile {
    /// The run's proxy CA alone: a harness's own variable, in either mode.
    ProxyCa,
    /// The system's roots plus the proxy CA ([`write_bundle`]): a [`SHARED_CA_ENV`] row.
    SystemRootsAndProxyCa,
}

/// The file `name` is handed: the bundle for a [`SHARED_CA_ENV`] row, the proxy CA otherwise.
pub fn handed_file(name: &str) -> HandedFile {
    if SHARED_CA_ENV.contains(&name) {
        HandedFile::SystemRootsAndProxyCa
    } else {
        HandedFile::ProxyCa
    }
}

/// The bundle `goalcup start` writes into the run's `ca/` directory `dir`, only when the profile
/// names a [`SHARED_CA_ENV`] row; `None` otherwise, and nothing is written or run.
pub fn bundle_for(
    profile: &CaptureSpec,
    dir: &Path,
    proxy_ca_der: &[u8],
) -> Result<Option<PathBuf>, CliError> {
    if profile
        .ca
        .env
        .iter()
        .any(|name| handed_file(name) == HandedFile::SystemRootsAndProxyCa)
    {
        write_bundle(dir, proxy_ca_der).map(Some)
    } else {
        Ok(None)
    }
}

/// The bundle's file name in the run's `ca/` directory.
pub const BUNDLE_FILE_NAME: &str = "goalcup-ca-bundle.pem";

/// The Linux distribution bundles, in the order they are tried (Debian and Ubuntu, Fedora and
/// RHEL, openSUSE, RHEL 7+, Alpine).
pub const LINUX_BUNDLES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// The macOS keychain that holds the system's trusted roots.
const MACOS_ROOTS_KEYCHAIN: &str = "/System/Library/Keychains/SystemRootCertificates.keychain";

/// The most bytes of trust-store text read from one source.
const MAX_SOURCE_BYTES: u64 = 16 << 20;

/// The system's trusted roots as PEM text, read from the OS trust store. `Err` says why none
/// could be read.
pub fn system_roots_source() -> Result<Vec<u8>, String> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/bin/security")
            .args(["find-certificate", "-a", "-p", MACOS_ROOTS_KEYCHAIN])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("`/usr/bin/security` did not run ({e})"))?;
        if !out.status.success() {
            return Err(format!(
                "`/usr/bin/security` could not export {MACOS_ROOTS_KEYCHAIN} ({})",
                out.status
            ));
        }
        return Ok(out.stdout);
    }
    for path in LINUX_BUNDLES {
        let Ok(file) = std::fs::File::open(path) else {
            continue;
        };
        let mut text = Vec::new();
        if std::io::Read::read_to_end(&mut std::io::Read::take(file, MAX_SOURCE_BYTES), &mut text)
            .is_ok()
            && !certificates(&text).is_empty()
        {
            return Ok(text);
        }
    }
    Err(format!(
        "no system certificate bundle was found ({})",
        LINUX_BUNDLES.join(", ")
    ))
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
                        .add(rustls::pki_types::CertificateDer::from(der.as_slice()))
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
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// The bundle text: the proxy CA, then every root certificate in `roots_source`. `Err` when the
/// source holds no root certificate, since the file would then trust the recording proxy alone.
pub fn bundle_pem(roots_source: &[u8], proxy_ca_der: &[u8]) -> Result<String, String> {
    let roots = certificates(roots_source);
    if roots.is_empty() {
        return Err("the system trust store holds no root certificate".into());
    }
    let mut out = pem_block(proxy_ca_der);
    for der in &roots {
        out.push_str(&pem_block(der));
    }
    Ok(out)
}

/// Write `dir/`[`BUNDLE_FILE_NAME`]: the proxy CA plus the system's roots, mode 0600. An
/// existing entry at that path, a symlink included, is refused rather than followed.
pub fn write_bundle(dir: &Path, proxy_ca_der: &[u8]) -> Result<PathBuf, CliError> {
    let refused = |why: String| {
        CliError::Refused(format!(
            "the harness trusts the recording proxy through a CA variable its tool children read \
             too, so the run hands it the system's roots plus the recording proxy's CA, and \
             {why}. Nothing is launched."
        ))
    };
    let pem = system_roots_source()
        .and_then(|source| bundle_pem(&source, proxy_ca_der))
        .map_err(refused)?;
    let path = dir.join(BUNDLE_FILE_NAME);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let io = |source| CliError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut file = options.open(&path).map_err(io)?;
    file.write_all(pem.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io)?;
    Ok(path)
}
