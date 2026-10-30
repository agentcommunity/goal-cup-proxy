//! The wire API shapes (PRD-09 sections 3 and 4): which shape a path picks, what a request and a
//! WebSocket client message say, and [`Reply`], which reads one call's reply event by event.
//!
//! Only model names, safe code tokens and numbers leave this file; nothing else from a body is
//! kept. Vendor pages: OpenAI Responses streaming events and WebSocket mode, Chat Completions
//! streaming, Anthropic Messages streaming, Gemini `generateContent`, OpenRouter usage accounting.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::{Shape, Tokens};

/// The shape a request path picks by its suffix, any query removed; never by host.
pub fn shape_of(path: &str) -> Option<Shape> {
    let p = path.split('?').next().unwrap_or("");
    [
        ("/responses", Shape::Responses),
        ("/messages", Shape::Anthropic),
        ("/chat/completions", Shape::Chat),
        (":generateContent", Shape::Gemini),
        (":streamGenerateContent", Shape::Gemini),
    ]
    .into_iter()
    .find_map(|(end, s)| p.ends_with(end).then_some(s))
}

/// A model name as given: printable ASCII up to 128 bytes, not a path or URL, no mask run.
pub fn model_name(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_graphic() && b != b'*')
        && !s.starts_with('/')
        && !s.contains("://");
    ok.then(|| s.to_string())
}

/// A safe code token: `[A-Za-z0-9_.-]`, up to 64 bytes.
pub fn safe_code(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    safe(s).then(|| s.to_string())
}

fn safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// What a request says: the model asked for and the reasoning effort.
#[derive(Debug, Default)]
pub struct Request {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Reads a request body (`None` when it did not parse); Gemini names its model in the path.
pub fn request(shape: Shape, path: &str, body: Option<&Value>) -> Request {
    let model = match shape {
        Shape::Gemini => {
            let p = path.split('?').next().unwrap_or("");
            let m = p.rsplit_once("models/").map(|(_, m)| m);
            let m = m.and_then(|m| m.split(':').next()).map(Value::from);
            model_name(m.as_ref())
        }
        _ => model_name(body.and_then(|b| b.get("model"))),
    };
    let effort = match shape {
        Shape::Responses => safe_code(body.and_then(|b| b.pointer("/reasoning/effort"))),
        _ => None,
    };
    Request { model, effort }
}

/// A WebSocket client text message.
#[derive(Debug, PartialEq, Eq)]
pub enum Client {
    /// `response.create`; `warmup` when it says `generate: false`.
    Create {
        warmup: bool,
        prev: Option<String>,
    },
    Interrupt,
    Other,
}

/// Reads a client message; a cut one that no longer parses is read by its leading `type`.
pub fn client(bytes: &[u8]) -> (Client, Option<Value>) {
    let v = serde_json::from_slice::<Value>(bytes).ok();
    let ty = match &v {
        Some(v) => v.get("type").and_then(Value::as_str).map(str::to_string),
        None => leading_type(bytes),
    };
    let c = match ty.as_deref() {
        Some("response.create") => Client::Create {
            warmup: v.as_ref().and_then(|v| v.get("generate")) == Some(&Value::Bool(false)),
            prev: str_of(v.as_ref().and_then(|v| v.get("previous_response_id"))),
        },
        Some("response.interrupt") => Client::Interrupt,
        _ => Client::Other,
    };
    (c, v)
}

/// The `type` of a message cut before its end: `{"type":"…"` at its start.
fn leading_type(bytes: &[u8]) -> Option<String> {
    let rest = bytes.strip_prefix(br#"{"type":""#)?;
    let end = rest.iter().position(|&b| b == b'"')?;
    let s = std::str::from_utf8(&rest[..end]).ok()?;
    safe(s).then(|| s.to_string())
}

fn str_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

/// A WebSocket server message, as the pairing in `wire.rs` needs it.
#[derive(Debug, PartialEq, Eq)]
pub enum Server {
    Created { prev: Option<String> },
    Terminal { id: Option<String> },
    Error,
    Other,
}

pub fn server(v: &Value) -> Server {
    let r = v.get("response");
    match v.get("type").and_then(Value::as_str) {
        Some("response.created") => Server::Created {
            prev: str_of(r.and_then(|r| r.get("previous_response_id"))),
        },
        Some("response.completed" | "response.incomplete" | "response.failed") => {
            Server::Terminal {
                id: str_of(r.and_then(|r| r.get("id"))),
            }
        }
        Some("error") => Server::Error,
        _ => Server::Other,
    }
}

/// How a reply ended, by its own events. The first one decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Ok,
    Failed {
        rate: bool,
        status: Option<u16>,
        code: Option<String>,
    },
}

/// One call's reply, fed event by event.
#[derive(Debug, Default)]
pub struct Reply {
    pub model: Option<String>,
    /// The response id (compared only, never written to a row).
    pub id: Option<String>,
    pub tokens: Tokens,
    pub extra: BTreeMap<String, u64>,
    pub end: Option<End>,
    /// A safe reason token, such as a `response.incomplete` reason.
    pub detail: Option<String>,
}

impl Reply {
    /// Chat's `[DONE]`.
    pub fn done(&mut self) {
        self.finish(End::Ok);
    }

