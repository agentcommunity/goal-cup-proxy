//! `Masker::mask_meta` end to end (PRD-05 section 7, PRD-17 K6 and R8): the header rules and the
//! whole-record rules together, through the public masker. Fake keys are built from pieces.

use goalcup::mask::Masker;
use serde_json::{Value, json};

type Res = Result<(), Box<dyn std::error::Error>>;

/// The masker with no connector rules and no exact values. The S3 switch of `Masker::new` to
/// `&[connector::MaskRule]` changes only this line.
fn masker() -> Result<Masker, Box<dyn std::error::Error>> {
    Ok(Masker::new::<()>(&[], Vec::new())?)
}

fn header<'a>(meta: &'a Value, name: &str) -> Option<&'a str> {
    meta["headers"]
        .as_array()?
        .iter()
        .find(|pair| pair[0] == name)?[1]
        .as_str()
}

fn all_stars(s: Option<&str>) -> bool {
    s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b == b'*'))
}

#[test]
fn an_authorization_header_is_all_stars_and_listed() -> Res {
    let token = ["Bearer ", "tok", "-", &"Z9".repeat(12)].concat();
    let mut meta = json!({
        "method": "POST",
        "path": "/v1/responses",
        "ver": "1.1",
        "headers": [["authorization", token], ["content-type", "application/json"]],
    });
    masker()?.mask_meta(&mut meta);
    let value = header(&meta, "authorization");
    assert!(all_stars(value));
    assert_eq!(value.map(str::len), Some(token.len()));
    assert_eq!(header(&meta, "content-type"), Some("application/json"));
    assert_eq!(meta["masked"], json!(["authorization"]));
    assert_eq!(meta["method"], "POST");
    assert_eq!(meta["path"], "/v1/responses");
    Ok(())
}

#[test]
fn a_shaped_key_in_an_ordinary_header_is_masked() -> Res {
    let key = ["gh", "p_", &"a1B2".repeat(9)].concat();
    let value = format!("see {key} here");
    let mut meta = json!({
        "method": "GET",
        "path": "/",
        "headers": [["x-note", value], ["thread-id", "thr_01"]],
    });
    masker()?.mask_meta(&mut meta);
    let out = header(&meta, "x-note").unwrap_or_default();
    assert_eq!(out.len(), value.len());
    assert!(!out.contains(&key));
    assert!(out.starts_with("see ") && out.ends_with(" here"));
    assert_eq!(header(&meta, "thread-id"), Some("thr_01"));
    assert_eq!(meta["masked"], json!(["x-note"]));
    Ok(())
}

#[test]
fn a_native_stream_path_stays_readable() -> Res {
    let path = "/home/user/.claude/projects/demo/0b5e-41a2.jsonl";
    let mut meta = json!({ "path": path });
    masker()?.mask_meta(&mut meta);
    assert_eq!(meta, json!({ "path": path }));
    Ok(())
}

#[test]
fn a_pseudo_path_with_userinfo_is_masked() -> Res {
    let userinfo = ["svc", ":", "pw", "-", &"q".repeat(6)].concat();
    let target = format!("https://{userinfo}@model.example/v1/x");
    let mut meta = json!({
        "method": "GET",
        "ver": "2",
        "h2": 1,
        "headers": [[":method", "GET"], [":path", target], [":authority", "model.example"]],
    });
    masker()?.mask_meta(&mut meta);
    let out = header(&meta, ":path").unwrap_or_default();
    assert_eq!(out.len(), target.len());
    assert!(!out.contains(&userinfo));
    assert!(out.ends_with("@model.example/v1/x"));
    assert_eq!(header(&meta, ":authority"), Some("model.example"));
    assert_eq!(meta["masked"], json!([":path"]));
    Ok(())
}
