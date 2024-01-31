//! Network utilities for the swiftlink.

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tokio::io::{AsyncRead, AsyncReadExt};

mod sys;
pub mod tcp;
mod timeout_stream;
pub mod udp;

mod options;
pub use options::{ConnectOpts, TcpSocketOpts};

/// The maximum UDP payload size (defined in the original shadowsocks Python)
///
/// *I cannot find any references about why clowwindy used this value as the maximum
/// Socks5 UDP ASSOCIATE packet size. The only thing I can find is
/// [here](http://support.microsoft.com/kb/822061/)*
pub const MAXIMUM_UDP_PAYLOAD_SIZE: usize = 65536;

/// Address family `AF_INET`, `AF_INET6`
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddrFamily {
    /// `AF_INET`
    IPv4,
    /// `AF_INET6`
    IPv6,
}

impl From<&SocketAddr> for AddrFamily {
    fn from(addr: &SocketAddr) -> AddrFamily {
        match *addr {
            SocketAddr::V4(..) => AddrFamily::IPv4,
            SocketAddr::V6(..) => AddrFamily::IPv6,
        }
    }
}

impl From<SocketAddr> for AddrFamily {
    fn from(addr: SocketAddr) -> AddrFamily {
        match addr {
            SocketAddr::V4(..) => AddrFamily::IPv4,
            SocketAddr::V6(..) => AddrFamily::IPv6,
        }
    }
}

/// Helper function for converting IPv4 mapped IPv6 address
///
/// This is the same as `Ipv6Addr::to_ipv4_mapped`, but it is still unstable in the current libstd
#[allow(unused)]
pub fn to_ipv4_mapped(ipv6: &Ipv6Addr) -> Option<Ipv4Addr> {
    match ipv6.octets() {
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => Some(Ipv4Addr::new(a, b, c, d)),
        _ => None,
    }
}

/// Consumes all data from `reader` and throws away until EOF
pub async fn ignore_until_end<R>(reader: &mut R) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0u8; 2048];

    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
    }

    Ok(())
}