    /// A whole non-SSE body: one object, or a JSON array of chunks. Bad JSON gives nothing.
    pub fn body(&mut self, shape: Shape, bytes: &[u8]) {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Array(a)) => a.iter().for_each(|v| self.event(shape, v)),
            Ok(v) => self.event(shape, &v),
            Err(_) => {}
        }
    }

    /// One SSE event, WebSocket server message or body object.
    pub fn event(&mut self, shape: Shape, v: &Value) {
        if !v.is_object() {
            return;
        }
        match shape {
            Shape::Responses => self.responses(v),
            Shape::Anthropic => self.anthropic(v),
            Shape::Chat => self.chunk(v, "model", "id", "usage", "finish_reason", Self::chat),
            Shape::Gemini => self.chunk(
                v,
                "modelVersion",
                "responseId",
                "usageMetadata",
                "finishReason",
                Self::gemini,
            ),
        }
    }

    fn finish(&mut self, e: End) {
        self.end.get_or_insert(e);
    }

    fn named(&mut self, model: Option<&Value>, id: Option<&Value>) {
        if self.model.is_none() {
            self.model = model_name(model);
        }
        if self.id.is_none() {
            self.id = str_of(id);
        }
    }

    /// Responses: usage of the terminal event only (whole bodies are response objects).
    fn responses(&mut self, v: &Value) {
        let ty = v.get("type").and_then(Value::as_str);
        let r = match ty {
            Some("error") => return self.failed(v.get("status"), Some(v)),
            Some(_) => v.get("response"),
            None if v.get("object").is_none() && v.get("error").is_some_and(Value::is_object) => {
                return self.failed(None, v.get("error"));
            }
            None => Some(v),
        };
        let Some(r) = r.filter(|r| r.is_object()) else {
            return;
        };
        self.named(r.get("model"), r.get("id"));
        let status = match ty {
            Some("response.completed") => "completed",
            Some("response.incomplete") => "incomplete",
            Some("response.failed") => "failed",
            Some(_) => return,
            None => r.get("status").and_then(Value::as_str).unwrap_or(""),
        };
        match status {
            "completed" => self.finish(End::Ok),
            "incomplete" => {
                if self.end.is_none() {
                    self.detail = safe_code(r.pointer("/incomplete_details/reason"));
                }
                self.finish(End::Ok);
            }
            "failed" => self.failed(None, r.get("error")),
            _ => return,
        }
        if let Some(u) = r.get("usage").and_then(Value::as_object) {
            let (t, extra) = responses_usage(u);
            (self.tokens, self.extra) = (t, extra);
        }
    }

    /// Anthropic: `message_start` then every `message_delta`; per field the largest wins.
    fn anthropic(&mut self, v: &Value) {
        let (m, usage) = match v.get("type").and_then(Value::as_str) {
            Some("message_start") => (v.get("message"), v.pointer("/message/usage")),
            Some("message") => (Some(v), v.get("usage")),
            Some("message_delta") => (None, v.get("usage")),
            Some("message_stop") => return self.finish(End::Ok),
            Some("error") => return self.failed(None, v.get("error")),
            _ => return,
        };
        if let Some(m) = m {
            self.named(m.get("model"), m.get("id"));
            if m.get("stop_reason").is_some_and(Value::is_string) {
                self.finish(End::Ok);
            }
        }
        if let Some(u) = usage.and_then(Value::as_object) {
            let (t, extra) = anthropic_usage(u);
            let max = |a: Option<u64>, b: Option<u64>| a.max(b);
            let o = &mut self.tokens;
            o.input = max(o.input, t.input);
            o.cached = max(o.cached, t.cached);
            o.cache_write = max(o.cache_write, t.cache_write);
            o.output = max(o.output, t.output);
            o.reasoning = max(o.reasoning, t.reasoning);
            for (k, n) in extra {
                let e = self.extra.entry(k).or_insert(n);
                *e = (*e).max(n);
            }
        }
    }

    /// Chat and Gemini chunks: the last usage seen; a finish reason on any choice ends it ok.
    fn chunk(
        &mut self,
        v: &Value,
        model: &str,
        id: &str,
        usage: &str,
        finish: &str,
        read: fn(&Map<String, Value>) -> (Tokens, BTreeMap<String, u64>),
    ) {
        if let Some(e) = v.get("error").filter(|e| e.is_object()) {
            return self.failed(None, Some(e));
        }
        self.named(v.get(model), v.get(id));
        if let Some(u) = v.get(usage).and_then(Value::as_object) {
            (self.tokens, self.extra) = read(u);
        }
        let choices = v.get("choices").or_else(|| v.get("candidates"));
        let done = choices
            .and_then(Value::as_array)
            .is_some_and(|cs| cs.iter().any(|c| c.get(finish).is_some_and(Value::is_string)));
        if done {
            self.finish(End::Ok);
        }
    }

    fn chat(u: &Map<String, Value>) -> (Tokens, BTreeMap<String, u64>) {
        let cached = num(u, "prompt_tokens_details.cached_tokens")
            .or_else(|| num(u, "prompt_cache_hit_tokens"));
        let t = Tokens {
            input: less(num(u, "prompt_tokens"), cached),
            cached,
            cache_write: num(u, "prompt_tokens_details.cache_write_tokens"),
            output: num(u, "completion_tokens"),
            reasoning: num(u, "completion_tokens_details.reasoning_tokens"),
        };
        let used = [
            "prompt_tokens",
            "prompt_tokens_details.cached_tokens",
            "prompt_cache_hit_tokens",
            "prompt_tokens_details.cache_write_tokens",
            "completion_tokens",
            "completion_tokens_details.reasoning_tokens",
        ];
        (t, extra(u, &used))
    }

    fn gemini(u: &Map<String, Value>) -> (Tokens, BTreeMap<String, u64>) {
        let cached = num(u, "cachedContentTokenCount");
        let (cand, thoughts) = (num(u, "candidatesTokenCount"), num(u, "thoughtsTokenCount"));
        let output = match (cand, thoughts) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
        };
        let t = Tokens {
            input: less(num(u, "promptTokenCount"), cached),
            cached,
            cache_write: None,
            output,
            reasoning: thoughts,
        };
        let used = [
            "promptTokenCount",
            "cachedContentTokenCount",
            "candidatesTokenCount",
            "thoughtsTokenCount",
        ];
        (t, extra(u, &used))
    }

    /// An error object (or an `error` event holding one): rate limited when its code or type
    /// says `rate_limit` or its status is 429.
    fn failed(&mut self, status: Option<&Value>, e: Option<&Value>) {
        let e = e.filter(|e| e.is_object());
        let nested = e.and_then(|e| e.get("error")).filter(|n| n.is_object());
        let field = |k: &str| {
            let top = e.and_then(|e| e.get(k)).filter(|_| k != "type" || nested.is_none());
            top.or_else(|| nested.and_then(|n| n.get(k)))
        };
        let words = [field("code"), field("type"), field("status")];
        let code = words
            .iter()
            .find_map(|w| safe_code(*w).filter(|c| c != "error"));
        let numeric = words.iter().chain([&status]).find_map(|w| (*w)?.as_u64());
        let status = numeric
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| (100..=599).contains(n));
        let rate = status == Some(429)
            || words
                .iter()
                .filter_map(|w| (*w).and_then(Value::as_str))
                .any(|s| s.contains("rate_limit"));
        self.finish(End::Failed { rate, status, code });
    }
}

