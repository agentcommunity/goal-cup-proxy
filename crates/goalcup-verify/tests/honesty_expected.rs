//! A `long_tunnel` note the proxy flagged `"expected": true` (OWN-6) is kept as a fact
//! (`long_tunnel_expected`) but never raises the listed-hosts check; an unflagged one does.

mod common;

use common::*;
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
