//! Network utilities for the swiftlink.

mod sys;
pub mod tcp;
mod timeout_stream;
pub mod udp;

mod options;
use std::net::SocketAddr;

pub use options::{ConnectOpts, TcpSocketOpts};

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
