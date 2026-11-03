//! `Masker::mask_meta` end to end (PRD-05 section 7, PRD-17 K6 and R8): the header rules, the
//! connector names and the whole-record rules together, through the public masker. Fake keys
//! are built from pieces.

use goalcup::connector::{FieldValue, MaskPattern, MaskRule};
use goalcup::mask::Masker;
use serde_json::{Value, json};

type Res = Result<(), Box<dyn std::error::Error>>;

fn masker() -> Result<Masker, Box<dyn std::error::Error>> {
    Ok(Masker::new(&[], Vec::new())?)
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

/// A header named like a connector `json_field` or with a connector `header_prefix` is masked
/// whole, whatever the case of either name; `Masker::new` takes the names from the rules.
#[test]
fn a_header_named_by_a_connector_rule_is_masked() -> Res {
    let rules = [
        MaskRule {
            kind: "tool_state".into(),
            pattern: MaskPattern::JsonField {
                field: "X-Tool-State".into(),
                value: FieldValue::Scalar,
            },
        },
        MaskRule {
            kind: "acme_billing".into(),
            pattern: MaskPattern::HeaderPrefix("x-acme-".into()),
        },
    ];
    let m = Masker::new(&rules, Vec::new())?;
    let state = ["st", "-", &"q7".repeat(8)].concat();
    let mut meta = json!({
        "method": "POST",
        "path": "/v1/responses",
        "headers": [
            ["x-tool-state", state],
            ["X-Acme-Billing", "cc_version=2.1"],
            ["thread-id", "thr_01"],
        ],
    });
    m.mask_meta(&mut meta);
    assert!(all_stars(header(&meta, "x-tool-state")));
    assert!(all_stars(header(&meta, "X-Acme-Billing")));
    assert_eq!(header(&meta, "thread-id"), Some("thr_01"));
    assert_eq!(meta["masked"], json!(["x-tool-state", "X-Acme-Billing"]));
    Ok(())
}
