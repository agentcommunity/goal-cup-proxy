//! The shared secret shapes (seam S12, PRD-05 section 2) and the rules they make.
//!
//! This part reads `src/mask/shapes.tsv` on its own, through `include_str!`, the way the
//! verifier does, so it pins the data file without any `mask::` API.

use regex::bytes::{Regex, RegexBuilder};

const SHAPES: &str = include_str!("../src/mask/shapes.tsv");

struct Shape {
    kind: String,
    guard: bool,
    re: Regex,
}

/// Every data row as its three raw fields, comments and blank lines skipped.
fn rows() -> Vec<Vec<&'static str>> {
    SHAPES
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
        .collect()
}

/// Every row compiled with the builder settings `compile` in `goalcup-daemon` uses today.
fn shapes() -> Vec<Shape> {
    rows()
        .into_iter()
        .map(|row| {
            assert_eq!(row.len(), 3, "a row is kind, guard, regex: {row:?}");
            let re = RegexBuilder::new(row[2])
                .build()
                .unwrap_or_else(|e| panic!("{} does not compile: {e}", row[0]));
            Shape {
                kind: row[0].to_string(),
                guard: row[1] == "1",
                re,
            }
        })
        .collect()
}

/// The span a row claims: group 1 when the regex has a group, else the whole match.
fn span<'h>(shape: &Shape, hay: &'h [u8]) -> Option<&'h [u8]> {
    let caps = shape.re.captures(hay)?;
    let m = if shape.re.captures_len() > 1 {
        caps.get(1)?
    } else {
        caps.get(0)?
    };
    Some(m.as_bytes())
}

const KINDS: [&str; 20] = [
    "anthropic_key",
    "openai_key",
    "masked_key",
    "github_token",
    "github_token",
    "aws_access_key",
    "bearer",
    "jwt",
    "private_key",
    "slack_token",
    "generic_secret_assignment",
    "generic_secret_assignment",
    "url_password",
    "google_key",
    "huggingface_token",
    "npm_token",
    "xai_key",
    "stripe_key",
    "gitlab_token",
    "digitalocean_token",
];

#[test]
fn shapes_tsv_is_the_shared_list() {
    let rows = rows();
    assert_eq!(rows.len(), 20);
    for row in &rows {
        assert_eq!(row.len(), 3, "{row:?}");
        assert!(row[1] == "0" || row[1] == "1", "{row:?}");
    }

    let shapes = shapes();
    let kinds: Vec<&str> = shapes.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, KINDS);

    let guarded: Vec<usize> = shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.guard)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(guarded, [0, 1, 3, 4, 5, 7, 9, 13, 14, 15, 16, 17, 18, 19]);

    // The two fixed indexes the stream relies on, and the slot `email` left.
    assert_eq!(kinds[10], "generic_secret_assignment");
    assert_eq!(kinds[11], "generic_secret_assignment");
    assert_eq!(kinds[12], "url_password");

    let mut distinct: Vec<&str> = Vec::new();
    for kind in kinds {
        if !distinct.contains(&kind) {
            distinct.push(kind);
        }
    }
    assert_eq!(
        distinct,
        [
            "anthropic_key",
            "openai_key",
            "masked_key",
            "github_token",
            "aws_access_key",
            "bearer",
            "jwt",
            "private_key",
            "slack_token",
            "generic_secret_assignment",
            "url_password",
            "google_key",
            "huggingface_token",
            "npm_token",
            "xai_key",
            "stripe_key",
            "gitlab_token",
            "digitalocean_token",
        ]
    );
}

#[test]
fn url_password_masks_only_the_password() {
    let shapes = shapes();
    let url = &shapes[12];
    assert_eq!(url.kind, "url_password");

    let password = ["hun", "ter2"].concat();
    let plain = format!("postgres://app:{password}@localhost:5432");
    assert_eq!(span(url, plain.as_bytes()), Some(password.as_bytes()));

    let pass = ["pa", "ss"].concat();
    let escaped = format!(r"https:\/\/u:{pass}@h");
    assert_eq!(span(url, escaped.as_bytes()), Some(pass.as_bytes()));

    for left_alone in [
        "ssh://git@github.com",
        "https://host:443/path",
        "npm:react@18",
    ] {
        assert_eq!(span(url, left_alone.as_bytes()), None, "{left_alone}");
    }
}

#[test]
fn an_email_is_no_longer_a_secret() {
    let shapes = shapes();
    let local = ["ali", "ce"].concat();
    for text in [
        format!("{local}@example.com"),
        "Author: A B <a.b@example.org>".to_string(),
    ] {
        for shape in &shapes {
            assert!(
                !shape.re.is_match(text.as_bytes()),
                "{} matches {text}",
                shape.kind
            );
        }
    }
}

#[test]
fn the_codex_rate_limit_object_is_no_secret() {
    let text = concat!(
        r#"{"rate_limits":{"primary":{"used_percent":12.0,"window_minutes":300},"#,
        r#""plan_type":"plus"}}"#
    );
    for shape in &shapes() {
        assert!(
            !shape.re.is_match(text.as_bytes()),
            "{} matches",
            shape.kind
        );
    }
}
