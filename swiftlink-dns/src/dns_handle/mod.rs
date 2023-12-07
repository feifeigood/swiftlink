use futures_util::{future::BoxFuture, FutureExt};
use std::{str::FromStr, sync::Arc};

use crate::{
    libdns::{
        proto::{
            op::ResponseCode,
            rr::{rdata::SOA, Record},
        },
        resolver::{error::ResolveErrorKind, Name},
    },
    DnsConfig, DnsContext, DnsError, DnsRequest, DnsResponse, MAX_TTL,
};

pub use fakedns::FakeDnsHandle;
pub use forward::ForwardHandle;

mod fakedns;
mod forward;

#[async_trait::async_trait]
pub trait DnsRequestHandle: 'static + Send + Sync {
    async fn handle(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
        next: DnsRequestHandleNext<'_>,
    ) -> Result<DnsResponse, DnsError>;
}

#[derive(Clone)]
pub struct DnsRequestHandleNext<'a> {
    handles: &'a [Arc<dyn DnsRequestHandle>],
}

impl<'a> DnsRequestHandleNext<'a> {
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

pub struct DnsRequestHandlerBuilder {
    handle_stack: Vec<Arc<dyn DnsRequestHandle>>,
}

impl DnsRequestHandlerBuilder {
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
    pub fn build(self, cfg: Arc<DnsConfig>) -> DnsRequestHandler {
        DnsRequestHandler {
            cfg,
            handle_stack: self.handle_stack.into_boxed_slice(),
        }
    }
}

pub struct DnsRequestHandler {
    cfg: Arc<DnsConfig>,
    handle_stack: Box<[Arc<dyn DnsRequestHandle>]>,
}

impl DnsRequestHandler {
    pub async fn search(&self, req: &DnsRequest) -> Result<DnsResponse, DnsError> {
        let cfg = self.cfg.clone();
        let mut ctx = DnsContext::new(cfg);

        self.execute(&mut ctx, req).await
    }

    async fn execute(
        &self,
        ctx: &mut DnsContext,
        req: &DnsRequest,
    ) -> Result<DnsResponse, DnsError> {
        DnsRequestHandleNext::new(&self.handle_stack)
            .run(ctx, req)
            .await
    }
}
