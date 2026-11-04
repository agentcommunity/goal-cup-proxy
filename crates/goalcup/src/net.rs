//! Egress: the participant's proxy and trust roots, read once, for every leg goalcup opens itself
//! (the proxy's upstream leg, the server client, the run CA files).
//!
//! The proxy comes from `https_proxy`, `HTTPS_PROXY`, `all_proxy`, `ALL_PROXY` (curl's order,
//! first non-empty wins); only `http://` proxies are spoken. `no_proxy`/`NO_PROXY` and loopback go
//! direct. The roots are webpki's, the OS store's, and the certificates in the participant's
//! `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS` and `REQUESTS_CA_BUNDLE`. Inside a harness goalcup
//! launched ([`INSIDE_RUN`] set) those variables are goalcup's own, so none of them is read.
//!
//! A proxy password is never shown: `Debug` prints `Upstream(host:port)` only.

use crate::proxy::ca::certificates;
use base64::Engine as _;
use rustls::pki_types::CertificateDer;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::sync::OnceLock;

/// Set only in a harness goalcup launched: there `HTTPS_PROXY` is goalcup's own recorder.
pub const INSIDE_RUN: &str = crate::cli::time_left::FILE_ENV;

/// The proxy variables, in the order read.
const PROXY_VARS: [&str; 4] = ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"];

/// The participant's CA file variables, in the order read.
const CA_VARS: [&str; 3] = ["SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS", "REQUESTS_CA_BUNDLE"];

/// The Linux distribution bundles, in the order tried (Debian and Ubuntu, Fedora and RHEL,
/// openSUSE, RHEL 7+, Alpine).
#[cfg(not(target_os = "macos"))]
const LINUX_BUNDLES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// Where goalcup's own connections go out.
pub struct Egress {
    proxy: Option<Upstream>,
    no_proxy: Vec<String>,
    os: Vec<Vec<u8>>,
    own: Vec<Vec<u8>>,
    /// The variable naming a proxy goalcup cannot speak.
    unusable: Option<&'static str>,
}

/// The participant's `http://` proxy.
pub struct Upstream {
    pub host: String,
    pub port: u16,
    /// The variable it came from.
    pub from: &'static str,
    /// `user:pass`, percent-decoded.
    auth: Option<String>,
    ureq: ureq::Proxy,
}

impl Egress {
    /// The egress of this process, read once.
    pub fn load() -> &'static Egress {
        static EGRESS: OnceLock<Egress> = OnceLock::new();
        EGRESS.get_or_init(|| Egress::from_vars(&crate::cli::env_vars(), os_store()))
    }

    /// The egress for these variables and OS roots (DER).
    pub fn from_vars(vars: &BTreeMap<String, String>, os: Vec<Vec<u8>>) -> Egress {
        let get = |k: &str| vars.get(k).map(String::as_str).filter(|v| !v.is_empty());
        let mut egress = Egress {
            proxy: None,
            no_proxy: Vec::new(),
            os,
            own: Vec::new(),
            unusable: None,
        };
        if vars.contains_key(INSIDE_RUN) {
            return egress;
        }
        if let Some((from, url)) = PROXY_VARS.iter().find_map(|k| Some((*k, get(k)?))) {
            egress.proxy = Upstream::parse(url, from);
            egress.unusable = egress.proxy.is_none().then_some(from);
        }
        egress.no_proxy = get("no_proxy")
            .or_else(|| get("NO_PROXY"))
            .unwrap_or_default()
            .split(',')
            .map(|e| e.trim().to_ascii_lowercase())
            .map(|e| {
                e.trim_start_matches("*.")
                    .trim_start_matches('.')
                    .to_owned()
            })
            .filter(|e| !e.is_empty())
            .collect();
        for path in CA_VARS.iter().filter_map(|k| get(k)) {
            for der in certificates(&std::fs::read(path).unwrap_or_default()) {
                if !egress.own.contains(&der) {
                    egress.own.push(der);
                }
            }
        }
        egress
    }

    /// The proxy to reach `host` through, if any.
    pub fn proxy_for(&self, host: &str) -> Option<&Upstream> {
        let proxy = self.proxy.as_ref()?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let host = host.to_ascii_lowercase();
        let loopback =
            host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        let listed = self.no_proxy.iter().any(|e| {
            e == "*"
                || host == *e
                || host
                    .strip_suffix(e.as_str())
                    .is_some_and(|h| h.ends_with('.'))
        });
        (!loopback && !listed).then_some(proxy)
    }

    /// webpki's roots, then the OS store's and the participant's.
    pub fn roots(&self) -> rustls::RootCertStore {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        roots.add_parsable_certificates(self.bundle().into_iter().map(CertificateDer::from));
        roots
    }

    /// The OS roots, then the participant's CA files, DER, each once, order kept.
    pub fn bundle(&self) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for der in self.os.iter().chain(&self.own) {
            if !out.contains(der) {
                out.push(der.clone());
            }
        }
        out
    }

    /// The certificates of the participant's CA files, DER.
    pub fn own(&self) -> &[Vec<u8>] {
        &self.own
    }

    /// One plain line about the proxy, for `start` and `setup`; none when there is no proxy.
    pub fn line(&self) -> Option<String> {
        match (&self.proxy, self.unusable) {
            (Some(p), _) => Some(format!(
                "goalcup sends model calls and uploads through your proxy {}:{} ({}).",
                p.host, p.port, p.from
            )),
            (None, Some(var)) => Some(format!(
                "goalcup cannot use the proxy in {var} (only http:// proxies work), so it connects directly."
            )),
            (None, None) => None,
        }
    }
}

