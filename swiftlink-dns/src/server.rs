use cfg_if::cfg_if;
use futures_util::Future;
use std::{
    io,
    sync::{Arc, Mutex},
};

use swiftlink_infra::{fakedns, log::*};

use crate::{
    client::DnsClient,
    dns_handle::*,
    error::LookupError,
    libdns::{
        proto::{
            op::{Edns, Header, MessageType, OpCode, ResponseCode},
            rr::Record,
        },
        server::{
            authority::{
                AuthLookup, EmptyLookup, LookupObject, LookupOptions, MessageResponse,
                MessageResponseBuilder, ZoneType,
            },
            server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
            store::forwarder::ForwardLookup,
        },
    },
    DnsConfig, DnsRequest,
};

pub struct ServerHandleBuilder {
    config: Arc<DnsConfig>,
    client: Arc<DnsClient>,
    fakedns: Option<Arc<Mutex<fakedns::FakeDns>>>,
}

impl ServerHandleBuilder {
    pub fn new(config: Arc<DnsConfig>, client: Arc<DnsClient>) -> ServerHandleBuilder {
        ServerHandleBuilder {
            config,
            client,
            fakedns: None,
        }
    }

    pub fn with_fakedns(mut self, fakedns: Arc<Mutex<fakedns::FakeDns>>) -> Self {
        self.fakedns = Some(fakedns);
        self
    }

    pub fn build(self) -> ServerHandle {
        let mut builder = DnsRequestHandlerBuilder::new();

        if let Some(fakedns) = self.fakedns {
            builder = builder.with(FakeDnsHandle::new(fakedns));
        }

        let handler = Arc::new(
            builder
                .with(ForwardHandle::new(self.client))
                .build(self.config),
        );

        ServerHandle { handler }
    }
}

pub struct ServerHandle {
    handler: Arc<DnsRequestHandler>,
}

impl ServerHandle {
    pub fn new(handler: Arc<DnsRequestHandler>) -> Self {
        Self { handler }
    }
}

#[async_trait::async_trait]
impl RequestHandler for ServerHandle {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let result = match request.message_type() {
            MessageType::Query => match request.op_code() {
                OpCode::Query => {
                    let response_edns: Option<Edns>;

                    // check if it's edns

                    if let Some(req_edns) = request.edns() {
                        let mut response = MessageResponseBuilder::from_message_request(request);

                        let mut response_header = Header::response_from_request(request.header());

                        let mut resp_edns: Edns = Edns::new();

                        // check our version against the request
                        // TODO: what version are we?
                        let our_version = 0;
                        resp_edns.set_dnssec_ok(true);
                        resp_edns.set_max_payload(req_edns.max_payload().max(512));
                        resp_edns.set_version(our_version);
                        if req_edns.version() > our_version {
                            warn!(
                                "request edns version greater than {}: {}",
                                our_version,
                                req_edns.version()
                            );
                            response_header.set_response_code(ResponseCode::BADVERS);
                            resp_edns.set_rcode_high(ResponseCode::BADVERS.high());
                            response.edns(resp_edns);

                            // TODO: should ResponseHandle consume self?
                            let result = response_handle
                                .send_response(response.build_no_records(response_header))
                                .await;

                            // couldn't handle the request
                            return match result {
                                Err(e) => {
                                    error!("request error: {}", e);
                                    ResponseInfo::serve_failed()
                                }
                                Ok(info) => info,
                            };
                        }

                        response_edns = Some(resp_edns);
                    } else {
                        response_edns = None;
                    }

                    debug!(
                        "query received: {} {} {} client: {}",
                        request.id(),
                        request.query(),
                        request.query().query_type(),
                        request.src()
                    );

                    let info = async {
                        let request_id = request.id();

                        let request_header = request.header();

                        let (response_header, sections) = async {
                            let lookup_options = lookup_options_for_edns(request.edns());

                            // TODO: need remove this log?
                            // log algorithms being requested
                            if lookup_options.is_dnssec() {
                                info!(
                                    "request: {} lookup_options: {:?}",
                                    request_id, lookup_options
                                );
                            }

                            let mut response_header = Header::response_from_request(request_header);
                            response_header.set_authoritative(ZoneType::Forward.is_authoritative());

                            // debug!("performing {} on {}", query, authority.origin());

                            // let future = self.dns_server.search(request_info, lookup_options);

                            let future = async {
                                let req: &DnsRequest = &request.into();

                                let lookup_result: Result<Box<dyn LookupObject>, LookupError> =
                                    match self.handler.search(req).await {
                                        Ok(lookup) => Ok(Box::new(ForwardLookup(lookup))),
                                        Err(err) => Err(err),
                                    };

                                lookup_result
                            };

                            let sections = send_forwarded_response(
                                future,
                                request_header,
                                &mut response_header,
                            )
                            .await;

                            (response_header, sections)
                        }
                        .await;

                        let response = MessageResponseBuilder::from_message_request(request).build(
                            response_header,
                            sections.answers.iter(),
                            sections.ns.iter(),
                            sections.soa.iter(),
                            sections.additionals.iter(),
                        );

                        let result =
                            send_response(response_edns.clone(), response, response_handle.clone())
                                .await;

                        match result {
                            Err(e) => {
                                error!("error sending response: {}", e);
                                ResponseInfo::serve_failed()
                            }
                            Ok(i) => i,
                        }
                    }
                    .await;

                    Ok(info)
                }
                c => {
                    warn!("unimplemented op_code: {:?}", c);
                    let response = MessageResponseBuilder::from_message_request(request);

                    response_handle
                        .send_response(response.error_msg(request.header(), ResponseCode::NotImp))
                        .await
                }
            },
            MessageType::Response => {
                warn!("got a response as a request from id: {}", request.id());
                let response = MessageResponseBuilder::from_message_request(request);

                response_handle
                    .send_response(response.error_msg(request.header(), ResponseCode::FormErr))
                    .await
            }
        };

