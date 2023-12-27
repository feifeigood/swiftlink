use std::{
    io,
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd},
};

use cfg_if::cfg_if;
use socket2::{Socket, TcpKeepalive};

use crate::net::{options::TcpSocketOpts, ConnectOpts};

cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod linux;
        pub use self::linux::*;
    } else if #[cfg(any(target_os = "freebsd",
                        target_os = "openbsd",
                        target_os = "netbsd",
                        target_os = "dragonfly",
                        target_os = "macos",
                        target_os = "ios",
                        target_os = "watchos",
                        target_os = "tvos"))] {
        mod bsd;
        pub use self::bsd::*;
    } else {
        mod others;
        pub use self::others::*;
    }
}

#[inline]
fn set_tcp_keepalive(socket: &Socket, tcp: &TcpSocketOpts) -> io::Result<()> {
    if let Some(intv) = tcp.keepalive {
        #[allow(unused_mut)]
        let mut keepalive = TcpKeepalive::new().with_time(intv);

        #[cfg(any(
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "netbsd",
            target_vendor = "apple",
        ))]
        {
            keepalive = keepalive.with_interval(intv);
        }

        cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "android"))] {
                // FIXME: Linux Kernel doesn't support setting TCP Keep Alive. (MPTCP)
                // SO_KEEPALIVE works fine. But TCP_KEEPIDLE, TCP_KEEPINTV are not supported.
                // https://github.com/multipath-tcp/mptcp_net-next/issues/383
                // https://github.com/multipath-tcp/mptcp_net-next/issues/353
                if let Err(err) = socket.set_tcp_keepalive(&keepalive) {
                    log::debug!("set TCP keep-alive with time & interval failed with error: {:?}", err);

                    // Try again without time & interval
                    let keepalive = TcpKeepalive::new();
                    socket.set_tcp_keepalive(&keepalive)?;
                }
            } else {
                socket.set_tcp_keepalive(&keepalive)?;
            }
        }
    }

    Ok(())
}

#[inline(always)]
fn socket_call_warp<S: AsRawFd, F: FnOnce(&Socket) -> io::Result<()>>(stream: &S, f: F) -> io::Result<()> {
    let socket = unsafe { Socket::from_raw_fd(stream.as_raw_fd()) };
    let result = f(&socket);
    let _ = socket.into_raw_fd();
    result
}

pub fn set_common_sockopt_after_connect<S: AsRawFd>(stream: &S, opts: &ConnectOpts) -> io::Result<()> {
    socket_call_warp(stream, |socket| set_common_sockopt_after_connect_impl(socket, opts))
}

fn set_common_sockopt_after_connect_impl(socket: &Socket, opts: &ConnectOpts) -> io::Result<()> {
    if opts.tcp.nodelay {
        socket.set_nodelay(true)?;
    }

    set_tcp_keepalive(socket, &opts.tcp)?;

    Ok(())
}
