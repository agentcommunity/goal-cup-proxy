//! Shared test support for the `goalcup` integration tests (PRD-14 section 5). Each test binary
//! includes it as `mod common;` and uses the parts it needs, so unused items are allowed here.
//!
//! - [`ports`]: the only place a test binds port 0 (section 4).
//! - [`decode`]: body encoders and the synthetic SSE text for the decode tests.
//! - [`fds`]: raise the descriptor limit before a test that opens many.
//! - [`home`]: temporary homes for a child, and the real-home guard.
//! - [`pki`]: a test CA and leaf certificates.
//! - [`records`]: a part folder built from a list of records.
//! - [`upstream`]: the fake upstream (TLS, HTTP/1.1, HTTP/2, WebSocket).
//! - [`worker`]: the fake Worker (`docs/UPLOAD-API.md` from a script).
//! - [`recorder`]: helpers over the calls `Recorder::for_tests` collects.

#![allow(dead_code)]

pub mod decode;
pub mod fds;
pub mod home;
pub mod pki;
pub mod ports;
pub mod recorder;
pub mod records;
pub mod upstream;
pub mod worker;

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// The usual hang bound. It guards only a hang, never a speed (section 4).
pub const HANG: Duration = Duration::from_secs(30);

/// Runs `f` on its own thread and returns its value, or fails the test with one clear line when
/// it is still running after `bound`. A panic in `f` is passed on as it is.
pub fn within<T: Send + 'static>(bound: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(bound) {
        Ok(v) => v,
        Err(mpsc::RecvTimeoutError::Disconnected) => match worker.join() {
            Err(payload) => std::panic::resume_unwind(payload),
            Ok(()) => unreachable!("the value was sent or the thread panicked"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("hang: still running after {bound:?} (a hang bound, not a speed check)")
        }
    }
}

/// [`within`] for a future, inside a tokio runtime.
pub async fn within_async<F: Future>(bound: Duration, fut: F) -> F::Output {
    tokio::time::timeout(bound, fut)
        .await
        .unwrap_or_else(|_| panic!("hang: still waiting after {bound:?} (a hang bound)"))
}

/// The `goalcup` binary cargo built for these tests (with `test-hooks`).
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_goalcup"))
}
