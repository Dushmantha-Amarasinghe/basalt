//! Socket setup, with the settings Phase 0 measured.
//!
//! The receive buffer is the single most valuable line in this crate. Measured
//! end to end over the real link:
//!
//! | Buffer | Throughput |
//! |---|---|
//! | 64 KiB | 7.4 MB/s |
//! | 256 KiB | 17.4 MB/s |
//! | 1 MiB | 22.1 MB/s |
//! | 2 MiB | 22.7 MB/s |
//! | 8 MiB | 20.1 MB/s (bufferbloat) |
//!
//! A threefold spread from one socket option, with the Windows default landing
//! in the middle at 20.4 MB/s.

use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpSocket, TcpStream};

use crate::{NetError, Result};

/// Receive buffer for every connection: inside the flat optimum, clear of the
/// regression at 8 MiB.
pub const RECV_BUFFER: u32 = 2 * 1024 * 1024;

/// Default port. Clear of anything common, and the same one the Phase 0
/// harness used.
pub const DEFAULT_PORT: u16 = 7742;

/// Connects with the measured tuning applied.
///
/// `SO_RCVBUF` has to be set *before* connecting, because it determines the
/// window scale negotiated in the handshake. Setting it afterwards resizes the
/// buffer but leaves the scale where it was, silently capping how much data can
/// be in flight — the option appears to work and does nothing. That is why this
/// builds the socket by hand instead of using `TcpStream::connect`.
pub async fn connect(addr: SocketAddr) -> Result<TcpStream> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    // Best effort: an OS is free to clamp or ignore the request, and there is
    // nothing useful to do about it if it does.
    let _ = socket.set_recv_buffer_size(RECV_BUFFER);

    let stream = socket.connect(addr).await?;
    tune(&stream);
    Ok(stream)
}

/// Binds a listener for the host.
pub async fn listen(addr: SocketAddr) -> Result<TcpListener> {
    TcpListener::bind(addr).await.map_err(NetError::Io)
}

/// Per-connection tuning applied on both sides after the socket exists.
///
/// `TCP_NODELAY` matters because browsing and the pairing handshake are
/// latency-bound request/response exchanges, and Nagle's algorithm would add up
/// to 40 ms to each one. Bulk transfers write in megabyte chunks and never form
/// the small segments Nagle exists to coalesce, so there is nothing to lose.
pub fn tune(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
}

/// Resolves a host string to a socket address, defaulting the port.
///
/// Accepts `192.168.1.10`, `192.168.1.10:7742`, `laptop-b`, and bracketed IPv6.
pub async fn resolve(host: &str, default_port: u16) -> Result<SocketAddr> {
    let target = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        // A bare IPv6 address needs brackets before a port can be appended.
        format!("[{host}]:{default_port}")
    } else if has_explicit_port(host) {
        host.to_string()
    } else {
        format!("{host}:{default_port}")
    };

    tokio::net::lookup_host(&target)
        .await
        .map_err(|e| NetError::Protocol(format!("could not resolve {target}: {e}")))?
        .next()
        .ok_or_else(|| NetError::Protocol(format!("{target} resolved to no addresses")))
}

/// Whether a host string already carries a `:port`.
///
/// The awkward case is IPv6, which is full of colons. A bracketed address has
/// an explicit port only when a colon follows the closing bracket.
fn has_explicit_port(host: &str) -> bool {
    match host.rfind(']') {
        Some(bracket) => host[bracket..].contains(':'),
        None => host.matches(':').count() == 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_ports_are_recognised() {
        assert!(has_explicit_port("192.168.1.10:7742"));
        assert!(has_explicit_port("laptop-b:7742"));
        assert!(has_explicit_port("[fe80::1]:7742"));

        assert!(!has_explicit_port("192.168.1.10"));
        assert!(!has_explicit_port("laptop-b"));
        assert!(!has_explicit_port("[fe80::1]"));
        assert!(
            !has_explicit_port("fe80::1:2:3"),
            "a bare IPv6 address is all colons and no port"
        );
    }

    #[tokio::test]
    async fn resolving_applies_the_default_port() {
        let addr = resolve("127.0.0.1", 7742).await.unwrap();
        assert_eq!(addr.port(), 7742);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[tokio::test]
    async fn an_explicit_port_wins_over_the_default() {
        let addr = resolve("127.0.0.1:9000", 7742).await.unwrap();
        assert_eq!(addr.port(), 9000);
    }

    #[tokio::test]
    async fn a_bare_ipv6_address_gets_bracketed() {
        let addr = resolve("::1", 7742).await.unwrap();
        assert_eq!(addr.port(), 7742);
        assert!(addr.is_ipv6());
    }

    #[tokio::test]
    async fn an_unresolvable_name_is_an_error_not_a_hang() {
        assert!(
            resolve("this-host-does-not-exist.invalid", 7742)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_bound_listener_reports_the_port_the_os_chose() {
        let listener = listen("127.0.0.1:0".parse().unwrap()).await.unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), 0);
    }

    #[tokio::test]
    async fn connecting_applies_nodelay() {
        let listener = listen("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move { listener.accept().await });

        let stream = connect(addr).await.unwrap();
        assert!(stream.nodelay().unwrap());
        accept.await.unwrap().unwrap();
    }
}
