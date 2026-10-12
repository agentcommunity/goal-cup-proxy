//! A test-local ingest endpoint emulator.
//!
//! A real loopback TCP listener that reads the length-framed handshake and then length-prefixed
//! `IngestMessage` items with the protocol's bounded `decode_ingest_message`. It never writes a
//! byte back (the ingest protocol has no acknowledgement), and it shares no code with the sender
//! under test or with the daemon's receiver. Its controls stall reading, close a connection, or
//! close by policy; its observations are the accepted connections with every byte read, the decoded
//! items and how each connection ended. Each connection is served on its own thread, so a stalled
//! reader really leaves the kernel buffers to fill.

#![allow(dead_code)]

use goalcup_protocol::capture::{
    FrameEvent, IngestMessage, MAX_HANDSHAKE_BYTES, MAX_INGEST_FRAME_BYTES, checked_frame_len,
    decode_ingest_message,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The handshake as this endpoint independently decodes it: exactly these three fields.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    pub entry_id: String,
    pub producer: String,
    pub token: String,
}

/// What the endpoint does with a connection it accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Read the handshake, then items until EOF.
    Serve,
    /// Close at once, before reading anything.
    DropOnAccept,
    /// Read the handshake, then close before any item.
    CloseAfterHandshake,
    /// Read the handshake and this many items, then close.
    CloseAfterItems(usize),
    /// Read the handshake and this many items, then half of the next item's body, then close.
    CloseMidItem(usize),
}

/// How a connection ended, as this endpoint saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// EOF exactly at an item boundary.
    Eof,
    /// EOF inside an item (prefix or body).
    TornTail,
    /// The endpoint closed it (policy or explicit control).
    ClosedByEndpoint,
    /// The endpoint refused the handshake's credential and closed.
    Refused,
    /// Bytes that are not a valid ingest item; the endpoint closed.
    Invalid(String),
    /// A read error (for example a reset).
    Error(ErrorKind),
}

/// The endpoint's judgement of one connection, by the rules a receiver applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A valid ordered `Close` naming the last frame, no open stream, then EOF.
    Clean {
        final_seq: Option<u64>,
    },
    /// Anything else short of invalid framing: unknown tail loss.
    UnknownLoss(String),
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct Conn {
    pub accepted_at: Instant,
    pub ended_at: Option<Instant>,
    /// Every byte read from this connection, handshake included.
    pub raw: Vec<u8>,
    /// Bytes read after the handshake, including any torn tail.
    pub item_bytes: u64,
    /// Framed length (prefix included) of each complete item, in order.
    pub item_lens: Vec<usize>,
    pub handshake: Option<Handshake>,
    pub messages: Vec<IngestMessage>,
    pub end: Option<End>,
    /// Closed by [`Emulator::close_connection`].
    pub killed: bool,
}

impl Conn {
    fn new() -> Self {
        Conn {
            accepted_at: Instant::now(),
            ended_at: None,
            raw: Vec::new(),
            item_bytes: 0,
            item_lens: Vec::new(),
            handshake: None,
            messages: Vec::new(),
            end: None,
            killed: false,
        }
    }

    pub fn producer(&self) -> &str {
        self.handshake.as_ref().map_or("", |h| h.producer.as_str())
    }

    /// Complete item bytes received (prefixes included).
    pub fn complete_item_bytes(&self) -> u64 {
        self.item_lens.iter().map(|len| *len as u64).sum()
    }

    pub fn has_close(&self) -> bool {
        self.messages
            .iter()
            .any(|m| matches!(m, IngestMessage::Close { .. }))
    }

