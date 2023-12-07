use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
};

use serde_with::DeserializeFromStr;

use crate::{log::*, parse};

#[derive(Debug, Clone, PartialEq, Eq, Hash, DeserializeFromStr)]
pub struct Listener {
    sock_addr: SocketAddr,
    device: Option<String>,
}

impl Listener {
    pub fn sock_addr(&self) -> SocketAddr {
        self.sock_addr
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }
}

impl Default for Listener {
    fn default() -> Self {
        Listener {
            sock_addr: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into(),
            device: None,
        }
    }
}

impl FromStr for Listener {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut addr = Some(s);
        let mut device = None;

        if let Some(s) = addr {
            if let Some(at_idx) = s.find('@') {
                device = Some(s[at_idx + 1..].to_string());
                addr = Some(&s[0..at_idx]);
            }
        }

        let sock_addr = addr
            .map(|addr| parse::parse_sock_addrs(addr).ok())
            .unwrap_or_default()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{} listen addr expect [::]:80 or 0.0.0.0:80", s),
                )
            })?;

        Ok(Listener { sock_addr, device })
    }
}

pub fn bind_to<T>(
    func: impl Fn(SocketAddr, Option<&str>, &str) -> io::Result<T>,
    sock_addr: SocketAddr,
    bind_device: Option<&str>,
    bind_type: &str,
) -> T {
    func(sock_addr, bind_device, bind_type).unwrap_or_else(|err| {
        panic!("cound not bind to {bind_type}: {sock_addr}, {err}");
    })
}

pub fn tcp(
    sock_addr: SocketAddr,
    bind_device: Option<&str>,
    bind_type: &str,
) -> io::Result<tokio::net::TcpListener> {
    let device_note = bind_device
        .map(|device| format!("@{device}"))
        .unwrap_or_default();

    debug!("binding {} to {:?}{}", bind_type, sock_addr, device_note);

    let tcp_listener = std::net::TcpListener::bind(sock_addr)?;
    {
        let sock_ref = socket2::SockRef::from(&tcp_listener);
        sock_ref.set_nonblocking(true)?;
        sock_ref.set_reuse_address(true)?;

        #[cfg(target_os = "macos")]
        sock_ref.set_reuse_port(true)?;

        #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
        if let Some(device) = bind_device {
            sock_ref.bind_device(Some(device.as_bytes()))?;
        }

        // TODO: bind_device for Windows/MacOS
    }

    let tcp_listener = tokio::net::TcpListener::from_std(tcp_listener)?;

    info!(
        "listening for {} on {:?}{}",
        bind_type,
        tcp_listener
            .local_addr()
            .expect("could not lookup local address"),
        device_note
    );

    Ok(tcp_listener)
}

pub fn udp(
    sock_addr: SocketAddr,
    bind_device: Option<&str>,
    bind_type: &str,
) -> io::Result<tokio::net::UdpSocket> {
    let device_note = bind_device
        .map(|device| format!("@{device}"))
        .unwrap_or_default();

    debug!("binding {} to {:?}{}", bind_type, sock_addr, device_note);
    let udp_socket = std::net::UdpSocket::bind(sock_addr)?;

    {
        let sock_ref = socket2::SockRef::from(&udp_socket);
        sock_ref.set_nonblocking(true)?;
        sock_ref.set_reuse_address(true)?;

        #[cfg(target_os = "macos")]
        sock_ref.set_reuse_port(true)?;

        #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
        if let Some(device) = bind_device {
            sock_ref.bind_device(Some(device.as_bytes()))?;
        }

        // TODO: bind_device for Windows/MacOS
    }

    let udp_socket = tokio::net::UdpSocket::from_std(udp_socket)?;

    info!(
        "listening for {} on {:?}{}",
        bind_type,
        udp_socket
            .local_addr()
            .expect("could not lookup local address"),
        device_note
    );

    Ok(udp_socket)
}
