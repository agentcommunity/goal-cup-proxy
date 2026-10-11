//! `{name}` substitution that fails rather than leaving a placeholder on disk.
//!
//! One implementation for both sides: the daemon renders the connector's hook-install
//! files and the CLI renders launch argv and environment values.
//! Two implementations would be two ways for a per-run token to survive
//! unrendered into a file.

use crate::ProtocolError;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TemplateVars(pub BTreeMap<String, String>);

/// Prints every key, and every value except a `*_token` one, which prints `<redacted>`.
///
/// The map can hold `receiver_token` (the V1 `vars::build` in `goalcup-cli`, removed by D48,
/// put this run's live bearer token in it so a launch recipe could render
/// `{receiver_token}`), and a derived `Debug` would put such a token into whatever a `{:?}`
/// on a launch failure reaches: a terminal, a log file, an issue report. Redacting on the
/// type is the only place the guarantee holds for every caller; a wrapper at one call site
/// is a guarantee that lasts until the second call site.
///
/// The key is never redacted and no other value is touched, so a `{:?}` still says which
/// variables were supplied and what the rest of them rendered to — which is the whole
/// reason to print the map at all. The suffix and not an exact name, because any variable
/// named `*_token` is a secret and should not have to remember to come back here.
impl fmt::Debug for TemplateVars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TemplateVars ")?;
        let mut m = f.debug_map();
        for (k, v) in &self.0 {
            if k.ends_with("_token") {
                m.entry(k, &"<redacted>");
            } else {
                m.entry(k, v);
            }
        }
        m.finish()
    }
}

impl TemplateVars {
    pub fn new() -> TemplateVars {
        TemplateVars(BTreeMap::new())
    }

    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        self.0.insert(k.to_string(), v.into());
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(|s| s.as_str())
    }
}

/// One lexed piece of a template: literal text, or the name inside a `{name}` token.
enum Piece<'a> {
    Literal(&'a str),
    Token(&'a str),
}

/// The one lexer. `{{` and `}}` are literal braces; any other `{` must open a non-empty token
/// that closes at the next `}` (so a `{` inside a token is refused), and a lone `}` is refused.
/// `Err` is the reason, phrased to follow "the template".
fn lex(value: &str) -> Result<Vec<Piece<'_>>, String> {
    // Every brace is one ASCII byte, so each slice below starts and ends on a char boundary.
    let bytes = value.as_bytes();
    let mut pieces = Vec::new();
    let (mut i, mut run) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b'}' => {
                if run < i {
                    pieces.push(Piece::Literal(&value[run..i]));
                }
                let brace = bytes[i];
                if bytes.get(i + 1) == Some(&brace) {
                    pieces.push(Piece::Literal(&value[i..i + 1]));
                    i += 2;
                } else if brace == b'{' {
                    let Some(len) = value[i + 1..].find('}') else {
                        return Err("holds an unterminated `{`".to_string());
                    };
                    let name = &value[i + 1..i + 1 + len];
                    if name.is_empty() || name.contains('{') {
                        return Err("holds a `{` that does not open a token".to_string());
                    }
                    pieces.push(Piece::Token(name));
                    i += len + 2;
                } else {
                    return Err("holds a lone `}`".to_string());
                }
                run = i;
            }
            _ => i += 1,
        }
    }
    if run < bytes.len() {
        pieces.push(Piece::Literal(&value[run..]));
    }
    Ok(pieces)
}

/// The `{name}` tokens of a template, left to right, by the strict lexer [`render`] also uses:
/// `{{` and `}}` are literal braces, and any other brace must open and close a token. `Err` is
/// the reason, phrased to follow the thing being checked ("holds a lone `}`").
///
/// A validator that calls this and a renderer cannot disagree about where a token is.
pub fn tokens(template: &str) -> Result<Vec<&str>, String> {
    Ok(lex(template)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Token(name) => Some(name),
            Piece::Literal(_) => None,
        })
        .collect())
}