    /// The receiver rules: one producer from the handshake, sequence from zero without a
    /// gap, stream zero only for controls, payload only on open streams, `Close` naming the
    /// last frame with no open stream and nothing after it, then EOF.
    pub fn verdict(&self) -> Verdict {
        let Some(handshake) = &self.handshake else {
            return Verdict::UnknownLoss("no handshake".into());
        };
        let mut next_seq = 0u64;
        let mut last_seq = None;
        let mut open = BTreeSet::new();
        let mut high_stream = 0u64;
        let mut closed = None;
        for (i, message) in self.messages.iter().enumerate() {
            if closed.is_some() {
                return Verdict::Invalid(format!("item {i} after Close"));
            }
            match message {
                IngestMessage::Frame(frame) => {
                    if frame.stream.producer != handshake.producer {
                        return Verdict::Invalid(format!("item {i} names another producer"));
                    }
                    if frame.seq != next_seq {
                        return Verdict::Invalid(format!(
                            "item {i} has seq {} where {next_seq} was next",
                            frame.seq
                        ));
                    }
                    let stream = frame.stream.stream;
                    match &frame.event {
                        FrameEvent::Connection { .. }
                        | FrameEvent::Recovery { .. }
                        | FrameEvent::Loss { .. } => {}
                        FrameEvent::Open { .. } => {
                            if stream <= high_stream {
                                return Verdict::Invalid(format!("item {i} reuses a stream"));
                            }
                            high_stream = stream;
                            open.insert(stream);
                        }
                        FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } => {
                            if !open.contains(&stream) {
                                return Verdict::Invalid(format!("item {i} on a closed stream"));
                            }
                        }
                        FrameEvent::End { .. } => {
                            if !open.remove(&stream) {
                                return Verdict::Invalid(format!("item {i} ends a closed stream"));
                            }
                        }
                    }
                    last_seq = Some(frame.seq);
                    next_seq += 1;
                }
                IngestMessage::Close {
                    producer,
                    final_seq,
                } => {
                    if *producer != handshake.producer {
                        return Verdict::Invalid("Close names another producer".into());
                    }
                    if *final_seq != last_seq {
                        return Verdict::UnknownLoss(format!(
                            "Close names {final_seq:?}, the last frame was {last_seq:?}"
                        ));
                    }
                    if !open.is_empty() {
                        return Verdict::UnknownLoss("Close with open streams".into());
                    }
                    closed = Some(*final_seq);
                }
            }
        }
        match (closed, &self.end) {
            (Some(final_seq), Some(End::Eof)) => Verdict::Clean { final_seq },
            (Some(_), end) => Verdict::UnknownLoss(format!("Close then {end:?}")),
            (None, end) => Verdict::UnknownLoss(format!("no Close; ended {end:?}")),
        }
    }
}

#[derive(Debug)]
pub struct State {
    pub conns: Vec<Conn>,
    /// Connections open right now, and the most ever open at once.
    pub open_now: usize,
    pub max_open: usize,
    reading: bool,
    policy: Policy,
    expected_token: Option<String>,
    stopping: bool,
    sockets: Vec<TcpStream>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }
}

/// See the module documentation.
pub struct Emulator {
    addr: SocketAddr,
    shared: Arc<Shared>,
    threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
    accept: Option<JoinHandle<()>>,
}

impl Emulator {
    /// A fresh endpoint on `127.0.0.1:0`.
    pub fn start() -> Self {
        Self::bind("127.0.0.1:0".parse().unwrap())
    }

    /// An endpoint on a specific loopback address.
    pub fn bind(addr: SocketAddr) -> Self {
        Self::from_listener(TcpListener::bind(addr).expect("bind the emulator"))
    }

