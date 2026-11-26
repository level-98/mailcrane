//! Listening on every address that `localhost` can resolve to.
//!
//! Tests connect to the fake servers as `localhost`, because that is the name
//! in their certificate. `localhost` resolves to `::1` as well as
//! `127.0.0.1`, and Windows tries `::1` first. A server that listens only on
//! `127.0.0.1:<port>` leaves `[::1]:<port>` free for any other process, and
//! a client that reaches that process fails with a TLS error instead of
//! talking to the fake server. So the fake servers listen on both loopback
//! addresses with the same port.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::{TcpListener, TcpStream};

/// How many ports to try before giving up on finding one that is free on
/// both loopback addresses.
const ATTEMPTS: usize = 32;

pub(crate) struct Loopback {
    v4: TcpListener,
    /// `None` only when this machine has no IPv6 loopback, in which case
    /// `localhost` cannot reach `::1` either.
    v6: Option<TcpListener>,
    addr: SocketAddr,
}

impl Loopback {
    /// Binds `127.0.0.1` and `::1` to the same free port.
    pub(crate) async fn bind() -> Result<Self, String> {
        for _ in 0..ATTEMPTS {
            let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|error| error.to_string())?;
            let addr = v4.local_addr().map_err(|error| error.to_string())?;
            match TcpListener::bind((Ipv6Addr::LOCALHOST, addr.port())).await {
                Ok(v6) => {
                    return Ok(Self {
                        v4,
                        v6: Some(v6),
                        addr,
                    })
                }
                // Something else has `[::1]:<port>`, or Windows reserves the
                // port: try another one.
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
                    ) => {}
                // No IPv6 loopback on this machine.
                Err(_) => return Ok(Self { v4, v6: None, addr }),
            }
        }
        Err("no port free on both 127.0.0.1 and ::1".into())
    }

    /// Binds `127.0.0.1` and, when it is free, `::1` to `port` (for
    /// `cargo xtask dev-mail`, which needs ports the emulator setup can name).
    pub(crate) async fn bind_port(port: u16) -> Result<Self, String> {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| format!("127.0.0.1:{port}: {error}"))?;
        let addr = v4.local_addr().map_err(|error| error.to_string())?;
        let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok();
        Ok(Self { v4, v6, addr })
    }

    /// Binds a fixed port when one is given, otherwise any free one.
    pub(crate) async fn bind_or_any(port: Option<u16>) -> Result<Self, String> {
        match port {
            Some(port) => Self::bind_port(port).await,
            None => Self::bind().await,
        }
    }

    /// The `127.0.0.1` address; the port is the same on `::1`.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The next connection on either address.
    pub(crate) async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        match &self.v6 {
            // `TcpListener::accept` is cancel safe, so losing the race drops
            // no connection.
            Some(v6) => tokio::select! {
                accepted = self.v4.accept() => accepted,
                accepted = v6.accept() => accepted,
            },
            None => self.v4.accept().await,
        }
    }
}
