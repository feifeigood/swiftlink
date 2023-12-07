use std::{
    borrow::Borrow,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use swiftlink_infra::fakedns;

use crate::{
    dns_handle::{DnsRequestHandle, DnsRequestHandleNext},
    libdns::{
        proto::{
            op::ResponseCode,
            rr::{
                rdata::{a, aaaa},
                RData, Record, RecordType,
            },
        },
        resolver::{lookup::Lookup, Name},
    },
    DnsContext, DnsError, DnsRequest, DnsResponse,
};

#[derive(Debug)]
pub struct FakeDnsHandle {
    fakedns: Arc<Mutex<fakedns::FakeDns>>,
}

impl FakeDnsHandle {
    pub fn new(fakedns: Arc<Mutex<fakedns::FakeDns>>) -> Self {
        Self { fakedns }
    }
}

#[async_trait::async_trait]
impl DnsRequestHandle for FakeDnsHandle {
    async fn handle(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
        next: DnsRequestHandleNext<'_>,
    ) -> Result<DnsResponse, DnsError> {
        let name: &Name = req.query().name().borrow();
        let rtype = req.query().query_type();

        match rtype {
            RecordType::A | RecordType::AAAA => {
                let ipv6 = matches!(rtype, RecordType::AAAA);
                let host = name.to_ascii().trim_end_matches('.').to_owned();
                let fakeip = self.fakedns.lock().unwrap().lookup_ip(&host, ipv6);
                if let Some(ip) = fakeip {
                    let query = req.query().original().clone();
                    let name = query.name().to_owned();
                    let valid_until = Instant::now() + Duration::from_secs(1);

                    let record = match ip {
                        IpAddr::V4(ipv4) => Record::from_rdata(name, 1, RData::A(a::A::from(ipv4))),
                        IpAddr::V6(ipv6) => {
                            Record::from_rdata(name, 1, RData::AAAA(aaaa::AAAA::from(ipv6)))
                        }
                    };

                    return Ok(Lookup::new_with_deadline(
                        query,
                        vec![record].into(),
                        valid_until,
                    ));
                }
            }
            RecordType::SVCB | RecordType::HTTPS => {
                return Err(DnsError::ResponseCode(ResponseCode::NXDomain).into())
            }
            _ => {}
        }

        next.run(ctx, req).await
    }
}
