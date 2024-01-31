use std::sync::Arc;
use tokio::{
    net::{self},
    sync::mpsc::{self},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use swiftlink_infra::{auth::Authenticator, log::*};

use crate::context::{InboundConnection, ServiceContext};
use crate::inbound::ServerHandle;

use self::tcp::serve_tcp;

mod tcp;
mod udp;

/// SOCKS4/4a, SOCKS5 Local Server builder
pub struct SocksBuilder {
    context: Arc<ServiceContext>,
    authenticator: Arc<Option<Authenticator>>,
    tcp_in: mpsc::Sender<InboundConnection>,
}

impl SocksBuilder {
    pub fn new(context: Arc<ServiceContext>, tcp_in: mpsc::Sender<InboundConnection>) -> Self {
        Self {
            context,
            authenticator: Arc::new(None),
            tcp_in,
        }
    }

    pub fn set_authenticator(&mut self, authenticator: Arc<Option<Authenticator>>) {
        self.authenticator = authenticator;
    }

    pub fn build(self) -> Socks {
        Socks {
            context: self.context,
            authenticator: self.authenticator,
            tcp_in: self.tcp_in,
        }
    }
}

/// SOCKS4/4a, SOCKS5 Local Server
pub struct Socks {
    context: Arc<ServiceContext>,
    authenticator: Arc<Option<Authenticator>>,
    tcp_in: mpsc::Sender<InboundConnection>,
}

impl Socks {
    pub fn builder(context: Arc<ServiceContext>, tcp_in: mpsc::Sender<InboundConnection>) -> SocksBuilder {
        SocksBuilder::new(context, tcp_in)
    }

    pub fn serve(self, tcp_listener: net::TcpListener, udp_socket: net::UdpSocket) -> ServerHandle {
        let shutdown_token = CancellationToken::new();
        let mut join_set = JoinSet::new();

        {
            let shutdown = shutdown_token.clone();
            let authenticator = self.authenticator.clone();
            let tcp_in = self.tcp_in.clone();
            join_set.spawn(async move { serve_tcp(tcp_listener, shutdown, authenticator, tcp_in).await });
        }

        ServerHandle(join_set, shutdown_token)
    }
}
