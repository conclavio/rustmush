//! Listeners (doc 29 §9.2, doc 33 §4.4).
//!
//! Each generation binds every configured address itself, with
//! `SO_REUSEPORT` (Unix) so two generations share the port while one drains,
//! and `IPV6_V6ONLY` so an IPv6 listener never also takes IPv4. Closing a
//! listener in a reuseport group resets the connections waiting in *its*
//! accept queue unless `net.ipv4.tcp_migrate_req = 1`, which
//! [`migrate_req_enabled`] checks at startup.

use std::net::SocketAddr;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;

/// The accept backlog.
pub const BACKLOG: i32 = 1024;

/// Bind one address.
pub fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    #[cfg(unix)]
    {
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(BACKLOG)?;
    TcpListener::from_std(socket.into())
}

/// Whether `net.ipv4.tcp_migrate_req` is on. `None` when the file is absent
/// (not Linux, or a kernel older than 5.14).
#[must_use]
pub fn migrate_req_enabled() -> Option<bool> {
    std::fs::read_to_string("/proc/sys/net/ipv4/tcp_migrate_req")
        .ok()
        .map(|s| s.trim() == "1")
}

/// The startup warning when the sysctl is off
/// (`deploy/sshdoorway/99-sshdoorway.conf` installs it box-wide).
pub fn warn_if_migrate_req_off() {
    if migrate_req_enabled() == Some(false) {
        tracing::warn!(
            "net.ipv4.tcp_migrate_req is 0: connections queued on a draining generation will be reset"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binds_loopback_v4() {
        let l = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(l.local_addr().unwrap().port() > 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn two_generations_share_one_port() {
        let a = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = a.local_addr().unwrap();
        let b = bind(addr).unwrap();
        assert_eq!(b.local_addr().unwrap(), addr);
    }
}