    /// An endpoint serving an already-bound listener (for example a [`ReservedPort`]).
    pub fn from_listener(listener: TcpListener) -> Self {
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                conns: Vec::new(),
                open_now: 0,
                max_open: 0,
                reading: true,
                policy: Policy::Serve,
                expected_token: None,
                stopping: false,
                sockets: Vec::new(),
            }),
            changed: Condvar::new(),
        });
        let threads = Arc::new(Mutex::new(Vec::new()));
        let accept = {
            let shared = Arc::clone(&shared);
            let threads = Arc::clone(&threads);
            std::thread::spawn(move || accept_loop(listener, shared, threads))
        };
        Emulator {
            addr,
            shared,
            threads,
            accept: Some(accept),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn set_policy(&self, policy: Policy) {
        self.shared.update(|s| s.policy = policy);
    }

    /// Close a connection whose handshake names another token (receiver policy refusal).
    pub fn expect_token(&self, token: &str) {
        let token = token.to_string();
        self.shared.update(|s| s.expected_token = Some(token));
    }

    /// Stop reading items (the handshake is still read). Kernel buffers then fill.
    pub fn stall(&self) {
        self.shared.update(|s| s.reading = false);
    }

    pub fn resume(&self) {
        self.shared.update(|s| s.reading = true);
    }

    /// Close connection `index` from the endpoint side.
    pub fn close_connection(&self, index: usize) {
        self.shared.update(|s| {
            s.conns[index].killed = true;
            let _ = s.sockets[index].shutdown(Shutdown::Both);
        });
    }

    pub fn accepted(&self) -> usize {
        self.shared.lock().conns.len()
    }

    pub fn max_open(&self) -> usize {
        self.shared.lock().max_open
    }

    pub fn conns(&self) -> Vec<Conn> {
        self.shared.lock().conns.clone()
    }

    pub fn conn(&self, index: usize) -> Conn {
        self.shared.lock().conns[index].clone()
    }

    /// Waits up to `timeout` for `ready` to hold; returns whether it did.
    pub fn wait_until(&self, timeout: Duration, ready: impl Fn(&State) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        loop {
            if ready(&state) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// Waits until connection `index` exists and has ended.
    pub fn wait_ended(&self, index: usize, timeout: Duration) -> Conn {
        assert!(
            self.wait_until(timeout, |s| s
                .conns
                .get(index)
                .is_some_and(|c| c.end.is_some())),
            "connection {index} did not end: {:?}",
            self.shared
                .lock()
                .conns
                .get(index)
                .map(|c| (&c.end, c.messages.len()))
        );
        self.conn(index)
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        self.shared.update(|s| {
            s.stopping = true;
            for socket in &s.sockets {
                let _ = socket.shutdown(Shutdown::Both);
            }
        });
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        let threads = std::mem::take(&mut *self.threads.lock().unwrap());
        for thread in threads {
            let _ = thread.join();
        }
    }
}

fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    loop {
        if shared.lock().stopping {
            return;
        }
        match listener.accept() {
            Ok((socket, _)) => {
                // Accepted sockets inherit the listener's nonblocking flag on BSD.
                socket.set_nonblocking(false).unwrap();
                let clone = socket.try_clone().unwrap();
                let mut index = 0;
                shared.update(|s| {
                    index = s.conns.len();
                    s.conns.push(Conn::new());
                    s.sockets.push(clone);
                    s.open_now += 1;
                    s.max_open = s.max_open.max(s.open_now);
                });
                let serving = Arc::clone(&shared);
                let thread = std::thread::spawn(move || {
                    let end = serve(&serving, index, socket);
                    serving.update(|s| {
                        let conn = &mut s.conns[index];
                        conn.end = Some(if conn.killed {
                            End::ClosedByEndpoint
                        } else {
                            end
                        });
                        conn.ended_at = Some(Instant::now());
                        s.open_now -= 1;
                    });
                });
                threads.lock().unwrap().push(thread);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

/// Reads until `buf` is full or EOF; the count read.
fn read_full(socket: &mut TcpStream, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match socket.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn close(socket: &TcpStream) -> End {
    let _ = socket.shutdown(Shutdown::Both);
    End::ClosedByEndpoint
}

fn serve(shared: &Shared, index: usize, mut socket: TcpStream) -> End {
    let (policy, expected_token) = {
        let s = shared.lock();
        (s.policy, s.expected_token.clone())
    };
    if policy == Policy::DropOnAccept {
        return close(&socket);
    }
    let record = |bytes: &[u8], item: bool| {
        shared.update(|s| {
            let conn = &mut s.conns[index];
            conn.raw.extend_from_slice(bytes);
            if item {
                conn.item_bytes += bytes.len() as u64;
            }
        })
    };

    // The handshake: length prefix, bounded body, three known fields.
    let mut prefix = [0u8; 4];
    match read_full(&mut socket, &mut prefix) {
        Ok(4) => {}
        Ok(_) => return End::TornTail,
        Err(e) => return End::Error(e.kind()),
    }
    record(&prefix, false);
    let Ok(len) = checked_frame_len(prefix, MAX_HANDSHAKE_BYTES) else {
        let _ = socket.shutdown(Shutdown::Both);
        return End::Invalid("handshake over its limit".into());
    };
    let mut body = vec![0u8; len];
    match read_full(&mut socket, &mut body) {
        Ok(n) if n == len => {}
        Ok(n) => {
            record(&body[..n], false);
            return End::TornTail;
        }
        Err(e) => return End::Error(e.kind()),
    }
    record(&body, false);
    let handshake: Handshake = match serde_json::from_slice(&body) {
        Ok(handshake) => handshake,
        Err(_) => {
            let _ = socket.shutdown(Shutdown::Both);
            return End::Invalid("handshake is not the three-field object".into());
        }
    };
    let refused = expected_token
        .as_deref()
        .is_some_and(|expected| expected != handshake.token);
    shared.update(|s| s.conns[index].handshake = Some(handshake));
    if refused {
        let _ = socket.shutdown(Shutdown::Both);
        return End::Refused;
    }
    if policy == Policy::CloseAfterHandshake {
        return close(&socket);
    }

    loop {
        // The read gate: a stalled endpoint does not read, so the sender's writes back up.
        {
            let mut s = shared.lock();
            while !s.reading && !s.stopping {
                s = shared.changed.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            if s.stopping {
                drop(s);
                return close(&socket);
            }
        }
        let items = shared.lock().conns[index].messages.len();
        if policy == Policy::CloseAfterItems(items) {
            return close(&socket);
        }
        let mut prefix = [0u8; 4];
        match read_full(&mut socket, &mut prefix) {
            Ok(0) => return End::Eof,
            Ok(4) => {}
            Ok(n) => {
                record(&prefix[..n], true);
                return End::TornTail;
            }
            Err(e) => return End::Error(e.kind()),
        }
        record(&prefix, true);
        let Ok(len) = checked_frame_len(prefix, MAX_INGEST_FRAME_BYTES) else {
            let _ = socket.shutdown(Shutdown::Both);
            return End::Invalid("item over its limit".into());
        };
        if policy == Policy::CloseMidItem(items) {
            let mut half = vec![0u8; len / 2];
            if let Ok(n) = read_full(&mut socket, &mut half) {
                record(&half[..n], true);
            }
            return close(&socket);
        }
        let mut body = vec![0u8; len];
        match read_full(&mut socket, &mut body) {
            Ok(n) if n == len => {}
            Ok(n) => {
                record(&body[..n], true);
                return End::TornTail;
            }
            Err(e) => return End::Error(e.kind()),
        }
        record(&body, true);
        let message = match decode_ingest_message(&body) {
            Ok(message) => message,
            Err(e) => {
                let _ = socket.shutdown(Shutdown::Both);
                return End::Invalid(e.to_string());
            }
        };
        shared.update(|s| {
            let conn = &mut s.conns[index];
            conn.item_lens.push(4 + len);
            conn.messages.push(message);
        });
    }
}

/// Decodes the hex golden (whitespace ignored).
pub fn from_hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// A loopback TCP port that is bound but not listening. On macOS a SYN to it is dropped, so a
/// connect hangs until the caller's own timeout (not refused), and no other socket (another
/// test's `127.0.0.1:0` endpoint included) can take the port meanwhile.
/// [`ReservedPort::listen`] later turns the same socket into a listener.
pub struct ReservedPort {
    fd: OwnedFd,
    addr: SocketAddr,
}

impl ReservedPort {
    pub fn new() -> Self {
        // SAFETY: plain socket calls on a descriptor this function owns; every pointer passed
        // refers to a live, correctly sized local `sockaddr_in`.
        unsafe {
            let raw = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
            let fd = OwnedFd::from_raw_fd(raw);
            let mut sin: libc::sockaddr_in = std::mem::zeroed();
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            {
                sin.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
            }
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_port = 0;
            sin.sin_addr = libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            };
            let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            let bound = libc::bind(
                fd.as_raw_fd(),
                &sin as *const libc::sockaddr_in as *const libc::sockaddr,
                len,
            );
            assert_eq!(bound, 0, "bind: {}", std::io::Error::last_os_error());
            let mut out: libc::sockaddr_in = std::mem::zeroed();
            let mut out_len = len;
            let named = libc::getsockname(
                fd.as_raw_fd(),
                &mut out as *mut libc::sockaddr_in as *mut libc::sockaddr,
                &mut out_len,
            );
            assert_eq!(named, 0, "getsockname: {}", std::io::Error::last_os_error());
            let addr = SocketAddr::from(([127, 0, 0, 1], u16::from_be(out.sin_port)));
            ReservedPort { fd, addr }
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Starts listening on the reserved port.
    pub fn listen(self) -> TcpListener {
        // SAFETY: `listen` on the bound descriptor this value owns.
        let listening = unsafe { libc::listen(self.fd.as_raw_fd(), 128) };
        assert_eq!(listening, 0, "listen: {}", std::io::Error::last_os_error());
        TcpListener::from(self.fd)
    }
}

/// A loopback port every connect to is refused at once: the local port of a connection this
/// value keeps open. Nothing listens there, and the ephemeral allocator never hands the port
/// to another test's `127.0.0.1:0` bind while the connection lives.
pub struct RefusingPort {
    _held: (TcpStream, TcpStream),
    addr: SocketAddr,
}

impl RefusingPort {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let near = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (far, _) = listener.accept().unwrap();
        let addr = near.local_addr().unwrap();
        RefusingPort {
            _held: (near, far),
            addr,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}
