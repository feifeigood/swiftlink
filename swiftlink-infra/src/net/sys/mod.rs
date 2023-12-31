use std::{
    io::{self, ErrorKind},
    net::{IpAddr, SocketAddr},
};

use cfg_if::cfg_if;
use socket2::{SockAddr, Socket};
use tokio::net::TcpSocket;

use crate::{log::*, net::options::ConnectOpts};

cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub use self::unix::*;
    } else if #[cfg(windows)] {
        mod windows;
        pub use self::windows::*;
    }
}

fn set_common_sockopt_for_connect(addr: SocketAddr, socket: &TcpSocket, conn_opts: &ConnectOpts) -> io::Result<()> {
    // Binds to IP address
    if let Some(ip) = conn_opts.bind_local_addr {
        match (ip, addr.ip()) {
            (IpAddr::V4(..), IpAddr::V4(..)) => {
                socket.bind(SocketAddr::new(ip, 0))?;
            }
            (IpAddr::V6(..), IpAddr::V6(..)) => {
                socket.bind(SocketAddr::new(ip, 0))?;
            }
            _ => {}
        }
    }

    // Set `SO_SNDBUF`
    if let Some(buf_size) = conn_opts.tcp.send_buffer_size {
        socket.set_send_buffer_size(buf_size)?;
    }

    // Set `SO_RCVBUF`
    if let Some(buf_size) = conn_opts.tcp.recv_buffer_size {
        socket.set_recv_buffer_size(buf_size)?;
    }

    Ok(())
}

/// Try to call `bind()` with dual-stack enabled.
///
/// Users have to ensure that `addr` is a dual-stack inbound address (`::`) when `ipv6_only` is `false`.
#[cfg(unix)]
pub fn socket_bind_dual_stack<S>(socket: &S, addr: &SocketAddr, ipv6_only: bool) -> io::Result<()>
where
    S: std::os::unix::io::AsRawFd,
{
    use std::os::unix::prelude::{FromRawFd, IntoRawFd};

    let fd = socket.as_raw_fd();

    let sock = unsafe { Socket::from_raw_fd(fd) };
    let result = socket_bind_dual_stack_inner(&sock, addr, ipv6_only);
    sock.into_raw_fd();

    result
}

/// Try to call `bind()` with dual-stack enabled.
///
/// Users have to ensure that `addr` is a dual-stack inbound address (`::`) when `ipv6_only` is `false`.
#[cfg(windows)]
pub fn socket_bind_dual_stack<S>(socket: &S, addr: &SocketAddr, ipv6_only: bool) -> io::Result<()>
where
    S: std::os::windows::io::AsRawSocket,
{
    use std::os::windows::prelude::{FromRawSocket, IntoRawSocket};

    let handle = socket.as_raw_socket();

    let sock = unsafe { Socket::from_raw_socket(handle) };
    let result = socket_bind_dual_stack_inner(&sock, addr, ipv6_only);
    sock.into_raw_socket();

    result
}

fn socket_bind_dual_stack_inner(socket: &Socket, addr: &SocketAddr, ipv6_only: bool) -> io::Result<()> {
    let saddr = SockAddr::from(*addr);

    if ipv6_only {
        // Requested to set IPV6_V6ONLY
        socket.set_only_v6(true)?;
        socket.bind(&saddr)?;
    } else {
        if let Err(err) = socket.set_only_v6(false) {
            warn!("failed to set IPV6_V6ONLY: false for socket, error: {}", err);

            // This is not a fatal error, just warn and skip
        }

        match socket.bind(&saddr) {
            Ok(..) => {}
            Err(ref err) if err.kind() == ErrorKind::AddrInUse => {
                // This is probably 0.0.0.0 with the same port has already been occupied
                debug!(
                    "0.0.0.0:{} may have already been occupied, retry with IPV6_V6ONLY",
                    addr.port()
                );

                if let Err(err) = socket.set_only_v6(true) {
                    warn!("failed to set IPV6_V6ONLY: true for socket, error: {}", err);

                    // This is not a fatal error, just warn and skip
                }
                socket.bind(&saddr)?;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}
