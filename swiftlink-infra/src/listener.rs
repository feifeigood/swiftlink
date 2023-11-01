use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
};

use enum_dispatch::enum_dispatch;
use serde_with::DeserializeFromStr;

use crate::{log::*, parse};

#[enum_dispatch(IListener)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, DeserializeFromStr)]
pub enum Listener {
    Udp(UdpListener),
    Tcp(TcpListener),
}

#[enum_dispatch]
pub trait IListener {
    fn listen(&self) -> ListenerAddress;
    fn mut_listen(&mut self) -> &mut ListenerAddress;
    fn port(&self) -> u16;
    fn device(&self) -> Option<&str>;
    fn server_opts(&self) -> &ServerOpts;
    fn sock_addr(&self) -> SocketAddr {
        match self.listen() {
            ListenerAddress::Localhost => {
                SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port()).into()
            }
            ListenerAddress::Any => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.port()).into(),
            ListenerAddress::V4(ip) => (ip, self.port()).into(),
            ListenerAddress::V6(ip) => (ip, self.port()).into(),
        }
    }
}

/// server bind ip and port
/// bind udp server
///   bind [IP]:[port]@device -udp
/// bind tcp server
///   bind [IP]:[port]@device
impl FromStr for Listener {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = parse::split_options(s, ' ');

        let mut addr = None;
        let mut device = None;
        let mut server_kind = None;

        while let Some(part) = parts.next() {
            if part.starts_with('-') {
                match part {
                    "-udp" | "--udp" => {
                        server_kind = Some("udp");
                    }
                    "-tcp" | "--tcp" => {
                        server_kind = Some("tcp");
                    }
                    opt => {
                        warn!("unknown option: {}", opt);
                    }
                }
            } else if addr.is_none() {
                addr = Some(part);
            } else {
                error!("unexpected options: {}", part);
            }
        }

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

        let listener = match server_kind {
            Some(kind) if kind == "udp" => Self::Udp(UdpListener {
                listen: sock_addr.ip().into(),
                port: sock_addr.port(),
                device,
                opts: ServerOpts::default(),
            }),
            _ => Self::Tcp(TcpListener {
                listen: sock_addr.ip().into(),
                port: sock_addr.port(),
                device,
                opts: ServerOpts::default(),
            }),
        };

        Ok(listener)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UdpListener {
    /// listen adress
    pub listen: ListenerAddress,
    /// listen port
    pub port: u16,
    /// bind network device.
    pub device: Option<String>,
    /// the server options
    pub opts: ServerOpts,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TcpListener {
    /// listen adress
    pub listen: ListenerAddress,
    /// listen port
    pub port: u16,
    /// bind network device.
    pub device: Option<String>,
    /// the server options
    pub opts: ServerOpts,
}

macro_rules! impl_listener {
    ($($name:ident),+) => {
        $(
            impl IListener for $name {
                fn listen(&self) -> ListenerAddress {
                    self.listen
                }
                fn mut_listen(&mut self) -> &mut ListenerAddress {
                    &mut self.listen
                }

                fn port(&self) -> u16 {
                    self.port
                }
                fn device(&self) -> Option<&str> {
                    self.device.as_deref()
                }

                fn server_opts(&self) -> &ServerOpts {
                    &self.opts
                }
            }
        )+
    }
}

impl_listener!(UdpListener, TcpListener);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListenerAddress {
    Localhost,
    Any,
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl From<IpAddr> for ListenerAddress {
    fn from(value: IpAddr) -> Self {
        match value {
            IpAddr::V4(ip) => ListenerAddress::V4(ip),
            IpAddr::V6(ip) => ListenerAddress::V6(ip),
        }
    }
}

impl ListenerAddress {
    // Returns the ip addr of this [`ListenerAddress`]
    fn ip_addr(self) -> IpAddr {
        match self {
            ListenerAddress::Localhost => IpAddr::V4(Ipv4Addr::LOCALHOST),
            ListenerAddress::Any => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            ListenerAddress::V4(ip) => ip.into(),
            ListenerAddress::V6(ip) => ip.into(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ServerOpts {}

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

#[cfg(test)]
mod tests {}
