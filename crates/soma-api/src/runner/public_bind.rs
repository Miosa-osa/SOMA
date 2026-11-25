//! Binding the public TCP listener and the HTTP/3 endpoint on one port.
//!
//! HTTP/3 binds the UDP port with the number TCP got. With a configured port that is one fixed
//! number, and a bind that fails is a configuration that cannot serve. With port `0` the system
//! chooses the TCP port alone, and the UDP port with that number may already belong to another
//! socket on the host: TCP and UDP ports are separate spaces. The runner then asks for another
//! TCP port rather than failing a start its configuration did not make impossible.

use std::{io, net::SocketAddr};

use tokio::net::TcpListener;

/// How many system-chosen ports a start tries before it reports the last UDP bind failure.
const EPHEMERAL_ATTEMPTS: usize = 16;

/// Binds `listen` over TCP, then `bind_udp` on the port TCP got.
///
/// # Errors
///
/// Returns the TCP bind failure, or the UDP one: at once for a configured port, and after
/// [`EPHEMERAL_ATTEMPTS`] system-chosen ports were all taken over UDP for port `0`.
pub(super) async fn bind_pair<Q>(
    listen: SocketAddr,
    mut bind_udp: impl FnMut(SocketAddr) -> io::Result<Q>,
) -> io::Result<(TcpListener, SocketAddr, Q)> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let tcp = TcpListener::bind(listen).await?;
        let address = tcp.local_addr()?;
        match bind_udp(address) {
            Ok(udp) => return Ok((tcp, address, udp)),
            Err(error)
                if listen.port() == 0
                    && error.kind() == io::ErrorKind::AddrInUse
                    && attempts < EPHEMERAL_ATTEMPTS => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, net::SocketAddr};

    use super::{EPHEMERAL_ATTEMPTS, bind_pair};

    fn taken() -> io::Error {
        io::Error::from(io::ErrorKind::AddrInUse)
    }

    /// The CI flake this answers: the UDP twin of a system-chosen TCP port was held elsewhere.
    #[tokio::test]
    async fn a_chosen_port_whose_udp_twin_is_taken_is_chosen_again() {
        let mut refused = 0;
        let (_tcp, address, udp) = bind_pair("127.0.0.1:0".parse().expect("address"), |address| {
            if refused < 2 {
                refused += 1;
                Err(taken())
            } else {
                Ok(address)
            }
        })
        .await
        .expect("a third port binds");
        assert_eq!(refused, 2);
        assert_eq!(udp, address, "UDP is bound on the port TCP got");
    }

    #[tokio::test]
    async fn a_configured_port_whose_udp_twin_is_taken_fails_at_once() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = held.local_addr().expect("address").port();
        drop(held);
        let listen = SocketAddr::from(([127, 0, 0, 1], port));
        let mut calls = 0;
        let error = bind_pair::<()>(listen, |_| {
            calls += 1;
            Err(taken())
        })
        .await
        .expect_err("a fixed port is not chosen again");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn chosen_ports_stop_after_the_attempt_limit() {
        let mut calls = 0;
        let error = bind_pair::<()>("127.0.0.1:0".parse().expect("address"), |_| {
            calls += 1;
            Err(taken())
        })
        .await
        .expect_err("every twin was taken");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(calls, EPHEMERAL_ATTEMPTS);
    }
}
