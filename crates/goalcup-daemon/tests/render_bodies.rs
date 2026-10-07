//! Every `connectors/<id>/files/*` body must survive `template::render` and come out as
//! the language it claims to be. `render` is strict — a lone `{` starts a variable, and an
//! unknown name is a hard error — so a body that forgets to double a literal brace fails
//! here, in milliseconds, naming the file, rather than at workspace creation, where the
//! failure reads as "no entry could ever be started on this connector".
//!
//! If one of these fails, the fix is a doubled brace in the body — never a looser
//! `render`. One substitution syntax is the rule: a second, lenient entry
//! point that passed unknown `{…}` through would let an unescaped body ship a literal
//! `{receiver_token}` into a config file the harness then reads.

use goalcup_daemon::workspace::template_body;
use goalcup_protocol::template::{TemplateVars, render};

fn vars() -> TemplateVars {
    let mut v = TemplateVars::new();
    for (k, val) in [
        ("receiver_http", "http://127.0.0.1:7777"),
        ("receiver_token", "tok_abc"),
        ("entry_id", "e1"),
        ("workspace", "/ws/e1"),
        ("status_line_path", "/art/e1/status.txt"),
        ("artifact_root", "/art/e1"),
    ] {
        v.set(k, val);
    }
    v
}

/// The shipped bodies, and — for each — the per-run values it is expected to carry once
/// rendered. Claude's launch settings carry only the status line since the hook object went
/// (m2/wire deletion pass); Codex's `config.toml` carries the workspace it trusts.
const BODIES: &[(&str, &str, &[&str])] = &[
    (
        "claude-code-cli",
        "settings-launch.json",
        &["/art/e1/status.txt"],
    ),
    ("codex-cli", "config.toml", &["/ws/e1"]),
];

fn connectors() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// The first `{` followed by an identifier character — the shape of an unsubstituted
/// template variable. A rendered body must have none: `render` turns `{{` into a literal
/// `{`, and a literal `{` in JSON or TypeScript is always followed by whitespace, a quote,
/// a newline or a closing brace, never by `[A-Za-z_]`.
///
/// The one exception is `${…}`, which is a template-literal interpolation in
/// `goalcup-tracker.ts` — what the doubled `${{…}}` in that body is *for*. A `{` preceded by `$` is therefore not a
/// placeholder, and a body that forgot to double one still fails, because `render` would
/// have refused it as an unknown variable long before this check ran.
fn holds_a_placeholder(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '{' || (i > 0 && chars[i - 1] == '$') {
            continue;
        }
        let mut j = i + 1;
        if !(j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '_')) {
            continue;
        }
        while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
            j += 1;
        }
        // A bare identifier and then a closing brace, and nothing else in between: that is
        // the exact shape `render` substitutes, and the exact shape it would have refused.
        // The OpenCode plugin's doc block quotes the loader glob `{plugin,plugins}/*.{ts,js}`
        // — real literal braces, correctly doubled — and a comma is not part of a name.
        if j < chars.len() && chars[j] == '}' {
            return Some(chars[i..].iter().take(48).collect());
        }
    }
    None
}

#[test]
fn every_shipped_body_renders_and_leaves_no_unsubstituted_identifier() {
    let root = connectors();
    for (id, name, expected) in BODIES {
        let body = template_body(&root.join(id), name).unwrap();
        let out = render(&body, &vars()).unwrap_or_else(|e| panic!("{id}/{name}: {e}"));
        assert!(
            !out.contains("{receiver_token}"),
            "{id}/{name} shipped a literal variable"
        );
        if let Some(tail) = holds_a_placeholder(&out) {
            panic!("{id}/{name} still holds a placeholder: {tail}");
        }
        for want in *expected {
            assert!(
                out.contains(want),
                "{id}/{name} substituted nothing: no {want:?} in the rendered body"
            );
        }
        if name.ends_with(".json") {
            let _: serde_json::Value = serde_json::from_str(&out)
                .unwrap_or_else(|e| panic!("{id}/{name} is not JSON after rendering: {e}"));
        }
    }
}

#[test]
fn the_claude_settings_carry_the_status_line_and_no_hooks() {
    let root = connectors();
    let out = render(
        &template_body(&root.join("claude-code-cli"), "settings-launch.json").unwrap(),
        &vars(),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        v.get("hooks").is_none(),
        "the hook object went with the hook leg (m2/wire deletion pass)"
    );
    assert_eq!(v["statusLine"]["type"], "command");
    assert_eq!(
        v["disableAllHooks"], false,
        "explicit: Claude Code's `disableAllHooks` also switches the custom status line off"
    );
    assert!(!out.contains("tok_abc") && !out.contains("http://127.0.0.1:7777"));
    // The status line reads a file, so it never holds a secret.
    let cmd = v["statusLine"]["command"].as_str().unwrap();
    assert!(cmd.starts_with("cat "), "got {cmd:?}");
}
