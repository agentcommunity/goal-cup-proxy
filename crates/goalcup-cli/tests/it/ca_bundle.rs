//! The file a shared CA variable (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) is handed: the proxy CA
//! from the OS trust store, certificates only, written privately (WP12).

use goalcup_cli::ca_bundle::{BUNDLE_FILE_NAME, bundle_pem, certificates, system_roots_source};
use goalcup_cli::proxy::tls::ProxyCa;

fn proxy_ca() -> ProxyCa {
    ProxyCa::generate(&["api.example.org".to_string()]).expect("a CA")
}

/// The OS trust store yields real roots on every platform the tracer runs on.
#[test]
fn the_system_trust_store_holds_roots() {
    let source = system_roots_source().expect("the OS trust store is readable");
    let roots = certificates(&source);
    assert!(roots.len() >= 20, "only {} roots", roots.len());
}

/// Only `CERTIFICATE` blocks that parse as certificates are kept, each encoded again from its
/// DER: a private key, another block type, a broken block and text outside blocks never reach
/// the bundle. The proxy CA comes first, then each root.
#[test]
fn the_bundle_holds_certificates_only_with_the_proxy_ca_first() {
    let root = proxy_ca();
    let proxy = proxy_ca();
    let key = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg\n-----END PRIVATE KEY-----\n";
    let source = format!(
        "comment line\n{key}{root}-----BEGIN TRUSTED CERTIFICATE-----\nAAAA\n\
         -----END TRUSTED CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nnot base64!\n\
         -----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nMIIBAAAA\n\
         -----END CERTIFICATE-----\n{key}",
        root = root.certificate_pem(),
    );
    let pem = bundle_pem(source.as_bytes(), proxy.certificate_der()).unwrap();
    assert!(!pem.contains("PRIVATE"), "{pem}");
    assert!(!pem.contains("TRUSTED"), "{pem}");
    assert!(!pem.contains("comment"), "{pem}");
    let kept = certificates(pem.as_bytes());
    assert_eq!(
        kept,
        [
            proxy.certificate_der().to_vec(),
            root.certificate_der().to_vec()
        ]
    );
    assert_eq!(
        pem.matches("-----BEGIN CERTIFICATE-----").count(),
        2,
        "{pem}"
    );
    assert!(pem.lines().all(|l| l.len() <= 64), "{pem}");
}

/// A trust store with no root is refused, rather than a file that trusts the proxy alone.
#[test]
fn a_store_without_a_root_is_refused() {
    let proxy = proxy_ca();
    for source in [
        &b""[..],
        b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
    ] {
        assert!(bundle_pem(source, proxy.certificate_der()).is_err());
    }
}

/// The bundle is written mode 0600 into the run's `ca/` directory, and an entry already at its
/// path, a symlink included, is refused rather than followed.
#[cfg(unix)]
#[test]
fn the_bundle_is_written_privately_and_never_over_an_existing_entry() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let proxy = proxy_ca();
    let path = goalcup_cli::ca_bundle::write_bundle(dir.path(), proxy.certificate_der()).unwrap();
    assert_eq!(path, dir.path().join(BUNDLE_FILE_NAME));
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let kept = certificates(&std::fs::read(&path).unwrap());
    assert_eq!(kept[0], proxy.certificate_der().to_vec());
    assert!(kept.len() >= 21);

    let other = tempfile::tempdir().unwrap();
    let target = other.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, other.path().join(BUNDLE_FILE_NAME)).unwrap();
    assert!(goalcup_cli::ca_bundle::write_bundle(other.path(), proxy.certificate_der()).is_err());
    assert!(!target.exists());
    assert!(goalcup_cli::ca_bundle::write_bundle(dir.path(), proxy.certificate_der()).is_err());
}
