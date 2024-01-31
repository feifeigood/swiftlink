use std::io;

use swiftlink_transport::socks5::Address;

use crate::{
    context::{Metadata, Network},
    outbound::{
        AnyOutboundDatagram, AnyOutboundStream, OutboundDatagramHandle, OutboundStreamHandle, OutboundTransport,
    },
};

pub struct Handle;

#[async_trait::async_trait]
impl OutboundStreamHandle for Handle {
    fn remote_server_addr(&self, metadata: &Metadata) -> Option<(Network, Address)> {
        Some((Network::TCP, metadata.target.clone()))
    }

    async fn handle(&self, _metadata: &Metadata, stream: Option<AnyOutboundStream>) -> io::Result<AnyOutboundStream> {
        stream.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no stream"))
    }
}

#[async_trait::async_trait]
impl OutboundDatagramHandle for Handle {
    fn remote_server_addr(&self, metadata: &Metadata) -> Option<(Network, Address)> {
        Some((Network::UDP, metadata.target.clone()))
    }

    async fn handle(
        &self,
        _metadata: &Metadata,
        transport: Option<OutboundTransport<AnyOutboundStream, AnyOutboundDatagram>>,
    ) -> io::Result<AnyOutboundDatagram> {
        if let Some(OutboundTransport::Datagram(datagram)) = transport {
            Ok(datagram)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "no datagram"))
        }
    }
}
