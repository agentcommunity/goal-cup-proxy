//! A test certificate authority, independent of the proxy's own CA code, for fake upstreams.

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const HTTP11: &[u8] = b"http/1.1";
pub const H2: &[u8] = b"h2";

/// The ring provider, the only one the proxy uses.
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A test-only CA.
pub struct TestPki {
    params: CertificateParams,
    key: KeyPair,
    root: CertificateDer<'static>,
    root_pem: String,
}

/// A leaf certificate and its key, as DER and PEM.
pub struct Leaf {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    pub cert_pem: String,
    pub key_pem: String,
}

impl TestPki {
    pub fn new(common_name: &str) -> TestPki {
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate().expect("generate a CA key");
        let cert = params.self_signed(&key).expect("self-sign the test CA");
        TestPki {
            root: cert.der().clone(),
            root_pem: cert.pem(),
            params,
            key,
        }
    }

    pub fn root(&self) -> CertificateDer<'static> {
        self.root.clone()
    }

    pub fn root_pem(&self) -> &str {
        &self.root_pem
    }

    /// Writes the root as a PEM file in `dir`, usable as `GOALCUP_TEST_EXTRA_ROOTS`.
    pub fn roots_file(&self, dir: &Path) -> PathBuf {
        let path = dir.join("test-roots.pem");
        std::fs::write(&path, &self.root_pem).expect("write the test roots file");
        path
    }

    /// A server leaf for `names` (DNS names or IP addresses), signed by this CA.
    pub fn leaf(&self, names: &[&str]) -> Leaf {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        let mut params = CertificateParams::new(names).expect("leaf names");
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let key = KeyPair::generate().expect("generate a leaf key");
        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).expect("sign the leaf");
        Leaf {
            chain: vec![cert.der().clone()],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
        }
    }

    /// A rustls server config for a leaf over `names`, offering `alpn` in order.
    pub fn server_config(&self, names: &[&str], alpn: &[&[u8]]) -> Arc<ServerConfig> {
        let leaf = self.leaf(names);
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(leaf.chain, leaf.key)
            .expect("server certificate");
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        Arc::new(config)
    }

    /// A rustls client config trusting only this CA, offering `alpn` in order.
    pub fn client_config(&self, alpn: &[&[u8]]) -> Arc<ClientConfig> {
        let mut store = RootCertStore::empty();
        store.add(self.root()).expect("add the test root");
        let mut config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(store)
            .with_no_client_auth();
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        Arc::new(config)
    }
}
