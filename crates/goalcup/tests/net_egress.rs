//! `net::Egress`: the participant's proxy and trust roots, one source for every outbound leg.

use goalcup::net::{Egress, INSIDE_RUN, admin_trusted};
use goalcup::proxy::ca::RunCa;
use std::collections::{BTreeMap, HashSet};

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn the_participants_proxy_is_used_and_its_password_kept_out_of_sight() {
    let e = Egress::from_vars(
        &vars(&[("HTTPS_PROXY", "http://u:s3cret@proxy.corp:8080/")]),
        vec![],
    );
    let up = e.proxy_for("api.openai.com").unwrap();
    assert_eq!(
        (up.host.as_str(), up.port, up.from),
        ("proxy.corp", 8080, "HTTPS_PROXY")
    );
    assert_eq!(
        up.connect_head("api.openai.com", 443),
        "CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: Basic dTpzM2NyZXQ=\r\n\r\n"
    );
    assert_eq!(
        e.line().unwrap(),
        "goalcup sends model calls and uploads through your proxy proxy.corp:8080 (HTTPS_PROXY)."
    );
    assert!(!format!("{e:?}{up:?}").contains("s3cret"));
}

#[test]
fn proxy_variables_are_read_in_curls_order() {
    let at = |p: &[(&str, &str)]| {
        Egress::from_vars(&vars(p), vec![])
            .proxy_for("api.openai.com")
            .map(|u| (u.host.clone(), u.port))
    };
    assert_eq!(
        at(&[("HTTPS_PROXY", "http://a:1"), ("https_proxy", "http://b:2")]),
        Some(("b".into(), 2))
    );
    assert_eq!(at(&[("ALL_PROXY", "c:3")]), Some(("c".into(), 3)));
    assert_eq!(at(&[("HTTPS_PROXY", "http://d")]), Some(("d".into(), 80)));
    assert_eq!(at(&[("HTTP_PROXY", "http://e:4")]), None); // https traffic only
    assert_eq!(at(&[("HTTPS_PROXY", "")]), None);
}

#[test]
fn a_proxy_goalcup_cannot_speak_is_named_and_skipped() {
    for url in ["socks5://s:1080", "https://p:443"] {
        let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", url)]), vec![]);
        assert!(e.proxy_for("api.openai.com").is_none());
        assert_eq!(
            e.line().unwrap(),
            "goalcup cannot use the proxy in HTTPS_PROXY (only http:// proxies work), so it connects directly."
        );
    }
}

#[test]
fn no_proxy_loopback_and_a_goalcup_harness_go_direct() {
    let e = Egress::from_vars(
        &vars(&[
            ("HTTPS_PROXY", "http://p:3128"),
            ("NO_PROXY", ".corp.example, internal.host,*.svc"),
        ]),
        vec![],
    );
    for h in [
        "git.corp.example",
        "corp.example",
        "internal.host",
        "a.svc",
        "127.0.0.1",
        "localhost",
        "::1",
    ] {
        assert!(e.proxy_for(h).is_none(), "{h}");
    }
    assert!(e.proxy_for("api.openai.com").is_some());
    assert!(e.proxy_for("notinternal.host").is_some());
    let star = Egress::from_vars(
        &vars(&[("HTTPS_PROXY", "http://p:3128"), ("no_proxy", "*")]),
        vec![],
    );
    assert!(star.proxy_for("api.openai.com").is_none());
    let dir = tempfile::tempdir().unwrap();
    let pem = dir.path().join("a.pem");
    std::fs::write(
        &pem,
        RunCa::mint(&["example.org".into()]).unwrap().export().cert,
    )
    .unwrap();
    let inside = Egress::from_vars(
        &vars(&[
            ("HTTPS_PROXY", "http://127.0.0.1:40123"),
            (INSIDE_RUN, "/x/upload.json"),
            ("SSL_CERT_FILE", pem.to_str().unwrap()),
        ]),
        vec![],
    );
    assert!(inside.proxy_for("api.openai.com").is_none());
    assert!(inside.own().is_empty());
    assert_eq!(inside.line(), None);
}

#[test]
fn the_participants_ca_files_join_the_roots_once() {
    let (dir, ca, os) = (
        tempfile::tempdir().unwrap(),
        RunCa::mint(&["example.org".into()]).unwrap(),
        RunCa::mint(&["example.net".into()]).unwrap(),
    );
    let pem = dir.path().join("a.pem");
    std::fs::write(&pem, ca.export().cert).unwrap();
    let p = pem.to_str().unwrap();
    let e = Egress::from_vars(
        &vars(&[
            ("SSL_CERT_FILE", p),
            ("NODE_EXTRA_CA_CERTS", p),
            ("REQUESTS_CA_BUNDLE", "/no/such/file"),
        ]),
        vec![os.cert_der().to_vec()],
    );
    assert_eq!(e.own(), [ca.cert_der().to_vec()]);
    assert_eq!(e.bundle(), [os.cert_der().to_vec(), ca.cert_der().to_vec()]);
    assert_eq!(e.roots().len(), webpki_roots::TLS_SERVER_ROOTS.len() + 2);
}

#[test]
fn only_admin_trusted_system_keychain_certificates_count() {
    let (a40, b40) = ("A".repeat(40), "B".repeat(40));
    let plist = format!(
        "<plist version=\"1.0\"><dict><key>trustList</key><dict>\
        <key>{a40}</key><dict><key>trustSettings</key><array/></dict>\
        <key>{b40}</key><dict><key>trustSettings</key><array><dict>\
        <key>kSecTrustSettingsResult</key><integer>3</integer></dict></array></dict>\
        </dict><key>trustVersion</key><integer>1</integer></dict></plist>"
    );
    assert_eq!(admin_trusted(&plist), HashSet::from([a40.clone()]));
    let empty = "<plist version=\"1.0\"><dict><key>trustList</key><dict/>\
        <key>trustVersion</key><integer>1</integer></dict></plist>";
    assert!(admin_trusted(empty).is_empty());
}

#[test]
fn odd_proxy_urls_and_trust_lists_never_panic_or_leak() {
    let up = |url: &str| Egress::from_vars(&vars(&[("HTTPS_PROXY", url)]), vec![]);
    let e = up("http://h:1/x@y");
    let p = e.proxy_for("api.openai.com").unwrap();
    assert_eq!((p.host.as_str(), p.port), ("h", 1));
    let bare = "CONNECT a:443 HTTP/1.1\r\nHost: a:443\r\n\r\n";
    assert_eq!(p.connect_head("a", 443), bare);
    for url in ["http://@h:1", "http://:pw@h"] {
        assert_eq!(
            up(url).proxy_for("a").unwrap().connect_head("a", 443),
            bare,
            "{url}"
        );
    }
    let v6 = up("http://[::1]:3128");
    assert!(v6.proxy_for("api.openai.com").is_none());
    assert!(
        v6.line()
            .unwrap()
            .starts_with("goalcup cannot use the proxy in HTTPS_PROXY")
    );
    assert!(admin_trusted("<key>trustList</key></dict><dict>").is_empty());
}
