use std::{
    cell::RefCell,
    collections::HashMap,
    io::{self, ErrorKind},
    mem,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd},
    ptr,
    time::{Duration, Instant},
};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpSocket, TcpStream, UdpSocket};

use crate::{
    log::*,
    net::{
        sys::{set_common_sockopt_after_connect, set_common_sockopt_for_connect, socket_bind_dual_stack},
        AddrFamily, ConnectOpts,
    },
};

pub(crate) async fn create_tcp_stream_impl(server_addr: SocketAddr, conn_opts: &ConnectOpts) -> io::Result<TcpStream> {
    let socket = if conn_opts.tcp.mptcp {
        create_mptcp_socket()?
    } else {
        match server_addr {
            SocketAddr::V4(..) => TcpSocket::new_v4()?,
            SocketAddr::V6(..) => TcpSocket::new_v6()?,
        }
    };

    // Binds to a specific network interface (device)
    if let Some(ref iface) = conn_opts.bind_interface {
        set_ip_bound_if(&socket, &server_addr, iface)?;
    }

    set_common_sockopt_for_connect(server_addr, &socket, conn_opts)?;

    let stream = socket.connect(server_addr).await?;

    set_common_sockopt_after_connect(&stream, conn_opts)?;

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
        cache.borrow_mut().insert(iface.to_owned(), (index, Instant::now()));
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

pub(crate) async fn bind_udp_socket_impl(bind_addr: &SocketAddr, conn_opts: &ConnectOpts) -> io::Result<UdpSocket> {
    let af: AddrFamily = From::from(bind_addr);

    let socket = if af != AddrFamily::IPv6 {
        UdpSocket::bind(bind_addr).await?
    } else {
        let socket = Socket::new(Domain::for_address(*bind_addr), Type::DGRAM, Some(Protocol::UDP))?;
        socket_bind_dual_stack(&socket, &bind_addr, false)?;

        // UdpSocket::from_std requires socket to be non-blocking
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket.into())?
    };

    // Set IP_BOUND_IF for BSD-like
    if let Some(ref iface) = conn_opts.bind_interface {
        set_ip_bound_if(&socket, bind_addr, iface)?;
    }

    Ok(socket)
}

pub(crate) async fn create_udp_socket_impl(af: AddrFamily, conn_opts: &ConnectOpts) -> io::Result<UdpSocket> {
    let bind_addr = match (af, conn_opts.bind_local_addr) {
        (AddrFamily::IPv4, Some(IpAddr::V4(ip))) => SocketAddr::new(ip.into(), 0),
        (AddrFamily::IPv6, Some(IpAddr::V6(ip))) => SocketAddr::new(ip.into(), 0),
        (AddrFamily::IPv4, _) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        (AddrFamily::IPv6, _) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };

    bind_udp_socket_impl(&bind_addr, conn_opts).await
}
