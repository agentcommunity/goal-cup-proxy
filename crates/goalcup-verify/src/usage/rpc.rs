//! A thread socket of JSON-RPC frames ([`super::hints::thread_rpc`]): the model runs on the
//! vendor's server, and the server tells the client of each model turn. Amp 0.0.1793664053:
//! `message_added` carries `params.message` with `threadId`, `messageId`, `role`, `state.type`
//! and, on an assistant turn, `usage` (`model`, `inputTokens`, `outputTokens`,
//! `cacheReadInputTokens`, `cacheCreationInputTokens`), `createdAt`, and `content` whose `Task`
//! `tool_use` blocks start the subagents. A subagent's turns are not sent; each turn that asks
//! for a tool is seen as a `tool_lease` whose `parentToolCallId` names that `Task` call, with no
//! usage. One `message_added` per assistant message was seen, sent complete (the stream before
//! it comes as `delta`); `client_resume` replays a thread's history as `message_added` again.

use serde_json::Value;

use super::Tokens;
use super::hints::id;
use super::native::ts;
use super::shapes::{End, Reply, model_name};

/// One model turn: its message id, the `Task` call of a subagent's turn, what it said, the
/// `Task` calls it made and when the server made it (wall ns).
pub struct Turn {
    pub id: String,
    pub task: Option<String>,
    pub reply: Reply,
    pub tasks: Vec<String>,
    pub at: Option<u64>,
}

/// The thread a `message_added` names.
pub fn thread(v: &Value) -> Option<String> {
    id(v.pointer("/params/message/threadId")?.as_str()?)
}

/// The model turn a server message reports, if any.
pub fn turn(v: &Value) -> Option<Turn> {
    let p = v.get("params")?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).and_then(id);
    match v.get("method")?.as_str()? {
        "tool_lease" => Some(Turn {
            id: s(p, "messageId")?,
            task: Some(s(p, "parentToolCallId")?),
            reply: Reply {
                end: Some(End::Ok),
                ..Reply::default()
            },
            tasks: Vec::new(),
            at: None,
        }),
        "message_added"
            if p.pointer("/message/role").and_then(Value::as_str) == Some("assistant") =>
        {
            let m = &p["message"];
            let u = &m["usage"];
            let n = |k: &str| u.get(k).and_then(Value::as_u64);
            let end = match m.pointer("/state/type").and_then(Value::as_str) {
                Some("complete") => Some(End::Ok),
                Some("error") => Some(End::Failed {
                    rate: false,
                    status: None,
                    code: None,
                }),
                _ => None,
            };
            let reply = Reply {
                model: model_name(u.get("model")),
                tokens: Tokens {
                    input: n("inputTokens"),
                    cached: n("cacheReadInputTokens"),
                    cache_write: n("cacheCreationInputTokens"),
                    output: n("outputTokens"),
                    reasoning: None,
                },
                end,
                ..Reply::default()
            };
            let blocks = m["content"].as_array().into_iter().flatten();
            let tasks = blocks
                .filter(|b| b["type"] == "tool_use" && b["name"] == "Task")
                .filter_map(|b| s(b, "id"))
                .collect();
            Some(Turn {
                id: s(m, "messageId")?,
                task: None,
                reply,
                tasks,
                at: m["createdAt"].as_str().and_then(ts),
            })
        }
        _ => None,
    }
}