impl Upstream {
    /// `http://[user[:pass]@]host[:port][/]` or `host[:port]`; `None` for anything else.
    fn parse(url: &str, from: &'static str) -> Option<Upstream> {
        let rest = match url.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
            Some(_) => return None,
            None => url,
        };
        let (auth, hostport) = match rest.rsplit_once('@') {
            Some((auth, hostport)) => (Some(auth), hostport),
            None => (None, rest),
        };
        let hostport = hostport.split('/').next().unwrap_or_default();
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok()?),
            None => (hostport, 80),
        };
        if host.is_empty() {
            return None;
        }
        let auth = auth.map(|a| {
            let (user, pass) = a.split_once(':').unwrap_or((a, ""));
            format!("{}:{}", decode(user), decode(pass))
        });
        let at = auth.as_ref().map(|a| format!("{a}@")).unwrap_or_default();
        let ureq = ureq::Proxy::new(format!("http://{at}{host}:{port}")).ok()?;
        Some(Upstream {
            host: host.to_owned(),
            port,
            from,
            auth,
            ureq,
        })
    }

    /// The CONNECT request for `host:port`, with `Proxy-Authorization` when there is a user.
    pub fn connect_head(&self, host: &str, port: u16) -> String {
        let auth = self.auth.as_ref().map(|a| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(a);
            format!("Proxy-Authorization: Basic {b64}\r\n")
        });
        format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n{}\r\n",
            auth.unwrap_or_default()
        )
    }

    /// The same proxy for `ureq`.
    pub fn ureq(&self) -> ureq::Proxy {
        self.ureq.clone()
    }
}

impl fmt::Debug for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Upstream({}:{})", self.host, self.port)
    }
}

impl fmt::Debug for Egress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Egress")
            .field("proxy", &self.proxy)
            .field("unusable", &self.unusable)
            .field("os", &self.os.len())
            .field("own", &self.own.len())
            .finish()
    }
}

/// `%XX` decoded; anything else kept as is.
fn decode(s: &str) -> String {
    let (bytes, mut out, mut i) = (s.as_bytes(), Vec::new(), 0);
    while i < bytes.len() {
        let hexed = s.get(i + 1..i + 3).filter(|_| bytes[i] == b'%');
        if let Some(byte) = hexed.and_then(|h| hex::decode(h).ok()) {
            out.extend(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The SHA-1s (upper-case hex) the admin trust settings export trusts: every 40-hex `<key>` of
/// `trustList`, except one whose settings hold `kSecTrustSettingsResult` 3 (Deny).
pub fn admin_trusted(plist: &str) -> HashSet<String> {
    let mut entries: Vec<(String, bool)> = Vec::new();
    let Some((_, list)) = plist.split_once("<key>trustList</key>") else {
        return HashSet::new();
    };
    let (mut depth, mut last_key) = (0usize, "");
    for tag in list.split('<').skip(1) {
        let (name, text) = tag.split_once('>').unwrap_or((tag, ""));
        match name {
            "dict" => depth += 1,
            "dict/" if depth == 0 => break,
            "/dict" => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            "key" => {
                if depth == 1 && text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
                    entries.push((text.to_ascii_uppercase(), false));
                }
                last_key = text;
            }
            "integer" if depth > 1 && last_key == "kSecTrustSettingsResult" => {
                if text.trim() == "3"
                    && let Some(entry) = entries.last_mut()
                {
                    entry.1 = true;
                }
            }
            _ => {}
        }
    }
    entries
        .into_iter()
        .filter(|(_, denied)| !denied)
        .map(|(sha1, _)| sha1)
        .collect()
}

/// The OS trust store's roots, DER; empty when it cannot be read. Linux: the first bundle in
/// [`LINUX_BUNDLES`] holding a certificate.
#[cfg(not(target_os = "macos"))]
pub(crate) fn os_store() -> Vec<Vec<u8>> {
    LINUX_BUNDLES
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .map(|text| certificates(&text))
        .find(|certs| !certs.is_empty())
        .unwrap_or_default()
}

/// The OS trust store's roots, DER; empty when it cannot be read. macOS: the System Roots
/// keychain, plus each `/Library/Keychains/System.keychain` certificate the admin trusts.
#[cfg(target_os = "macos")]
pub(crate) fn os_store() -> Vec<Vec<u8>> {
    use sha1::{Digest as _, Sha1};
    let mut out = certificates(&security(&[
        "find-certificate",
        "-a",
        "-p",
        "/System/Library/Keychains/SystemRootCertificates.keychain",
    ]));
    let path = std::env::temp_dir().join(format!("goalcup-trust-{}.plist", std::process::id()));
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .is_ok()
    {
        let file = path.to_string_lossy();
        security(&["trust-settings-export", "-d", &file]);
        let trusted = admin_trusted(&std::fs::read_to_string(&path).unwrap_or_default());
        let _ = std::fs::remove_file(&path);
        let system = security(&[
            "find-certificate",
            "-a",
            "-p",
            "/Library/Keychains/System.keychain",
        ]);
        out.extend(
            certificates(&system)
                .into_iter()
                .filter(|der| trusted.contains(&hex::encode_upper(Sha1::digest(der)))),
        );
    }
    out
}

/// The output of `/usr/bin/security`, empty on failure; no environment, stdin or stderr.
#[cfg(target_os = "macos")]
fn security(args: &[&str]) -> Vec<u8> {
    std::process::Command::new("/usr/bin/security")
        .args(args)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| out.stdout)
        .unwrap_or_default()
}
