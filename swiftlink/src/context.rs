use std::{
    fmt,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};

// use swiftlink_dns::DnsResolver;
use swiftlink_infra::fakedns::FakeDns;
use swiftlink_transport::socks5::Address;
use tokio::net::TcpStream;
use uuid::Uuid;

#[derive(Clone)]
pub struct Context {
    // Connect IPv6 address first
    ipv6_first: bool,
}

/// `Context` for sharing between services
pub type SharedContext = Arc<Context>;

impl Context {
    pub fn new() -> Self {
        Self { ipv6_first: false }
    }

    pub fn new_shared() -> SharedContext {
        SharedContext::new(Context::new())
    }

    pub fn ipv6_first(&self) -> bool {
        self.ipv6_first
    }

    pub fn set_ipv6_first(&mut self, ipv6_first: bool) {
        self.ipv6_first = ipv6_first;
    }
}

#[derive(Clone)]
pub struct ServiceContext {
    context: SharedContext,
    fakedns: Option<Arc<Mutex<FakeDns>>>,
}

impl ServiceContext {
    pub fn new() -> Self {
        Self {
            context: Context::new_shared(),
            fakedns: None,
        }
    }

    pub fn set_fakedns(&mut self, fakedns: Arc<Mutex<FakeDns>>) {
        self.fakedns = Some(fakedns);
    }

    pub fn fakedns(&self) -> Option<Arc<Mutex<FakeDns>>> {
        self.fakedns.clone()
    }
}

impl Default for ServiceContext {
    fn default() -> Self {
        ServiceContext::new()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum Network {
    TCP,
    UDP,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::TCP => write!(f, "TCP"),
            Self::UDP => write!(f, "UDP"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub inbound_tag: String,
    pub source: Address,
    pub target: Address,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            inbound_tag: "".to_string(),
            source: Address::SocketAddress(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
            target: Address::SocketAddress(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
        }
    }
}

pub struct InboundConnection {
    id: Uuid,
    stream: TcpStream,
    metadata: Metadata,
}

impl InboundConnection {
    pub fn with_context(stream: TcpStream, metadata: Metadata) -> Self {
        Self {
            id: Uuid::new_v4(),
            stream,
            metadata,
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
}
