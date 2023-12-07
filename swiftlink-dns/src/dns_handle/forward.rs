use std::{borrow::Borrow, sync::Arc};

use swiftlink_infra::log::debug;

use crate::{
    client::DnsClient,
    dns_handle::{DnsRequestHandle, DnsRequestHandleNext},
    libdns::resolver::Name,
    resolver::{GenericResolver, LookupOptions},
    DnsContext, DnsError, DnsRequest, DnsResponse,
};

#[derive(Debug)]
pub struct ForwardHandle {
    client: Arc<DnsClient>,
}

impl ForwardHandle {
    pub fn new(client: Arc<DnsClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl DnsRequestHandle for ForwardHandle {
    async fn handle(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
        _next: DnsRequestHandleNext<'_>,
    ) -> Result<DnsResponse, DnsError> {
        let name: &Name = req.query().name().borrow();
        let rtype = req.query().query_type();

        let client = &self.client;

        // if dns request query nameserver, lookup local cache first
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

        // forward dns request
        client.lookup(name.clone(), lookup_options).await
    }
}
