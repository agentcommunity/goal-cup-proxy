//! Loopback ports for tests. This is the only place a test binds port 0 (PRD-14 section 4): the
//! old flake was "bind :0, drop, bind the number again", which another process can take in
//! between. Both helpers keep their socket for the test's life, so the port stays theirs.

use socket2::{Domain, Socket, Type};
use std::net::{SocketAddr, TcpListener};

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// A listening socket on 127.0.0.1, held until dropped. For tokio, call
/// `set_nonblocking(true)` and `tokio::net::TcpListener::from_std`.
pub fn held() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind(loopback()).expect("bind a loopback listener");
    let addr = listener.local_addr().expect("listener address");
    (listener, addr)
}

/// A socket bound on 127.0.0.1 that never listens: a connect to its address is refused, and no
/// other process can take the port while the socket is held.
pub fn refusing() -> (Socket, SocketAddr) {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("create a socket");
    socket
        .bind(&loopback().into())
        .expect("bind a loopback socket");
    let addr = socket
        .local_addr()
        .expect("socket address")
        .as_socket()
        .expect("an IP socket address");
    (socket, addr)
}
