use std::borrow::Borrow;

use hickory_resolver::Name;

use swiftlink_infra::log::debug;

use crate::{
    dns_client::{DnsClient, GenericResolver, LookupOptions},
    dns_handle::NextDnsRequestHandle,
    DnsContext, DnsError, DnsRequest, DnsRequestHandle, DnsResponse,
};

pub struct ForwardRequestHandle {
    client: DnsClient,
}

impl ForwardRequestHandle {
    pub fn new(client: DnsClient) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl DnsRequestHandle for ForwardRequestHandle {
    async fn handle(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
        _next: NextDnsRequestHandle<'_>,
    ) -> Result<DnsResponse, DnsError> {
        let name: &Name = req.query().name().borrow();
        let rtype = req.query().query_type();

        let client = &self.client;

        if let Some(lookup) = client.lookup_nameserver(name.clone(), rtype).await {
            debug!(
                "lookup nameserver {} {} ip {:?}",
                name,
                rtype,
                lookup
                    .records()
                    .iter()
                    .filter_map(|record| record.data().map(|data| data.ip_addr()))
                    .flatten()
                    .collect::<Vec<_>>()
            );
            ctx.no_cache = true;
            return Ok(lookup);
        }

        let lookup_options = LookupOptions {
            record_type: rtype,
            client_subnet: None,
        };

        client.lookup(name.clone(), lookup_options).await
    }
}

struct LookupIpOptions {}

#[cfg(test)]
mod tests {}
