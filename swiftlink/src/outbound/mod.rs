use std::io;
use tokio::io::{AsyncRead, AsyncWrite};

use swiftlink_transport::socks5::Address;

use crate::context::{Metadata, Network};

pub mod manager;

pub mod direct;
pub mod reject;
pub mod tls;
pub mod trojan;

/// The receive half of an unreliable transport
#[async_trait::async_trait]
pub trait OutboundDatagramRecvHalf: Send + Sync + Unpin {
    async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, Address)>;
}

/// The send half of an unreliable transport
#[async_trait::async_trait]
pub trait OutboundDatagramSendHalf: Send + Sync + Unpin {
    async fn send_to(&mut self, buf: &[u8], addr: &Address) -> io::Result<usize>;
    async fn close(&mut self) -> io::Result<()>;
}

/// An unreliable transport for outbound handle
pub trait OutboundDatagram: Send + Unpin {
    fn split(self: Box<Self>) -> (Box<dyn OutboundDatagramRecvHalf>, Box<dyn OutboundDatagramSendHalf>);
}

pub type AnyOutboundDatagram = Box<dyn OutboundDatagram>;

/// An outbound handle for UDP connection
#[async_trait::async_trait]
pub trait OutboundDatagramHandle<S = AnyOutboundStream, D = AnyOutboundDatagram>: Send + Sync + Unpin {
    fn remote_server_addr(&self, metadata: &Metadata) -> Option<(Network, Address)>;
    async fn handle(&self, metadata: &Metadata, transport: Option<OutboundTransport<S, D>>) -> io::Result<D>;
}

pub type AnyOutboundDatagramHandle = Box<dyn OutboundDatagramHandle>;

/// An reliable transport for outbound handle
pub trait OutboundStream: AsyncRead + AsyncWrite + Send + Sync + Unpin {}

impl<S> OutboundStream for S where S: AsyncRead + AsyncWrite + Send + Sync + Unpin {}

pub type AnyOutboundStream = Box<dyn OutboundStream>;

/// An outbound handle for TCP connection
#[async_trait::async_trait]
pub trait OutboundStreamHandle<S = AnyOutboundStream>: Send + Sync + Unpin {
    fn remote_server_addr(&self, metadata: &Metadata) -> Option<(Network, Address)>;
    async fn handle(&self, metadata: &Metadata, stream: Option<S>) -> io::Result<S>;
}

pub type AnyOutboundStreamHandle = Box<dyn OutboundStreamHandle>;

pub enum OutboundTransport<S, D> {
    Stream(S),
    Datagram(D),
}

pub type AnyOutboundTransport = OutboundTransport<AnyOutboundStream, AnyOutboundDatagram>;

/// An outbound handle for TCP and UDP connection
pub trait OutboundHandle: Send + Sync + Unpin {
    fn stream(&self) -> Option<&AnyOutboundStreamHandle>;
    fn datagram(&self) -> Option<&AnyOutboundDatagramHandle>;
}

pub type AnyOutboundHandle = Box<dyn OutboundHandle>;