        match result {
            Err(e) => {
                error!("request failed: {}", e);
                ResponseInfo::serve_failed()
            }
            Ok(info) => info,
        }
    }
}

async fn send_forwarded_response(
    future: impl Future<Output = Result<Box<dyn LookupObject>, LookupError>>,
    request_header: &Header,
    response_header: &mut Header,
) -> LookupSections {
    response_header.set_recursion_available(true);
    response_header.set_authoritative(false);

    // Don't perform the recursive query if this is disabled...
    let answers = if !request_header.recursion_desired() {
        // cancel the future??
        // future.cancel();
        drop(future);

        info!(
            "request disabled recursion, returning no records: {}",
            request_header.id()
        );

        Box::new(EmptyLookup)
    } else {
        match future.await {
            Err(e) => {
                if e.is_nx_domain() {
                    response_header.set_response_code(ResponseCode::NXDomain);
                }

                let res: Box<dyn LookupObject> = match e.as_soa() {
                    Some(soa) => Box::new(ForwardLookup(soa)),
                    None => {
                        debug!("error resolving: {}", e);
                        Box::new(EmptyLookup)
                    }
                };

                res
            }
            Ok(rsp) => rsp,
        }
    };

    LookupSections {
        answers,
        ns: Box::<AuthLookup>::default(),
        soa: Box::<AuthLookup>::default(),
        additionals: Box::<AuthLookup>::default(),
    }
}

struct LookupSections {
    answers: Box<dyn LookupObject>,
    ns: Box<dyn LookupObject>,
    soa: Box<dyn LookupObject>,
    additionals: Box<dyn LookupObject>,
}

async fn send_response<'a, R: ResponseHandler>(
    _response_edns: Option<Edns>,
    response: MessageResponse<
        '_,
        'a,
        impl Iterator<Item = &'a Record> + Send + 'a,
        impl Iterator<Item = &'a Record> + Send + 'a,
        impl Iterator<Item = &'a Record> + Send + 'a,
        impl Iterator<Item = &'a Record> + Send + 'a,
    >,
    mut response_handle: R,
) -> io::Result<ResponseInfo> {
    #[cfg(feature = "dnssec")]
    if let Some(mut resp_edns) = response_edns {
        // set edns DAU and DHU
        // send along the algorithms which are supported by this authority
        let mut algorithms = SupportedAlgorithms::default();
        algorithms.set(Algorithm::RSASHA256);
        algorithms.set(Algorithm::ECDSAP256SHA256);
        algorithms.set(Algorithm::ECDSAP384SHA384);
        algorithms.set(Algorithm::ED25519);

        let dau = EdnsOption::DAU(algorithms);
        let dhu = EdnsOption::DHU(algorithms);

        resp_edns.options_mut().insert(dau);
        resp_edns.options_mut().insert(dhu);

        response.set_edns(resp_edns);
    }

    response_handle.send_response(response).await
}

fn lookup_options_for_edns(edns: Option<&Edns>) -> LookupOptions {
    let _edns = match edns {
        Some(edns) => edns,
        None => return LookupOptions::default(),
    };

    cfg_if! {
        if #[cfg(feature = "dnssec")] {
            let supported_algorithms = if let Some(&EdnsOption::DAU(algs)) = edns.option(EdnsCode::DAU)
            {
               algs
            } else {
               debug!("no DAU in request, used default SupportAlgorithms");
               SupportedAlgorithms::default()
            };

            LookupOptions::for_dnssec(edns.dnssec_ok(), supported_algorithms)
        } else {
            LookupOptions::default()
        }
    }
}

trait ServeFaild {
    fn serve_failed() -> Self;
}

impl ServeFaild for ResponseInfo {
    fn serve_failed() -> Self {
        let mut header = Header::new();
        header.set_response_code(ResponseCode::ServFail);
        header.into()
    }
}
