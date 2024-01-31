use std::io;

use bytes::{BufMut, BytesMut};
use futures_util::TryFutureExt;
use sha2::{Digest, Sha224};
use swiftlink_transport::socks5::Address;
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

use swiftlink_infra::log::trace;

use crate::{
    context::{Metadata, Network},
    outbound::{
        AnyOutboundDatagram, AnyOutboundStream, OutboundDatagram, OutboundDatagramHandle, OutboundDatagramRecvHalf,
        OutboundDatagramSendHalf, OutboundStreamHandle, OutboundTransport,
    },
};

use self::consts::{CMD_TCP_CONNECT, CMD_UDP_ASSOCIATE, CRLF};

#[rustfmt::skip]
mod consts {
    pub const CMD_TCP_CONNECT:   u8  = 0x01;
    pub const CMD_UDP_ASSOCIATE: u8  = 0x03;
    pub const CMD_MUX:           u8  = 0x7f;
    pub const CRLF:              u16 = 0x0D0A;
}

const KEY_LEN: usize = 56;

pub struct Handle {
    pub addr: String,
    pub port: u16,
    pub password: String,
}

#[async_trait::async_trait]
impl OutboundStreamHandle for Handle {
    fn remote_server_addr(&self, _metadata: &Metadata) -> Option<(Network, Address)> {
        Some((Network::TCP, Address::from((self.addr.clone(), self.port))))
    }

    async fn handle(&self, metadata: &Metadata, stream: Option<AnyOutboundStream>) -> io::Result<AnyOutboundStream> {
        let mut stream = stream.ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no stream"))?;

        let header_len = KEY_LEN + 2 + 1 + metadata.target.serialized_len() + 2;
        let mut buf = Vec::with_capacity(header_len);
        let password = Sha224::digest(self.password.as_bytes());
        let hex_passwd = hex::encode(&password[..]);

        let cursor = &mut buf;
        cursor.put_slice(hex_passwd.as_bytes());
        cursor.put_u16(CRLF);
        cursor.put_u8(CMD_TCP_CONNECT);
        metadata.target.write_to_buf(cursor);
        cursor.put_u16(CRLF);

        stream.write_all(&buf).await?;

        Ok(Box::new(stream))
    }
}

#[async_trait::async_trait]
impl OutboundDatagramHandle for Handle {
    fn remote_server_addr(&self, _metadata: &Metadata) -> Option<(Network, Address)> {
        Some((Network::TCP, Address::from((self.addr.clone(), self.port))))
    }

    async fn handle(
        &self,
        metadata: &Metadata,
        transport: Option<OutboundTransport<AnyOutboundStream, AnyOutboundDatagram>>,
    ) -> io::Result<AnyOutboundDatagram> {
        let stream = if let Some(OutboundTransport::Stream(stream)) = transport {
            stream
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "no stream"));
        };

        let header_len = KEY_LEN + 2 + 1 + metadata.target.serialized_len() + 2;
        let mut buf = Vec::with_capacity(header_len);
        let password = Sha224::digest(self.password.as_bytes());
        let hex_passwd = hex::encode(&password[..]);

        let cursor = &mut buf;
        cursor.put_slice(hex_passwd.as_bytes());
        cursor.put_u16(CRLF);
        cursor.put_u8(CMD_UDP_ASSOCIATE);
        metadata.target.write_to_buf(cursor);
        cursor.put_u16(CRLF);

        Ok(Box::new(Datagram {
            stream,
            target: Some(metadata.target.clone()),
            header: Some(buf),
        }))
    }
}

pub struct Datagram<S> {
    stream: S,
    target: Option<Address>,
    header: Option<Vec<u8>>,
}

impl<S> OutboundDatagram for Datagram<S>
where
    S: 'static + AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    fn split(self: Box<Self>) -> (Box<dyn OutboundDatagramRecvHalf>, Box<dyn OutboundDatagramSendHalf>) {
        let (r, w) = split(self.stream);
        (
            Box::new(DatagramRecvHalf(r, self.target)),
            Box::new(DatagramSendHalf(w, self.header)),
        )
    }
}

pub struct DatagramRecvHalf<T>(ReadHalf<T>, Option<Address>);

#[async_trait::async_trait]
impl<T> OutboundDatagramRecvHalf for DatagramRecvHalf<T>
where
    T: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, Address)> {
        let addr = Address::read_from(&mut self.0).await?;

        let mut len_buf = [0u8; 2];
        self.0.read_exact(&mut len_buf).await?;
        let len = ((len_buf[0] as usize) << 8) | (len_buf[1] as usize);
        if buf.len() < len {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "buffer too small"));
        }
        self.0.read_exact(&mut buf[..len]).await?;

        // If the initial destination is of domain type, we return that
        // domain address instead of the real source address. That also
        // means we assume all received packets are comming from a same
        // address.
        if self.1.is_some() {
            trace!("received UDP {} bytes from {}", len, self.1.as_ref().unwrap());
            Ok((len, self.1.as_ref().unwrap().clone()))
        } else {
            trace!("received UDP {} bytes from {}", len, &addr);
            Ok((len, addr))
        }
    }
}

pub struct DatagramSendHalf<T>(WriteHalf<T>, Option<Vec<u8>>);

#[async_trait::async_trait]
impl<T> OutboundDatagramSendHalf for DatagramSendHalf<T>
where
    T: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn send_to(&mut self, buf: &[u8], addr: &Address) -> io::Result<usize> {
        trace!("send UDP {} bytes to {}", buf.len(), addr);
        let mut data = BytesMut::new();
        addr.write_to_buf(&mut data);
        data.put_u16(buf.len() as u16);
        data.put_u16(CRLF);
        data.put_slice(buf);

        // Writes the header along with the first payload.
        if self.1.is_some() {
            if let Some(mut header) = self.1.take() {
                header.extend_from_slice(&data);
                return self.0.write_all(&header).map_ok(|_| buf.len()).await;
            }
        }

        self.0.write_all(&data).map_ok(|_| buf.len()).await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}
