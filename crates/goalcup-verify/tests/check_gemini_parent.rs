//! Gemini CLI chat files give threads with parent links (point 2): a subagent file under
//! `chats/<parent session id>/` is a child of that session, whose own file is named only by the
//! first 8 characters of its id. No per-call rows are read; wire calls stay wire-only.

mod common;

use common::{Entry, edit_event, read_records};
use goalcup_format::{EndReason, StreamKind};
use goalcup_verify::usage::{Calls, Join, Kind};
use serde_json::{Value, json};

const MAIN: &str = "aaaa1111-fake-session-id";
const KID: &str = "bbbb2222-fake-subagent-id";

fn chat(e: &mut Entry, path: &str, body: &str) {
    let s = e.open(StreamKind::Native, 0, 0, json!({"path": path, "gen": 0, "from": 0}));
    e.data(s, 0, 0, body.as_bytes());
    e.end(s, EndReason::Complete, body.len() as u64, Value::Null);
}

#[test]
fn a_subagent_chat_file_names_its_parent_session() {
    for head in [true, false] {
        let mut e = Entry::new("gemini-parent");
        e.standard();
        let first = if head { format!("{{\"sessionId\":\"{MAIN}\"}}\n") } else { "{}\n".into() };
        chat(&mut e, "chats:chats/session-2026-11-01T10-00-aaaa1111.jsonl", &first);
        chat(&mut e, &format!("chats:chats/{MAIN}/{KID}.jsonl"), "{}\n");
        let mut recs = read_records(&e.seal().join("rec"));
        edit_event(&mut recs, "start", 0, |v| v["connector"] = "gemini-cli".into());
        let mut calls = Calls::new();
        recs.iter().for_each(|r| calls.push(r));
        let t = calls.finish();
        let kid = t.threads.iter().find(|x| x.id == KID).expect("subagent thread");
        assert_eq!((kid.parent.as_deref(), kid.depth), (Some(if head { MAIN } else { "session-2026-11-01T10-00-aaaa1111" }), Some(1)));
        assert!(t.threads.iter().any(|x| x.parent.is_none() && x.depth == Some(0)));
        let calls = t.rows.iter().filter(|r| r.kind == Kind::Call);
        assert!(calls.clone().count() > 0 && calls.into_iter().all(|r| r.join == Some(Join::WireOnly)));
    }
}