fn responses_usage(u: &Map<String, Value>) -> (Tokens, BTreeMap<String, u64>) {
    let cached = num(u, "input_tokens_details.cached_tokens");
    let t = Tokens {
        input: less(num(u, "input_tokens"), cached),
        cached,
        cache_write: None,
        output: num(u, "output_tokens"),
        reasoning: num(u, "output_tokens_details.reasoning_tokens"),
    };
    let used = [
        "input_tokens",
        "input_tokens_details.cached_tokens",
        "output_tokens",
        "output_tokens_details.reasoning_tokens",
    ];
    (t, extra(u, &used))
}

/// Anthropic; `output_tokens_details.thinking_tokens` is taken as `reasoning` where given.
fn anthropic_usage(u: &Map<String, Value>) -> (Tokens, BTreeMap<String, u64>) {
    let t = Tokens {
        input: num(u, "input_tokens"),
        cached: num(u, "cache_read_input_tokens"),
        cache_write: num(u, "cache_creation_input_tokens"),
        output: num(u, "output_tokens"),
        reasoning: num(u, "output_tokens_details.thinking_tokens"),
    };
    let used = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "output_tokens",
        "output_tokens_details.thinking_tokens",
    ];
    (t, extra(u, &used))
}

/// An integer at `a` or `a.b` (one level down).
fn num(u: &Map<String, Value>, path: &str) -> Option<u64> {
    match path.split_once('.') {
        Some((a, b)) => u.get(a)?.get(b)?.as_u64(),
        None => u.get(path)?.as_u64(),
    }
}

fn less(total: Option<u64>, part: Option<u64>) -> Option<u64> {
    total.map(|t| t.saturating_sub(part.unwrap_or(0)))
}

/// Other integer members named `…tokens`, `…Tokens` or `…TokenCount`, at the top or one object
/// down (as `a.b`). `cost`, credits, plan and rate limits never match.
fn extra(u: &Map<String, Value>, used: &[&str]) -> BTreeMap<String, u64> {
    let tokenish = |k: &str| {
        safe(k) && (k.ends_with("tokens") || k.ends_with("Tokens") || k.ends_with("TokenCount"))
    };
    let mut out = BTreeMap::new();
    let mut put = |k: String, v: &Value| {
        if let Some(n) = v.as_u64().filter(|_| !used.contains(&k.as_str())) {
            out.insert(k, n);
        }
    };
    for (k, v) in u {
        if let Some(inner) = v.as_object().filter(|_| safe(k)) {
            for (k2, v2) in inner.iter().filter(|(k2, _)| tokenish(k2)) {
                put(format!("{k}.{k2}"), v2);
            }
        } else if tokenish(k) {
            put(k.clone(), v);
        }
    }
    out
}
