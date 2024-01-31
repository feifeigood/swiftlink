use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use bytes::{BufMut, BytesMut};
use tokio::{
    net::{self, UdpSocket},
    time,
};
use tokio_util::sync::CancellationToken;

use swiftlink_infra::net::{to_ipv4_mapped, MAXIMUM_UDP_PAYLOAD_SIZE};
use swiftlink_transport::socks5::{Address, UdpAssociateHeader};

use crate::{
    context::ServiceContext,
    inbound::association::{UdpAssociationManager, UdpInboundWrite},
};

#[derive(Clone)]
struct SocksUdpInboundWriter {
    socket: Arc<UdpSocket>,
}

#[async_trait::async_trait]
impl UdpInboundWrite for SocksUdpInboundWriter {
    async fn send_to(&self, src_addr: SocketAddr, dst_addr: &Address, data: &[u8]) -> io::Result<()> {
        let dst_addr = match dst_addr {
            Address::SocketAddress(sa) => {
                // Try to convert IPv4 mapped IPv6 address if server is running on dual-stack mode
                let saddr = match *sa {
                    SocketAddr::V4(..) => *sa,
                    SocketAddr::V6(ref v6) => match to_ipv4_mapped(v6.ip()) {
                        Some(v4) => SocketAddr::new(IpAddr::from(v4), v6.port()),
                        None => *sa,
                    },
                };

                Address::SocketAddress(saddr)
            }
            daddr => daddr.clone(),
        };

        // Reassemble packet
        let mut payload_buffer = BytesMut::new();
        let header = UdpAssociateHeader::new(0, dst_addr.clone());
        payload_buffer.reserve(header.serialized_len() + data.len());

        header.write_to_buf(&mut payload_buffer);
        payload_buffer.put_slice(data);

        self.socket.send_to(&payload_buffer, src_addr).await.map(|_| ())
    }
}

pub async fn serve_udp(
    context: Arc<ServiceContext>,
    socket: net::UdpSocket,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let socket = Arc::new(socket);

    let (mut manager, cleanup_interval, mut keepalive_rx) = UdpAssociationManager::new(
        context,
        SocksUdpInboundWriter { socket: socket.clone() },
        Some(Duration::from_secs(60)),
        None,
    );

    let mut buffer = [0u8; MAXIMUM_UDP_PAYLOAD_SIZE];
    let mut cleanup_timer = time::interval(cleanup_interval);

    loop {
        tokio::select! {
            _ = cleanup_timer.tick() => {
                // cleanup expired associations. iter() will remove expired elements
                manager.cleanup_expired().await;
            },

            _ = shutdown.cancelled() => {
                // A graceful shutdown is initiated. Break out of the loop.
                break;
            }
        }
    }

    Ok(())
}
