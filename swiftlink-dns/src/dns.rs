use std::sync::Arc;

use hickory_resolver::lookup::Lookup;
use swiftlink_infra::ServerOpts;

use crate::{dns_conf::Config, dns_error::LookupError};

/// Maximum TTL as defined in https://tools.ietf.org/html/rfc2181, 2147483647
///   Setting this to a value of 1 day, in seconds
pub const MAX_TTL: u32 = 86400_u32;

pub struct DnsContext {
    cfg: Arc<Config>,
    pub server_opts: ServerOpts,
}

impl DnsContext {
    pub fn new(cfg: Arc<Config>, server_opts: ServerOpts) -> Self {
        DnsContext { cfg, server_opts }
    }

    #[inline]
    pub fn cfg(&self) -> &Arc<Config> {
        &self.cfg
    }

    #[inline]
    pub fn server_opts(&self) -> &ServerOpts {
        &self.server_opts
    }
}

mod request {
    use std::net::SocketAddr;

    use hickory_proto::{
        op::{LowerQuery, Query},
        rr::{Name, RecordType},
    };
    use hickory_server::server::{Protocol, Request as OriginRequest};

    #[derive(Clone)]
    pub struct Request {
        id: u16,
        /// Message with the associated query or update data
        query: LowerQuery,
        /// Source address of the Client
        src: SocketAddr,
        /// Protocol of the request
        protocol: Protocol,
    }

    impl From<&OriginRequest> for Request {
        fn from(req: &OriginRequest) -> Self {
            Self {
                id: req.id(),
                query: req.query().to_owned(),
                src: req.src(),
                protocol: req.protocol(),
            }
        }
    }

    impl Request {
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

    impl From<Query> for Request {
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
}

pub type DnsRequest = request::Request;
pub type DnsResponse = Lookup;
pub type DnsError = LookupError;
