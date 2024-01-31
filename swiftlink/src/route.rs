use std::{io, sync::Arc};

use enum_dispatch::enum_dispatch;
use maxminddb::Mmap;
use swiftlink_transport::socks5::Address;

/// Return true If params contains `no-resolve`
fn has_no_resolve(params: &[String]) -> bool {
    params.iter().any(|x| x.eq("no-resolve"))
}

// #[enum_dispatch(RuleMatch)]
// pub enum Rule {
//     TypeDomain(Domain),
// }

// #[enum_dispatch]
// pub trait RuleMatch: Send + Sync + Unpin {
//     fn check_context_matched(&self, context: InboundContext) -> Option<String>;

//     fn should_resolve_ip(&self) -> bool {
//         false
//     }

//     fn should_find_process(&self) -> bool {
//         false
//     }
// }

// #[derive(Debug, Clone)]
// pub struct Domain {
//     target: String,
//     domain: String,
// }

// impl RuleMatch for Domain {
//     fn check_context_matched(&self, context: InboundContext) -> Option<String> {
//         match context.destination {
//             Address::DomainNameAddress(ref dn, _) => dn.eq(self.domain.as_str()).then_some(self.target.to_owned()),
//             _ => None,
//         }
//     }
// }

// #[derive(Debug, Clone)]
// struct DomainSuffix {
//     target: String,
//     suffix: String,
// }

// #[derive(Debug, Clone)]
// struct DomainKeyword {
//     target: String,
//     keyword: String,
// }

// struct GeoIP {
//     target: String,
//     reader: Arc<maxminddb::Reader<Mmap>>,
//     country: String,
//     no_resolve_ip: bool,
// }

// struct Match {}

// pub struct Router {
//     rules: Vec<Rule>,
// }

// impl Router {
//     pub fn new() -> Self {
//         Self { rules: Vec::new() }
//     }

//     pub async fn route_inbound(&self, context: InboundContext) -> io::Result<()> {
//         todo!()
//     }

//     async fn route_tcp_inner(&self, context: InboundContext) -> io::Result<()> {
//         todo!()
//     }
// }
