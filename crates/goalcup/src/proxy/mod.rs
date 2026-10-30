//! The local recording proxy (PRD-03). The harness reaches its model hosts through
//! `HTTPS_PROXY=http://127.0.0.1:<port>`; for a listed host the proxy ends TLS with a leaf from the
//! run CA, opens its own TLS leg upstream and passes the plaintext through unchanged. Each direction
//! relays first and records second: a chunk is written to the other side before the same bytes go
//! to that connection's passive parser, which hands them to the recorder (seam S1) and never makes
//! the relay wait. Anything that cannot be decrypted or parsed still passes, as a tunnel with a
//! named GAP.

pub mod ca;
pub mod tls;
