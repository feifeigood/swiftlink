use cfg_if::cfg_if;
use socket2::Socket;
use std::{
    io, mem,
    net::SocketAddr,
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd},
};
use tokio::net::{TcpSocket, TcpStream};

use crate::{
    log,
    net::{sys::set_common_sockopt_for_connect, ConnectOpts},
};

use super::set_common_sockopt_after_connect;

pub(crate) async fn connect_tcp_with_opts_impl(
    addr: SocketAddr,
    opts: &ConnectOpts,
) -> io::Result<TcpStream> {
    let socket = if opts.tcp.mptcp {
        create_mptcp_socket(&addr)?
    } else {
        match addr {
            SocketAddr::V4(..) => TcpSocket::new_v4()?,
            SocketAddr::V6(..) => TcpSocket::new_v6()?,
        }
    };

    // Any traffic to localhost should not be protected
    // This is a workaround for VPNService
    #[cfg(target_os = "android")]
    if !addr.ip().is_loopback() {
        use std::{io::ErrorKind, time::Duration};
        use tokio::time;

        if let Some(ref path) = opts.vpn_protect_path {
            // RPC calls to `VpnService.protect()`
            // Timeout in 3 seconds like shadowsocks-libev
            match time::timeout(
                Duration::from_secs(3),
                vpn_protect(path, socket.as_raw_fd()),
            )
            .await
            {
                Ok(Ok(..)) => {}
                Ok(Err(err)) => return Err(err),
                Err(..) => return Err(io::Error::new(ErrorKind::TimedOut, "protect() timeout")),
            }
        }
    }

    // Set SO_MARK for mark-based routing on Linux (since 2.6.25)
    // NOTE: This will require CAP_NET_ADMIN capability (root in most cases)
    if let Some(mark) = opts.fwmark {
        let ret = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_MARK,
                &mark as *const _ as *const _,
                mem::size_of_val(&mark) as libc::socklen_t,
            )
        };
        if ret != 0 {
            let err = io::Error::last_os_error();
            log::error!("set SO_MARK error: {}", err);
            return Err(err);
        }
    }

    // Set SO_BINDTODEVICE for binding to a specific interface
    if let Some(ref iface) = opts.bind_interface {
        set_bindtodevice(&socket, iface)?;
    }

    set_common_sockopt_for_connect(addr, &socket, opts)?;

    let stream = socket.connect(addr).await?;

    set_common_sockopt_after_connect(&stream, opts)?;

    Ok(stream)
}

fn create_mptcp_socket(bind_addr: &SocketAddr) -> io::Result<TcpSocket> {
    unsafe {
        let family = match bind_addr {
            SocketAddr::V4(..) => libc::AF_INET,
            SocketAddr::V6(..) => libc::AF_INET6,
        };
        let fd = libc::socket(family, libc::SOCK_STREAM, libc::IPPROTO_MPTCP);
        let socket = Socket::from_raw_fd(fd);
        socket.set_nonblocking(true)?;
        Ok(TcpSocket::from_raw_fd(socket.into_raw_fd()))
    }
}

fn set_bindtodevice<S: AsRawFd>(socket: &S, iface: &str) -> io::Result<()> {
    let iface_bytes = iface.as_bytes();

    unsafe {
        let ret = libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            iface_bytes.as_ptr() as *const _ as *const libc::c_void,
            iface_bytes.len() as libc::socklen_t,
        );

        if ret != 0 {
            let err = io::Error::last_os_error();
            log::error!("set SO_BINDTODEVICE error: {}", err);
            return Err(err);
        }
    }

    Ok(())
}

cfg_if! {
    if #[cfg(target_os = "android")] {
        use std::{
            io::ErrorKind,
            path::Path,
        };
        use tokio::io::AsyncReadExt;

        use super::uds::UnixStream;

        /// This is a RPC for Android to `protect()` socket for connecting to remote servers
        ///
        /// https://developer.android.com/reference/android/net/VpnService#protect(java.net.Socket)
        ///
        /// More detail could be found in [shadowsocks-android](https://github.com/shadowsocks/shadowsocks-android) project.
        async fn vpn_protect<P: AsRef<Path>>(protect_path: P, fd: RawFd) -> io::Result<()> {
            let mut stream = UnixStream::connect(protect_path).await?;

            // send fds
            let dummy: [u8; 1] = [1];
            let fds: [RawFd; 1] = [fd];
            stream.send_with_fd(&dummy, &fds).await?;

            // receive the return value
            let mut response = [0; 1];
            stream.read_exact(&mut response).await?;

            if response[0] == 0xFF {
                return Err(io::Error::new(ErrorKind::Other, "protect() failed"));
            }

            Ok(())
        }
    }
}
