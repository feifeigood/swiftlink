use std::{net::SocketAddr, sync::Arc};

pub use config::DnsConfig;
pub use libdns::server::ServerFuture;
pub use resolver::{build_dns_resolver, DnsResolver};
pub use server::{ServerHandle, ServerHandleBuilder};

use crate::libdns::{
    proto::{
        op::{LowerQuery, Query},
        rr::{Name, RecordType},
    },
    server::server::{Protocol, Request},
};

mod client;
mod config;
mod dns_handle;
mod dns_url;
mod error;
mod libdns;
mod preset_ns;
mod proxy;
mod resolver;
mod rustls;
mod server;

/// Maximum TTL as defined in https://tools.ietf.org/html/rfc2181, 2147483647
///   Setting this to a value of 1 day, in seconds
pub(crate) const MAX_TTL: u32 = 86400_u32;

pub type DnsError = error::LookupError;
pub type DnsResponse = libdns::resolver::lookup::Lookup;

#[derive(Debug, Clone)]
pub struct DnsRequest {
    id: u16,
    /// Message with the associated query or update data
    query: LowerQuery,
    /// Source address of the Client
    src: SocketAddr,
    /// Protocol of the request
    protocol: Protocol,
}

impl From<&Request> for DnsRequest {
    fn from(req: &Request) -> Self {
        Self {
            id: req.id(),
            query: req.query().to_owned(),
            src: req.src(),
            protocol: req.protocol(),
        }
    }
}

impl DnsRequest {
    /// see `Header::id()`
    pub fn id(&self) -> u16 {
        self.id
    }

    /// ```text
    /// Question        Carries the query name and other query parameters.
    /// ```
    #[inline]
    pub fn query(&self) -> &LowerQuery {
        &self.query
    }

    /// The IP address from which the request originated.
    #[inline]
    pub fn src(&self) -> SocketAddr {
        self.src
    }

    /// The protocol that was used for the request
    #[inline]
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    pub fn with_cname(&self, name: Name) -> Self {
        Self {
            id: self.id,
            query: LowerQuery::from(Query::query(name, self.query().query_type())),
            src: self.src,
            protocol: self.protocol,
        }
    }

    pub fn set_query_type(&mut self, query_type: RecordType) {
        let mut query = self.query.original().clone();
        query.set_query_type(query_type);
        self.query = LowerQuery::from(query)
    }
}

impl From<Query> for DnsRequest {
    fn from(query: Query) -> Self {
        use std::net::{Ipv4Addr, SocketAddrV4};

        Self {
            id: rand::random(),
            query: query.into(),
            src: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 53)),
            protocol: Protocol::Udp,
        }
    }
}

pub struct DnsContext {
    cfg: Arc<DnsConfig>,
    pub no_cache: bool,
    pub background: bool,
}

impl DnsContext {
    pub fn new(cfg: Arc<DnsConfig>) -> Self {
        DnsContext {
            cfg,
            no_cache: false,
            background: false,
        }
    }

    #[inline]
    pub fn cfg(&self) -> &Arc<DnsConfig> {
        &self.cfg
    }
}
