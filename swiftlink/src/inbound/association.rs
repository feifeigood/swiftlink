use std::{io, marker::PhantomData, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use lru_time_cache::LruCache;
use tokio::{sync::mpsc, task::JoinHandle, time};

use swiftlink_infra::log::*;
use swiftlink_transport::socks5::Address;

use crate::context::ServiceContext;

/// Packet size for all UDP associations' send queue
pub const UDP_ASSOCIATION_SEND_CHANNEL_SIZE: usize = 1024;

/// Keep-alive channel size for UDP associations' manager
pub const UDP_ASSOCIATION_KEEP_ALIVE_CHANNEL_SIZE: usize = 64;

/// Default UDP association's expire duration
pub const DEFAULT_UDP_EXPIRY_DURATION: Duration = Duration::from_secs(5 * 60);

/// Writer for sending packets back to client
///
/// Currently it requires `async-trait` for `async fn` in trait, which will allocate a `Box`ed `Future` every call of `send_to`.
/// This performance issue could be solved when `generic_associated_types` and `generic_associated_types` are stabilized.
#[async_trait::async_trait]
pub trait UdpInboundWrite {
    async fn send_to(&self, src_addr: SocketAddr, dst_addr: &Address, data: &[u8]) -> io::Result<()>;
}

type AssociationMap<W> = LruCache<SocketAddr, UdpAssociation<W>>;

/// UDP association manager
pub struct UdpAssociationManager<W>
where
    W: UdpInboundWrite + Clone + Send + Sync + Unpin + 'static,
{
    respond_writer: W,
    context: Arc<ServiceContext>,
    assoc_map: AssociationMap<W>,
    keepalive_tx: mpsc::Sender<SocketAddr>,
}

impl<W> UdpAssociationManager<W>
where
    W: UdpInboundWrite + Clone + Send + Sync + Unpin + 'static,
{
    pub fn new(
        context: Arc<ServiceContext>,
        respond_writer: W,
        time_to_live: Option<Duration>,
        capacity: Option<usize>,
    ) -> (UdpAssociationManager<W>, Duration, mpsc::Receiver<SocketAddr>) {
        let time_to_live = time_to_live.unwrap_or(DEFAULT_UDP_EXPIRY_DURATION);
        let assoc_map = match capacity {
            Some(capacity) => LruCache::with_expiry_duration_and_capacity(time_to_live, capacity),
            None => LruCache::with_expiry_duration(time_to_live),
        };

        let (keepalive_tx, keepalive_rx) = mpsc::channel(UDP_ASSOCIATION_KEEP_ALIVE_CHANNEL_SIZE);

        (
            UdpAssociationManager {
                respond_writer,
                context,
                assoc_map,
                keepalive_tx,
            },
            time_to_live,
            keepalive_rx,
        )
    }

    /// Sends `data` from `src_addr` to `dst_addr`
    pub async fn send_to(&mut self, src_addr: SocketAddr, dst_addr: Address, data: &[u8]) -> io::Result<()> {
        // Check or (re)create an association

        if let Some(assoc) = self.assoc_map.get(&src_addr) {
            return assoc.try_send((dst_addr, Bytes::copy_from_slice(data)));
        }

        let assoc = UdpAssociation::new(
            self.context.clone(),
            src_addr,
            self.keepalive_tx.clone(),
            self.respond_writer.clone(),
        );

        debug!("created udp association for {}", src_addr);

        assoc.try_send((dst_addr, Bytes::copy_from_slice(data)))?;
        self.assoc_map.insert(src_addr, assoc);

        Ok(())
    }

    /// Cleanup expired associations
    pub async fn cleanup_expired(&mut self) {
        self.assoc_map.iter();
    }

    /// Keep-alive association
    pub async fn keep_alive(&mut self, src_addr: &SocketAddr) {
        self.assoc_map.get(src_addr);
    }
}

struct UdpAssociation<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    assoc_handle: JoinHandle<()>,
    sender: mpsc::Sender<(Address, Bytes)>,
    writer: PhantomData<W>,
}

impl<W> Drop for UdpAssociation<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    fn drop(&mut self) {
        self.assoc_handle.abort();
    }
}

impl<W> UdpAssociation<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    fn new(
        context: Arc<ServiceContext>,
        src_addr: SocketAddr,
        keepalive_tx: mpsc::Sender<SocketAddr>,
        respond_writer: W,
    ) -> UdpAssociation<W> {
        let (assoc_handle, sender) = UdpAssociationContext::create(context, src_addr, keepalive_tx, respond_writer);
        UdpAssociation {
            assoc_handle,
            sender,
            writer: PhantomData,
        }
    }

    fn try_send(&self, data: (Address, Bytes)) -> io::Result<()> {
        if self.sender.try_send(data).is_err() {
            let err = io::Error::new(io::ErrorKind::Other, "udp relay channel full");
            return Err(err);
        }
        Ok(())
    }
}

struct UdpAssociationContext<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    context: Arc<ServiceContext>,
    src_addr: SocketAddr,
    // proxied_socket: Option<MonProxySocket>,
    keepalive_tx: mpsc::Sender<SocketAddr>,
    keepalive_flag: bool,
    respond_writer: W,
}

impl<W> Drop for UdpAssociationContext<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    fn drop(&mut self) {
        debug!("udp association for {} is closed", self.src_addr);
    }
}

impl<W> UdpAssociationContext<W>
where
    W: UdpInboundWrite + Send + Sync + Unpin + 'static,
{
    fn create(
        context: Arc<ServiceContext>,
        src_addr: SocketAddr,
        keepalive_tx: mpsc::Sender<SocketAddr>,
        respond_writer: W,
    ) -> (JoinHandle<()>, mpsc::Sender<(Address, Bytes)>) {
        // Pending packets UDP_ASSOCIATION_SEND_CHANNEL_SIZE for each association should be good enough for a server.
        // If there are plenty of packets stuck in the channel, dropping excessive packets is a good way to protect the server from
        // being OOM.
        let (sender, receiver) = mpsc::channel(UDP_ASSOCIATION_SEND_CHANNEL_SIZE);

        let mut assoc = UdpAssociationContext {
            context,
            src_addr,
            keepalive_tx,
            keepalive_flag: false,
            respond_writer,
        };
        let handle = tokio::spawn(async move { assoc.dispatch_packet(receiver).await });

        (handle, sender)
    }

    async fn dispatch_packet(&mut self, mut receiver: mpsc::Receiver<(Address, Bytes)>) {
        let mut keepalive_interval = time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                _ = keepalive_interval.tick() => {
                    if self.keepalive_flag {
                        if self.keepalive_tx.try_send(self.src_addr).is_err() {
                            debug!("udp relay {} keep-alive failed, channel full or closed", self.src_addr);
                        } else {
                            self.keepalive_flag = false;
                        }
                    }
                }
            }
        }
    }

    async fn dispatch_received_packet(&mut self, dst_addr: &Address, data: &[u8]) {
        trace!("udp relay {} -> {} with {} bytes", self.src_addr, dst_addr, data.len());

        // TODO: send received packet to proxied socket
    }
}
