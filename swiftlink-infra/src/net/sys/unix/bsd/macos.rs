use std::{
    cell::RefCell,
    collections::HashMap,
    io::{self, ErrorKind},
    mem,
    net::SocketAddr,
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd},
    ptr,
    time::{Duration, Instant},
};

use socket2::Socket;
use tokio::net::{TcpSocket, TcpStream};
use tracing::error;

use crate::net::{
    sys::{set_common_sockopt_after_connect, set_common_sockopt_for_connect},
    ConnectOpts,
};

pub(crate) async fn connect_tcp_with_opts_impl(
    addr: SocketAddr,
    opts: &ConnectOpts,
) -> io::Result<TcpStream> {
    let socket = if opts.tcp.mptcp {
        create_mptcp_socket()?
    } else {
        match addr {
            SocketAddr::V4(..) => TcpSocket::new_v4()?,
            SocketAddr::V6(..) => TcpSocket::new_v6()?,
        }
    };

    // Binds to a specific network interface (device)
    if let Some(ref iface) = opts.bind_interface {
        set_ip_bound_if(&socket, &addr, iface)?;
    }

    set_common_sockopt_for_connect(addr, &socket, opts)?;

    let stream = socket.connect(addr).await?;

    set_common_sockopt_after_connect(&stream, opts)?;

    Ok(stream)
}

fn create_mptcp_socket() -> io::Result<TcpSocket> {
    // https://opensource.apple.com/source/xnu/xnu-4570.41.2/bsd/sys/socket.h.auto.html
    const AF_MULTIPATH: libc::c_int = 39;

    unsafe {
        let fd = libc::socket(AF_MULTIPATH, libc::SOCK_STREAM, libc::IPPROTO_TCP);
        let socket = Socket::from_raw_fd(fd);
        socket.set_nonblocking(true)?;
        Ok(TcpSocket::from_raw_fd(socket.into_raw_fd()))
    }
}

fn find_interface_index_cached(iface: &str) -> io::Result<u32> {
    const INDEX_EXPIRE_DURATION: Duration = Duration::from_secs(5);

    thread_local! {
        static INTERFACE_INDEX_CACHE: RefCell<HashMap<String,(u32,Instant)>> = RefCell::new(HashMap::new());
    }

    let cache_index = INTERFACE_INDEX_CACHE.with(|cache| cache.borrow().get(iface).cloned());
    if let Some((idx, insert_time)) = cache_index {
        // short-path, cache hit for most cases
        let now = Instant::now();
        if now - insert_time < INDEX_EXPIRE_DURATION {
            return Ok(idx);
        }
    }

    let index = unsafe {
        let mut ciface = [0u8; libc::IFNAMSIZ];
        if iface.len() >= ciface.len() {
            return Err(ErrorKind::InvalidInput.into());
        }

        let iface_bytes = iface.as_bytes();
        ptr::copy_nonoverlapping(iface_bytes.as_ptr(), ciface.as_mut_ptr(), iface_bytes.len());

        libc::if_nametoindex(ciface.as_ptr() as *const libc::c_char)
    };

    if index == 0 {
        let err = io::Error::last_os_error();
        error!("if_nametoindex ifname: {} error: {}", iface, err);
        return Err(err);
    }

    INTERFACE_INDEX_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(iface.to_owned(), (index, Instant::now()));
    });

    Ok(index)
}

fn set_ip_bound_if<S: AsRawFd>(socket: &S, addr: &SocketAddr, iface: &str) -> io::Result<()> {
    const IP_BOUND_IF: libc::c_int = 25; // bsd/netinet/in.h
    const IPV6_BOUND_IF: libc::c_int = 125; // bsd/netinet6/in6.h

    unsafe {
        let index = find_interface_index_cached(iface)?;

        let ret = match addr {
            SocketAddr::V4(..) => libc::setsockopt(
                socket.as_raw_fd(),
                libc::IPPROTO_IP,
                IP_BOUND_IF,
                &index as *const _ as *const _,
                mem::size_of_val(&index) as libc::socklen_t,
            ),
            SocketAddr::V6(..) => libc::setsockopt(
                socket.as_raw_fd(),
                libc::IPPROTO_IPV6,
                IPV6_BOUND_IF,
                &index as *const _ as *const _,
                mem::size_of_val(&index) as libc::socklen_t,
            ),
        };

        if ret < 0 {
            let err = io::Error::last_os_error();
            error!(
                "set IF_BOUND_IF/IPV6_BOUND_IF ifname: {} ifindex: {} error: {}",
                iface, index, err
            );
            return Err(err);
        }
    }

    Ok(())
}
