//! A `long_tunnel` note the proxy flagged `"expected": true` (OWN-6) is kept as a fact
//! (`long_tunnel_expected`) but never raises the listed-hosts check; an unflagged one does.
//! Listed hosts are judged by this build's `connectors/model-hosts.toml`, not the record's label.

mod common;

use common::*;
use goalcup_format::{EndReason, StreamKind};
use serde_json::{Value, json};

fn with_notes(name: &str, notes: &[(&str, bool)]) -> Value {
    let mut e = Entry::new(name);
    for &(host, expected) in notes {
        let mut n = json!({"ev": "note", "what": "long_tunnel", "host": host});
        if expected {
            n["expected"] = json!(true);
        }
        e.event(n);
    }
    e.count();
    check(&e.seal(), &[]).1
}

fn unlisted(v: &Value) -> Value {
    item(v, "listed_hosts_decrypted")["facts"]["long_tunnel_unlisted"].clone()
}

#[test]
fn an_expected_long_tunnel_does_not_warn() {
    let v = with_notes("exp-only", &[("tool.example.test", true)]);
    assert_ne!(result(&v, "listed_hosts_decrypted"), "warn");
    assert_eq!(unlisted(&v), json!([]));
    let f = &item(&v, "listed_hosts_decrypted")["facts"];
    assert_eq!(f["long_tunnel_expected"], json!(["tool.example.test"]));
}

#[test]
fn the_same_note_without_the_flag_warns_and_names_the_host() {
    let v = with_notes("exp-none", &[("tool.example.test", false)]);
    assert_eq!(result(&v, "listed_hosts_decrypted"), "warn");
    assert_eq!(unlisted(&v), json!(["tool.example.test"]));
}

#[test]
fn only_the_unflagged_host_is_named() {
    let v = with_notes(
        "exp-both",
        &[("a.example.test", true), ("b.example.test", false)],
    );
    assert_eq!(result(&v, "listed_hosts_decrypted"), "warn");
    assert_eq!(unlisted(&v), json!(["b.example.test"]));
}

/// One tunnel the record labels unlisted, with this reason and END sizes, in a sealed entry.
fn tunnel(name: &str, host: &str, reason: &str, (up, down): (u64, u64)) -> Value {
    let mut e = Entry::new(name);
    let meta = json!({"host": host, "port": 443, "mode": "tunnel", "reason": reason,
        "listed": null, "kind": null, "alpn": null, "tls": null, "fingerprint": null});
    let s = e.open(StreamKind::Conn, 0, 0, meta);
    e.end(s, EndReason::Complete, 0, json!({"up": up, "down": down}));
    e.count();
    check(&e.seal(), &[]).1
}

#[test]
fn a_tunnel_to_a_provider_labelled_not_listed_fails() {
    let v = tunnel("tun-prov", "api.openai.com", "tls_refused", (10, 10));
    assert_eq!(result(&v, "listed_hosts_decrypted"), "fail");
}

#[test]
fn a_sign_in_tunnel_to_a_provider_is_judged() {
    let v = tunnel("tun-signin", "api.openai.com", "sign_in", (10, 10));
    assert_eq!(result(&v, "listed_hosts_decrypted"), "fail");
}

#[test]
fn a_sign_in_tunnel_over_1_mib_is_named() {
    let v = tunnel("tun-big", "auth.openai.com", "sign_in", (600_000, 600_000));
    assert_eq!(result(&v, "listed_hosts_decrypted"), "named");
}

#[test]
fn a_sign_in_tunnel_to_a_never_host_is_still_free() {
    let v = tunnel("tun-free", "auth.openai.com", "sign_in", (1_000, 1_000));
    assert_eq!(result(&v, "listed_hosts_decrypted"), "pass");
}

/// A clean entry whose start event's `hosts_sha256` is `to`; `None` removes the key, as in a
/// record an older build wrote.
fn with_hosts_sha256(name: &str, to: Option<&str>) -> std::path::PathBuf {
    mutate(&clean(name), None, |recs| {
        edit_event(recs, "start", 0, |s| match to {
            Some(h) => s["hosts_sha256"] = json!(h),
            None => drop(s.as_object_mut().unwrap().remove("hosts_sha256")),
        })
    })
}

#[test]
fn a_different_host_list_hash_is_a_visible_warn() {
    let dir = with_hosts_sha256("hosts-other", Some(&"a".repeat(64)));
    let v = check(&dir, &[]).1;
    assert_eq!(result(&v, "listed_hosts_decrypted"), "warn");
    let why = item(&v, "listed_hosts_decrypted")["why"].as_str().unwrap();
    assert!(why.contains("host list"), "{why}");
    let t = text(&dir);
    assert!(t.contains("host list"), "{t}");
}

#[test]
fn a_fresh_record_and_an_old_one_without_the_hash_do_not_warn() {
    // `start()` writes the hash the recorder's way (common/mod.rs), so a clean entry matches.
    assert_eq!(
        result(
            &check(&clean("hosts-same"), &[]).1,
            "listed_hosts_decrypted"
        ),
        "pass"
    );
    let v = check(&with_hosts_sha256("hosts-none", None), &[]).1;
    assert_eq!(result(&v, "listed_hosts_decrypted"), "pass");
}

#[test]
fn an_empty_or_all_zero_host_list_hash_is_absent() {
    for h in ["", &"0".repeat(64)] {
        let v = check(&with_hosts_sha256("hosts-zero", Some(h)), &[]).1;
        assert_eq!(result(&v, "listed_hosts_decrypted"), "pass", "{h:?}");
    }
}

/// The committed 1.0.0 record (index Review focus 4) is read without a warn on its host list.
#[test]
fn a_1_0_0_fixture_record_passes_the_listed_check() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/pad/fixtures/k7-part"
    );
    let v = check(std::path::Path::new(dir), &[]).1;
    assert_eq!(result(&v, "listed_hosts_decrypted"), "pass");
}
