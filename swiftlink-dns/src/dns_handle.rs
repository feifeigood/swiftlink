use std::{str::FromStr, sync::Arc};

use futures_util::{future::BoxFuture, FutureExt};
use hickory_proto::{
    op::ResponseCode,
    rr::{rdata::SOA, Record},
};
use hickory_resolver::{error::ResolveErrorKind, Name};
use swiftlink_infra::ServerOpts;

use crate::{
    dns::{DnsContext, DnsError, DnsRequest, DnsResponse, MAX_TTL},
    Config,
};

#[async_trait::async_trait]
pub trait DnsRequestHandle: 'static + Send + Sync {
    async fn handle(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
        next: NextDnsRequestHandle<'_>,
    ) -> Result<DnsResponse, DnsError>;
}

#[derive(Clone)]
pub struct NextDnsRequestHandle<'a> {
    handles: &'a [Arc<dyn DnsRequestHandle>],
}

impl<'a> NextDnsRequestHandle<'a> {
    pub(crate) fn new(handles: &'a [Arc<dyn DnsRequestHandle>]) -> Self {
        Self { handles }
    }

    #[inline]
    pub fn run(
        mut self,
        ctx: &'a mut DnsContext,
        req: &'a DnsRequest,
    ) -> BoxFuture<'a, Result<DnsResponse, DnsError>> {
        if let Some((current, rest)) = self.handles.split_first() {
            self.handles = rest;
            current.handle(ctx, req, self).boxed()
        } else {
            async move {
                let soa = Record::from_rdata(
                    req.query().name().to_owned().into(),
                    MAX_TTL,
                    SOA::new(
                        Name::from_str("a.gtld-servers.net").unwrap(),
                        Name::from_str("nstld.verisign-grs.com").unwrap(),
                        1800,
                        1800,
                        900,
                        604800,
                        86400,
                    ),
                );
                Err(ResolveErrorKind::NoRecordsFound {
                    query: req.query().original().to_owned().into(),
                    soa: Some(Box::new(soa)),
                    negative_ttl: None,
                    response_code: ResponseCode::ServFail,
                    trusted: true,
                }
                .into())
            }
            .boxed()
        }
    }
}

pub struct DnsHandlerBuilder {
    handle_stack: Vec<Arc<dyn DnsRequestHandle>>,
}

impl DnsHandlerBuilder {
    pub fn new() -> Self {
        Self {
            handle_stack: Default::default(),
        }
    }

    #[inline]
    pub fn with<H>(self, handle: H) -> Self
    where
        H: DnsRequestHandle + 'static,
    {
        self.with_arc(Arc::new(handle))
    }

    #[inline]
    pub fn with_arc(mut self, handle: Arc<dyn DnsRequestHandle>) -> Self {
        self.handle_stack.push(handle);
        self
    }

    #[inline]
    pub fn build(self, cfg: Arc<Config>) -> DnsHandler {
        DnsHandler {
            cfg,
            handle_stack: self.handle_stack.into_boxed_slice(),
        }
    }
}

pub struct DnsHandler {
    cfg: Arc<Config>,
    handle_stack: Box<[Arc<dyn DnsRequestHandle>]>,
}

impl DnsHandler {
    pub async fn search(
        &self,
        req: &DnsRequest,
        server_opts: &ServerOpts,
    ) -> Result<DnsResponse, DnsError> {
        let cfg = self.cfg.clone();
        let mut ctx = DnsContext::new(cfg, server_opts.clone());

        self.execute(&mut ctx, req).await
    }

    async fn execute(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
    ) -> Result<DnsResponse, DnsError> {
        let next = NextDnsRequestHandle::new(&self.handle_stack);
        next.run(ctx, req).await
    }
}