/// `{name}` is replaced; `{{` is a literal `{` and `}}` a literal `}`; anything else between
/// braces that is not a known variable is an error, and so is a lone `}` or an unterminated `{`
/// (the rules of [`tokens`]).
pub fn render(template: &str, vars: &TemplateVars) -> Result<String, ProtocolError> {
    let pieces = lex(template).map_err(|why| ProtocolError::Field {
        field: "template".into(),
        reason: format!("the template {why}"),
    })?;
    let mut out = String::with_capacity(template.len());
    for piece in pieces {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Token(name) => {
                let value = vars.get(name).ok_or_else(|| ProtocolError::Field {
                    field: "template".into(),
                    reason: format!("refers to {{{name}}}, which the tracker does not supply"),
                })?;
                out.push_str(value);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{TemplateVars, render, tokens};

    /// A `{:?}` on a launch failure is the obvious thing to write in an error path, and the
    /// map it prints carries this run's bearer token.
    #[test]
    fn debug_redacts_token_values_and_prints_everything_else() {
        let mut v = TemplateVars::new();
        v.set("receiver_token", "live-secret-abc123");
        v.set("receiver_http", "http://127.0.0.1:53311");
        v.set("session_id", "session-0000");
        let printed = format!("{v:?}");
        assert!(
            !printed.contains("live-secret-abc123"),
            "a token survived a `{{:?}}`: {printed}"
        );
        // The key stays: a reader has to be able to see that the variable was supplied.
        assert!(printed.contains("receiver_token"));
        assert!(printed.contains("<redacted>"));
        assert!(printed.contains("http://127.0.0.1:53311"));
        assert!(printed.contains("session-0000"));
        // The value is what is hidden, not the fact that a token was there.
        assert_eq!(v.get("receiver_token"), Some("live-secret-abc123"));
    }

    /// The renderer before `tokens` existed, kept here as the reference the new one is compared to.
    fn old_render(template: &str, vars: &TemplateVars) -> Result<String, ()> {
        let chars: Vec<char> = template.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '{' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                    out.push('{');
                    i += 2;
                }
                '}' if i + 1 < chars.len() && chars[i + 1] == '}' => {
                    out.push('}');
                    i += 2;
                }
                '{' => {
                    let rel = chars[i + 1..].iter().position(|c| *c == '}').ok_or(())?;
                    let name: String = chars[i + 1..i + 1 + rel].iter().collect();
                    out.push_str(vars.get(&name).ok_or(())?);
                    i += rel + 2;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    #[test]
    fn tokens_refuses_a_lone_close_brace_an_empty_name_and_a_nested_open_brace() {
        assert_eq!(tokens("a}b").unwrap_err(), "holds a lone `}`");
        assert_eq!(tokens("}").unwrap_err(), "holds a lone `}`");
        assert_eq!(tokens("{{}").unwrap_err(), "holds a lone `}`");
        assert_eq!(
            tokens("a{}b").unwrap_err(),
            "holds a `{` that does not open a token"
        );
        assert_eq!(
            tokens("{a{b}").unwrap_err(),
            "holds a `{` that does not open a token"
        );
        assert_eq!(tokens("{a").unwrap_err(), "holds an unterminated `{`");
        assert_eq!(tokens("x{").unwrap_err(), "holds an unterminated `{`");
    }

    #[test]
    fn tokens_reads_names_left_to_right_and_skips_escaped_braces() {
        assert_eq!(tokens("").unwrap(), Vec::<&str>::new());
        assert_eq!(tokens("plain").unwrap(), Vec::<&str>::new());
        assert_eq!(tokens("{a} and {b}{a}").unwrap(), ["a", "b", "a"]);
        assert_eq!(tokens("{{not}} {x} }}{{").unwrap(), ["x"]);
        assert_eq!(
            tokens("--cd={workspace}/é{port}").unwrap(),
            ["workspace", "port"]
        );
    }

    /// Render agrees with the old renderer on every input the old one rendered and the strict
    /// lexer accepts, and refuses everything the old one refused. The only inputs it newly
    /// refuses hold a lone `}`. Exhaustive over every string of up to seven characters from
    /// `{`, `}`, `a`, `b` and a multi-byte `é` (the variables are `a` and `b`).
    #[test]
    fn render_gives_the_old_answer_on_every_input_the_strict_lexer_accepts() {
        let mut vars = TemplateVars::new();
        vars.set("a", "X");
        vars.set("b", "{Y}");
        let alphabet = ['{', '}', 'a', 'b', 'é'];
        let mut inputs = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..7 {
            let mut next = Vec::new();
            for s in &frontier {
                for c in alphabet {
                    next.push(format!("{s}{c}"));
                }
            }
            inputs.extend(next.iter().cloned());
            frontier = next;
        }
        let (mut agreed, mut newly_refused) = (0, 0);
        for input in &inputs {
            let (old, new) = (old_render(input, &vars), render(input, &vars));
            match (&old, &new) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{input:?}");
                    agreed += 1;
                }
                (Err(()), Err(_)) => {}
                (Ok(_), Err(_)) => {
                    // Only a lone `}` (which the old renderer left in as a literal).
                    assert_eq!(tokens(input).unwrap_err(), "holds a lone `}`", "{input:?}");
                    newly_refused += 1;
                }
                (Err(()), Ok(rendered)) => panic!("{input:?} rendered {rendered:?} now"),
            }
        }
        assert!(
            agreed > 1000 && newly_refused > 100,
            "{agreed} {newly_refused}"
        );
    }

    #[test]
    fn render_substitutes_and_unescapes() {
        let mut v = TemplateVars::new();
        v.set("workspace", "/w");
        assert_eq!(render("a {workspace} b", &v).unwrap(), "a /w b");
        assert_eq!(
            render("{{literal}} {{{workspace}}}", &v).unwrap(),
            "{literal} {/w}"
        );
        let e = render("{nope}", &v).unwrap_err().to_string();
        assert!(e.contains("{nope}") && e.contains("does not supply"), "{e}");
        let e = render("oops }", &v).unwrap_err().to_string();
        assert!(e.contains("lone `}`"), "{e}");
    }

    /// The V1 templates the daemon renders are inside the strict grammar.
    #[test]
    fn every_shipped_v1_template_lexes_under_the_strict_grammar() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
        let mut seen = 0;
        for id in ["claude-code-cli", "codex-cli"] {
            let manifest_path = root.join(id).join("manifest.toml");
            let manifest = crate::manifest::ConnectorManifest::from_toml_str(
                &std::fs::read_to_string(&manifest_path)
                    .unwrap_or_else(|e| panic!("{}: {e}", manifest_path.display())),
            )
            .unwrap_or_else(|e| panic!("{}: {e}", manifest_path.display()));
            for hook in &manifest.hook_install {
                let path = root.join(id).join("files").join(&hook.template);
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                if let Err(why) = tokens(&text) {
                    panic!("{}: {why}", path.display());
                }
                seen += 1;
            }
        }
        assert_eq!(seen, 2, "the claude and codex V1 templates");
    }
}
