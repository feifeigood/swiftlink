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
    fn remote_server_addr(&self, _metadata: &Metadata) -> Option<(Network, Address)> {
        None
    }

    async fn handle(&self, _metadata: &Metadata, _stream: Option<AnyOutboundStream>) -> io::Result<AnyOutboundStream> {
        Err(io::Error::new(io::ErrorKind::Other, "rejected"))
    }
}

#[async_trait::async_trait]
impl OutboundDatagramHandle for Handle {
    fn remote_server_addr(&self, _metadata: &Metadata) -> Option<(Network, Address)> {
        None
    }

    async fn handle(
        &self,
        _metadata: &Metadata,
        _transport: Option<OutboundTransport<AnyOutboundStream, AnyOutboundDatagram>>,
    ) -> io::Result<AnyOutboundDatagram> {
        Err(io::Error::new(io::ErrorKind::Other, "rejected"))
    }
}
