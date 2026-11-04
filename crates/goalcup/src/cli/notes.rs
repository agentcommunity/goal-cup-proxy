//! The server's notes in the open and continue answers: at most five short lines, shown to the
//! participant as plain text. A wrong shape is ignored, never a failed open or continue (point 3).

use serde::{Deserialize, Deserializer};

/// The notes, cleaned: the first five strings, each without control, zero-width or bidi
/// characters, cut to 200 chars and trimmed; empty ones dropped.
#[derive(Debug, Default, PartialEq)]
pub struct Notes(Vec<String>);

impl<'de> Deserialize<'de> for Notes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d).unwrap_or_default();
        let lines = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str());
        Ok(Notes(lines.take(5).filter_map(clean).collect()))
    }
}

fn clean(s: &str) -> Option<String> {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    };
    let kept: String = s.chars().filter(|&c| !hidden(c)).take(200).collect();
    let line = kept.trim();
    (!line.is_empty()).then(|| line.to_string())
}

impl Notes {
    /// One `Note from the server:` line each; a failed write is ignored.
    pub fn print(&self, out: &mut dyn std::io::Write) {
        for line in &self.0 {
            let _ = writeln!(out, "Note from the server: {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    struct Wrap {
        #[serde(default)]
        notes: Notes,
    }

    fn notes(v: serde_json::Value) -> Vec<String> {
        serde_json::from_value::<Wrap>(v).unwrap().notes.0
    }

    #[test]
    fn keeps_five_lines_of_200_chars() {
        let got = notes(json!({"notes": ["1", "2", "3", "4", "5", "6", "7"]}));
        assert_eq!(got, ["1", "2", "3", "4", "5"]);
        let got = notes(json!({"notes": ["x".repeat(300), "😀".repeat(150)]}));
        assert_eq!(got[0].chars().count(), 200);
        assert_eq!(got[1].chars().count(), 150);
    }

    #[test]
    fn strips_controls_and_escapes() {
        let got = notes(json!({"notes": ["\u{1b}[31mred\u{1b}[0m\u{7}\n\u{9b}x\u{202e}y"]}));
        assert_eq!(got, ["[31mred[0mxy"]);
    }

    #[test]
    fn skips_lines_that_end_up_empty() {
        assert_eq!(notes(json!({"notes": ["\u{1b}", "  ", "ok"]})), ["ok"]);
    }

    #[test]
    fn wrong_shapes_are_ignored() {
        assert!(notes(json!({"notes": 5})).is_empty());
        assert_eq!(notes(json!({"notes": [1, "a", null]})), ["a"]);
        assert!(notes(json!({})).is_empty());
    }

    #[test]
    fn print_writes_prefixed_lines() {
        let mut out = Vec::new();
        Notes(vec!["ok".into()]).print(&mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Note from the server: ok\n"
        );
    }
}
