//! The fake upstream (PRD-14 section 5): TLS with a chosen ALPN or plain TCP, HTTP/1.1 (whole,
//! chunked, SSE, a content coding), HTTP/2 over the `h2` crate, and WebSocket with
//! permessage-deflate, from a script. It records every byte it received.
//!
//! Stub: the types are final so tests can be written against them; `start` is filled in
//! PRD-14 slice 2.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// TLS or not, and the ALPN protocols offered (`pki::HTTP11`, `pki::H2`).
#[derive(Clone, Debug)]
pub enum Tls {
    Off,
    On {
        /// DNS names or IP addresses on the leaf.
        names: Vec<String>,
        alpn: Vec<Vec<u8>>,
    },
}

/// How a response body is sent.
#[derive(Clone, Debug)]
pub enum Body {
    /// With `Content-Length`.
    Whole(Vec<u8>),
    /// `Transfer-Encoding: chunked`, one chunk per piece.
    Chunked(Vec<Vec<u8>>),
    /// `text/event-stream`, each piece flushed on its own (chunked on HTTP/1.1).
    Sse(Vec<Vec<u8>>),
}

/// A `Content-Encoding` the body is compressed with before sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    Gzip,
    Deflate,
    Br,
    Zstd,
}

/// One WebSocket message the upstream sends.
#[derive(Clone, Debug)]
pub enum WsMsg {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
}

/// What the upstream answers one request with.
#[derive(Clone, Debug)]
pub enum Answer {
    /// HTTP/1.1, or HTTP/2 when the connection chose `h2`.
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: Body,
        coding: Option<Coding>,
    },
    /// `101`, then these messages, compressed with permessage-deflate when `deflate`.
    Ws { deflate: bool, messages: Vec<WsMsg> },
}

/// The upstream's script: one answer per request, in order (the last repeats).
#[derive(Clone, Debug)]
pub struct Script {
    pub tls: Tls,
    pub answers: Vec<Answer>,
    /// Hold the TLS handshake (or, without TLS, the first read) this long.
    pub hold_handshake: Option<Duration>,
    /// Hold each answer this long after its request arrived.
    pub hold_answer: Option<Duration>,
    /// Close the connection once this many answer bytes are sent.
    pub close_after: Option<u64>,
}

impl Script {
    /// Plain TCP, one `200` with `body`.
    pub fn ok(body: &[u8]) -> Script {
        Script {
            tls: Tls::Off,
            answers: vec![Answer::Http {
                status: 200,
                headers: Vec::new(),
                body: Body::Whole(body.to_vec()),
                coding: None,
            }],
            hold_handshake: None,
            hold_answer: None,
            close_after: None,
        }
    }
}

/// What one connection sent the upstream, as raw bytes after TLS.
#[derive(Clone, Debug, Default)]
pub struct Received {
    pub conn: u64,
    pub bytes: Vec<u8>,
}

/// A running fake upstream on a held loopback port; it stops when dropped.
pub struct Upstream {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
}

impl Upstream {
    /// Starts serving `script` on a held port (`ports::held`).
    pub fn start(_script: Script) -> Upstream {
        todo!("PRD-14 slice 2: the fake upstream")
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Every connection so far, in accept order.
    pub fn received(&self) -> Vec<Received> {
        self.received
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
