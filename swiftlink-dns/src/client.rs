use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    ops::Deref,
    path::PathBuf,
    slice::Iter,
    sync::Arc,
    time::{Duration, Instant},
};

use swiftlink_infra::{log::*, net::ConnectOpts};
use tokio::sync::RwLock;

use crate::{
    config::NameServerInfo,
    dns_url::{DnsUrl, DnsUrlParamExt},
    error::LookupError,
    libdns::{
        self,
        proto::{
            op::{Edns, Message, MessageType, OpCode, Query},
            rr::rdata::opt::{ClientSubnet, EdnsOption},
            rr::{Record, RecordType},
            xfer::{DnsHandle, DnsRequest, DnsRequestOptions, FirstAnswer},
        },
        resolver::{
            config::{NameServerConfig, Protocol, ResolverOpts, TlsClientConfig},
            lookup::Lookup,
            name_server::GenericConnector,
            IntoName, Name,
        },
    },
    proxy::ProxyConfig,
    resolver::{GenericResolver, GenericResolverExt, LookupOptions},
    rustls::TlsClientConfigBundle,
    MAX_TTL,
};

use bootstrap::BootstrapResolver;
use connection_provider::TokioCustomeRuntimeProvider;

#[derive(Default)]
pub struct DnsClientBuilder {
    resolver_opts: ResolverOpts,
    server_infos: Vec<NameServerInfo>,
    ca_file: Option<PathBuf>,
    ca_path: Option<PathBuf>,
    proxies: Arc<HashMap<String, ProxyConfig>>,
    client_subnet: Option<ClientSubnet>,
}

impl DnsClientBuilder {
    pub fn add_servers<S: Into<NameServerInfo>>(self, servers: Vec<S>) -> Self {
        servers.into_iter().fold(self, |b, s| b.add_server(s))
    }

    pub fn add_server<S: Into<NameServerInfo>>(mut self, server: S) -> Self {
        self.server_infos.push(server.into());
        self
    }

    pub fn with_ca_file(mut self, file: PathBuf) -> Self {
        self.ca_file = Some(file);
        self
    }

    pub fn with_ca_path(mut self, file: PathBuf) -> Self {
        self.ca_path = Some(file);
        self
    }

    pub fn with_proxies(mut self, proxies: Arc<HashMap<String, ProxyConfig>>) -> Self {
        self.proxies = proxies;

        self
    }

    pub fn with_client_subnet<S: Into<ClientSubnet>>(mut self, subnet: S) -> Self {
        self.client_subnet = Some(subnet.into());
        self
    }

    pub async fn build(self) -> DnsClient {
        let DnsClientBuilder {
            resolver_opts,
            server_infos,
            ca_file,
            ca_path,
            proxies,
            client_subnet,
        } = self;

        let factory = NameServerFactory::new(TlsClientConfigBundle::new(ca_path, ca_file));

        // initialize bootstrap resolver using pure ip dns url or bootstrap-dns
        bootstrap::set_resolver(
            async {
                let mut bootstrap_infos = server_infos
                    .iter()
                    .filter(|info| {
                        info.bootstrap_dns && {
                            if info.url.ip().is_none() {
                                warn!("bootstrap-dns must use ip addess, {:?}", info.url.host());
                                false
                            } else {
                                true
                            }
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                // try to use pure ip dns url as bootstrap-dns If bootstrap-dns is not set.
                if bootstrap_infos.is_empty() {
                    bootstrap_infos = server_infos
                        .iter()
                        .filter(|info| info.url.ip().is_some() && info.proxy.is_none())
                        .cloned()
                        .collect::<Vec<_>>()
                }

                if bootstrap_infos.is_empty() {
                    warn!("not bootstrap-dns found, use system_conf instead.");
                } else {
                    bootstrap_infos.dedup();
                }

                if !bootstrap_infos.is_empty() {
                    for info in &bootstrap_infos {
                        info!("bootstrap-dns {}", info.url.to_string());
                    }
                }

                let resolver: Arc<BootstrapResolver> = if !bootstrap_infos.is_empty() {
                    let new_resolver = factory
                        .create_name_server_group(
                            &bootstrap_infos,
                            &Default::default(),
                            client_subnet,
                        )
                        .await;
                    BootstrapResolver::new(new_resolver.into())
                } else {
                    BootstrapResolver::from_system_conf()
                }
                .into();

                resolver
            }
            .await,
        )
        .await;

        let server_group = {
            if server_infos.len() == 0 {
                warn!("no nameserver found, use system_conf instead.");
                bootstrap::resolver().await.as_ref().into()
            } else {
                debug!("initialize nameserver group {:?}", server_infos);
                Arc::new(
                    factory
                        .create_name_server_group(&server_infos, &proxies, client_subnet)
                        .await,
                )
            }
        };

        DnsClient {
            resolver_opts,
            server_group,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DnsClient {
    resolver_opts: ResolverOpts,
    server_group: Arc<NameServerGroup>,
}

impl DnsClient {
    pub fn builder() -> DnsClientBuilder {
        DnsClientBuilder::default()
    }

    pub async fn lookup_nameserver(&self, name: Name, record_type: RecordType) -> Option<Lookup> {
        bootstrap::resolver()
            .await
            .local_lookup(name, record_type)
            .await
    }
}

#[async_trait::async_trait]
impl GenericResolver for DnsClient {
    fn options(&self) -> &ResolverOpts {
        &self.resolver_opts
    }

    #[inline]
    async fn lookup<N: IntoName + Send, O: Into<LookupOptions> + Send + Clone>(
        &self,
        name: N,
        options: O,
    ) -> Result<Lookup, LookupError> {
        GenericResolver::lookup(self.server_group.as_ref(), name, options).await
    }
}

#[derive(Default, Debug, Clone)]
pub struct NameServerGroup {
    resolver_opts: ResolverOpts,
    servers: Vec<Arc<NameServer>>,
}

impl NameServerGroup {
    #[inline]
    pub fn iter(&self) -> Iter<Arc<NameServer>> {
        self.servers.iter()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.servers.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

#[async_trait::async_trait]
impl GenericResolver for NameServerGroup {
    fn options(&self) -> &ResolverOpts {
        &self.resolver_opts
    }

    async fn lookup<N: IntoName + Send, O: Into<LookupOptions> + Send + Clone>(
        &self,
        name: N,
        options: O,
    ) -> Result<Lookup, LookupError> {
        use futures_util::future::select_all;
        let name = name.into_name()?;
        let mut tasks = self
            .servers
            .iter()
            .map(|ns| GenericResolver::lookup(ns.as_ref(), name.clone(), options.clone()))
            .collect::<Vec<_>>();

        loop {
            let (res, _idx, rest) = select_all(tasks).await;

            if matches!(res.as_ref(), Ok(lookup) if !lookup.records().is_empty()) {
                return res;
            }

            if rest.is_empty() {
                return res;
            }
            tasks = rest;
        }
    }
}

#[derive(Debug, Clone)]
pub struct NameServerFactory {
    tls_client_config: TlsClientConfigBundle,
    cache: Arc<RwLock<HashMap<String, Arc<NameServer>>>>,
}

impl NameServerFactory {
    pub fn new(tls_client_config: TlsClientConfigBundle) -> Self {
        Self {
            tls_client_config,
            cache: Default::default(),
        }
    }

    pub async fn create(
        &self,
        url: &VerifiedDnsUrl,
        proxy: Option<ProxyConfig>,
        resolver_opts: NameServerOpts,
        connect_opts: ConnectOpts,
    ) -> Arc<NameServer> {
        use crate::libdns::resolver::name_server::NameServer as N;

        let key = format!(
            "{}{:?}",
            url.to_string(),
            proxy.as_ref().map(|s| s.to_string()),
        );

        if let Some(ns) = self.cache.read().await.get(&key) {
            return ns.clone();
        }

        let config = Self::create_config_from_url(url, self.tls_client_config.clone());

        let inner = N::<GenericConnector<TokioCustomeRuntimeProvider>>::new(
            config,
            resolver_opts.deref().to_owned(),
            GenericConnector::new(TokioCustomeRuntimeProvider::new(proxy, connect_opts)),
        );

        let ns = Arc::new(NameServer {
            opts: resolver_opts,
            inner,
        });
        self.cache.write().await.insert(key, ns.clone());
        ns
    }

    fn create_config_from_url(
        url: &VerifiedDnsUrl,
        tls_client_config: TlsClientConfigBundle,
    ) -> NameServerConfig {
        use crate::libdns::resolver::config::Protocol::*;

        let addr = url.addr();

        let tls_dns_name = Some(url.host().to_string());

        let tls_config = if url.proto().is_encrypted() {
            let config = if !url.ssl_verify() {
                tls_client_config.verify_off
            } else if url.sni_off() {
                tls_client_config.sni_off
            } else {
                tls_client_config.normal
            };

            Some(TlsClientConfig(config))
        } else {
            None
        };

        match url.proto() {
            Udp => NameServerConfig {
                socket_addr: addr,
                protocol: Protocol::Udp,
                tls_dns_name: None,
                tls_config: None,
                trust_negative_responses: true,
                bind_addr: None,
            },
            Tcp => NameServerConfig {
                socket_addr: addr,
                protocol: Protocol::Tcp,
                tls_dns_name: None,
                tls_config: None,
                trust_negative_responses: true,
                bind_addr: None,
            },
            Tls => NameServerConfig {
                socket_addr: addr,
                protocol: Protocol::Tls,
                tls_dns_name,
                trust_negative_responses: true,
                bind_addr: None,
                tls_config,
            },
            Https => NameServerConfig {
                socket_addr: addr,
                protocol: Protocol::Https,
                tls_dns_name,
                trust_negative_responses: true,
                bind_addr: None,
                tls_config,
            },
            Quic => NameServerConfig {
                socket_addr: addr,
                protocol: Protocol::Quic,
                tls_dns_name,
                trust_negative_responses: true,
                bind_addr: None,
                tls_config,
            },
            _ => unimplemented!(),
        }
    }

    async fn create_name_server_group(
        &self,
        infos: &[NameServerInfo],
        proxies: &HashMap<String, ProxyConfig>,
        default_client_subnet: Option<ClientSubnet>,
    ) -> NameServerGroup {
        let mut servers = vec![];

        let resolver = bootstrap::resolver().await;

        for info in infos {
            let url = info.url.clone();
            let verified_urls = match TryInto::<VerifiedDnsUrl>::try_into(url) {
                Ok(url) => vec![url],
                // if url is not a ip address, then try to resolve it.
                Err(url) => {
                    if let Some(domain) = url.domain() {
                        match resolver.lookup_ip(domain).await {
                            Ok(lookup_ip) => lookup_ip
                                .into_iter()
                                .map_while(|ip| {
                                    let mut url = url.clone();
                                    url.set_ip(ip);
                                    TryInto::<VerifiedDnsUrl>::try_into(url).ok()
                                })
                                .collect::<Vec<_>>(),
                            Err(err) => {
                                warn!("lookup ip: {domain} failed, {err}");
                                vec![]
                            }
                        }
                    } else {
                        vec![]
                    }
                }
            };

            let nameserver_opts = NameServerOpts::new(
                info.edns_client_subnet
                    .map(|x| x.into())
                    .or(default_client_subnet),
                resolver.options().clone(),
            );

            let proxy = info
                .proxy
                .as_deref()
                .map(|n| proxies.get(n))
                .unwrap_or_default()
                .cloned();

            for url in verified_urls {
                servers.push(
                    self.create(
                        &url,
                        proxy.clone(),
                        nameserver_opts.clone(),
                        Default::default(),
                    )
                    .await,
                )
            }
        }

        NameServerGroup {
            resolver_opts: resolver.options().to_owned(),
            servers,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct NameServerOpts {
    client_subnet: Option<ClientSubnet>,
    resolver_opts: ResolverOpts,
}

impl NameServerOpts {
    #[inline]
    pub fn new(client_subnet: Option<ClientSubnet>, resolver_opts: ResolverOpts) -> Self {
        Self {
            client_subnet,
            resolver_opts,
        }
    }

    pub fn with_resolver_opts(mut self, resolver_opts: ResolverOpts) -> Self {
        self.resolver_opts = resolver_opts;
        self
    }
}

impl Deref for NameServerOpts {
    type Target = ResolverOpts;

    fn deref(&self) -> &Self::Target {
        &self.resolver_opts
    }
}

#[derive(Debug, Clone)]
pub struct NameServer {
    opts: NameServerOpts,
    inner: libdns::resolver::name_server::NameServer<GenericConnector<TokioCustomeRuntimeProvider>>,
}

impl NameServer {
    pub fn new(
        config: NameServerConfig,
        opts: NameServerOpts,
        proxy: Option<ProxyConfig>,
        connect_opts: ConnectOpts,
    ) -> NameServer {
        use crate::libdns::resolver::name_server::NameServer as N;

        let inner = N::<GenericConnector<TokioCustomeRuntimeProvider>>::new(
            config,
            opts.resolver_opts.clone(),
            GenericConnector::new(TokioCustomeRuntimeProvider::new(proxy, connect_opts)),
        );

        Self { opts, inner }
    }

    #[inline]
    pub fn options(&self) -> &NameServerOpts {
        &self.opts
    }
}

#[async_trait::async_trait]
impl GenericResolver for NameServer {
    fn options(&self) -> &ResolverOpts {
        &self.opts
    }

    async fn lookup<N: IntoName + Send, O: Into<LookupOptions> + Send + Clone>(
        &self,
        name: N,
        options: O,
    ) -> Result<Lookup, LookupError> {
        let name = name.into_name()?;
        let options: LookupOptions = options.into();

        let request_options = {
            let opts = &self.options();
            let mut request_opts = DnsRequestOptions::default();
            request_opts.recursion_desired = opts.recursion_desired;
            request_opts.use_edns = opts.edns0;
            request_opts
        };

        let query = Query::query(name, options.record_type);

        let client_subnet = options.client_subnet.or(self.opts.client_subnet);

        let req = DnsRequest::new(
            build_message(query, request_options, client_subnet),
            request_options,
        );

        let ns = self.inner.clone();

        let res = ns.send(req).first_answer().await?;

        let valid_until = Instant::now()
            + Duration::from_secs(
                res.answers()
                    .iter()
                    .map(|r| r.ttl())
                    .min()
                    .unwrap_or(MAX_TTL) as u64,
            );

        Ok(Lookup::new_with_deadline(
            res.query().unwrap().clone(),
            res.answers().into(),
            valid_until,
        ))
    }
}

pub struct VerifiedDnsUrl(DnsUrl);

impl VerifiedDnsUrl {
    #[allow(unused)]
    pub fn ip(&self) -> IpAddr {
        self.0.ip().expect("VerifiedDnsUrl must have ip.")
    }

    pub fn addr(&self) -> SocketAddr {
        self.0
            .addr()
            .expect("VerifiedDnsUrl must have socket address.")
    }
}

impl Deref for VerifiedDnsUrl {
    type Target = DnsUrl;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::convert::TryFrom<DnsUrl> for VerifiedDnsUrl {
    type Error = DnsUrl;

    fn try_from(value: DnsUrl) -> Result<Self, Self::Error> {
        if value.ip().is_none() {
            return Err(value);
        }
        Ok(Self(value))
    }
}

/// > An EDNS buffer size of 1232 bytes will avoid fragmentation on nearly all current networks.
/// https://dnsflagday.net/2020/
const MAX_PAYLOAD_LEN: u16 = 1232;

fn build_message(
    query: Query,
    request_options: DnsRequestOptions,
    client_subnet: Option<ClientSubnet>,
) -> Message {
    // build the message
    let mut message: Message = Message::new();
    // TODO: This is not the final ID, it's actually set in the poll method of DNS future
    //  should we just remove this?
    let id: u16 = rand::random();
    message
        .add_query(query)
        .set_id(id)
        .set_message_type(MessageType::Query)
        .set_op_code(OpCode::Query)
        .set_recursion_desired(request_options.recursion_desired);

    // Extended dns
    if client_subnet.is_some() || request_options.use_edns {
        message
            .extensions_mut()
            .get_or_insert_with(Edns::new)
            .set_max_payload(MAX_PAYLOAD_LEN)
            .set_version(0);

        if let (Some(client_subnet), Some(edns)) = (client_subnet, message.extensions_mut()) {
            edns.options_mut().insert(EdnsOption::Subnet(client_subnet));
        }
    }
    message
}

mod connection_provider {
    use std::{future::Future, io::Result, net::SocketAddr, pin::Pin};

    use tokio::net::UdpSocket as TokioUdpSocket;

    use swiftlink_infra::net::ConnectOpts;

    use crate::{
        libdns::{
            proto::{iocompat::AsyncIoTokioAsStd, TokioTime},
            resolver::{name_server::RuntimeProvider, TokioHandle},
        },
        proxy::{self, ProxyConfig, TcpStream},
    };

    /// The swiftlink dns Tokio Runtime for async execution
    #[derive(Clone)]
    pub struct TokioCustomeRuntimeProvider {
        proxy: Option<ProxyConfig>,
        connect_opts: ConnectOpts,
        handle: TokioHandle,
    }

    impl TokioCustomeRuntimeProvider {
        pub fn new(proxy: Option<ProxyConfig>, connect_opts: ConnectOpts) -> Self {
            Self {
                proxy,
                connect_opts,
                handle: TokioHandle::default(),
            }
        }
    }

    impl RuntimeProvider for TokioCustomeRuntimeProvider {
        type Handle = TokioHandle;
        type Timer = TokioTime;
        type Udp = TokioUdpSocket;
        type Tcp = AsyncIoTokioAsStd<TcpStream>;

        fn create_handle(&self) -> Self::Handle {
            self.handle.clone()
        }

        fn connect_tcp(
            &self,
            server_addr: SocketAddr,
        ) -> Pin<Box<dyn Send + Future<Output = Result<Self::Tcp>>>> {
            // TODO: bind interface
            let proxy_config = self.proxy.clone();
            let connect_opts = self.connect_opts.clone();

            Box::pin(async move {
                proxy::connect_tcp(server_addr, proxy_config.as_ref(), &connect_opts)
                    .await
                    .map(AsyncIoTokioAsStd)
            })
        }

        fn bind_udp(
            &self,
            local_addr: SocketAddr,
            _server_addr: SocketAddr,
        ) -> Pin<Box<dyn Send + Future<Output = Result<Self::Udp>>>> {
            // TODO: bind addr
            Box::pin(TokioUdpSocket::bind(local_addr))
        }
    }
}

mod bootstrap {
    use crate::libdns::{
        proto::rr::RecordType,
        resolver::{
            config::{NameServerConfigGroup, ResolverConfig},
            Name,
        },
    };

    use super::*;

    static RESOLVER: RwLock<Option<Arc<BootstrapResolver>>> = RwLock::const_new(None);

    pub async fn resolver() -> Arc<BootstrapResolver> {
        let lock = RESOLVER.read().await;
        if lock.is_none() {
            drop(lock);
            let resolver: Arc<BootstrapResolver> = Arc::new(BootstrapResolver::from_system_conf());
            set_resolver(resolver.clone()).await;
            resolver
        } else {
            lock.as_ref().unwrap().clone()
        }
    }

    pub async fn set_resolver(resolver: Arc<BootstrapResolver>) {
        *(RESOLVER.write().await) = Some(resolver)
    }

    pub struct BootstrapResolver<T: GenericResolver = NameServerGroup>
    where
        T: Send + Sync,
    {
        resolver: Arc<T>,
        ip_store: RwLock<HashMap<Query, Arc<[Record]>>>,
    }

    impl<T: GenericResolver + Sync + Send> BootstrapResolver<T> {
        pub fn new(resolver: Arc<T>) -> Self {
            Self {
                resolver,
                ip_store: Default::default(),
            }
        }

        pub fn with_new_resolver(self, resolver: Arc<T>) -> Self {
            Self {
                resolver,
                ip_store: self.ip_store,
            }
        }

        pub async fn local_lookup(&self, name: Name, record_type: RecordType) -> Option<Lookup> {
            let query = Query::query(name.clone(), record_type);
            let store = self.ip_store.read().await;

            let lookup = store.get(&query).cloned();

            lookup.map(|records| Lookup::new_with_max_ttl(query, records))
        }
    }

    impl BootstrapResolver<NameServerGroup> {
        pub fn from_system_conf() -> Self {
            let (resolv_config, resolv_opts) =
                crate::libdns::resolver::system_conf::read_system_conf().unwrap_or_else(|err| {
                    warn!("read system conf failed, {}", err);

                    use crate::preset_ns::{ALIDNS, ALIDNS_IPS, CLOUDFLARE, CLOUDFLARE_IPS};

                    let mut name_servers = NameServerConfigGroup::from_ips_https(
                        ALIDNS_IPS,
                        443,
                        ALIDNS.to_string(),
                        true,
                    );
                    name_servers.merge(NameServerConfigGroup::from_ips_https(
                        CLOUDFLARE_IPS,
                        443,
                        CLOUDFLARE.to_string(),
                        true,
                    ));

                    let mut resolv_opts = ResolverOpts::default();
                    // TODO: cache_size should configurable?
                    resolv_opts.cache_size = 256;
                    (
                        ResolverConfig::from_parts(None, vec![], name_servers),
                        resolv_opts,
                    )
                });
            let mut name_servers = vec![];

            for config in resolv_config.name_servers() {
                name_servers.push(Arc::new(super::NameServer::new(
                    config.clone(),
                    Default::default(),
                    None,
                    Default::default(),
                )));
            }

            Self::new(Arc::new(NameServerGroup {
                resolver_opts: resolv_opts.to_owned(),
                servers: name_servers,
            }))
        }
    }

    #[async_trait::async_trait]
    impl<T: GenericResolver + Sync + Send> GenericResolver for BootstrapResolver<T> {
        fn options(&self) -> &ResolverOpts {
            self.resolver.options()
        }

        #[inline]
        async fn lookup<N: IntoName + Send, O: Into<LookupOptions> + Send + Clone>(
            &self,
            name: N,
            options: O,
        ) -> Result<Lookup, LookupError> {
            let name = name.into_name()?;
            let options: LookupOptions = options.into();
            let record_type = options.record_type;
            if let Some(lookup) = self.local_lookup(name.clone(), record_type).await {
                return Ok(lookup);
            }

            match GenericResolver::lookup(self.resolver.as_ref(), name.clone(), options).await {
                Ok(lookup) => {
                    let records = lookup.records().to_vec();

                    debug!(
                        "lookup nameserver {} {}, {:?}",
                        name,
                        record_type,
                        records
                            .iter()
                            .flat_map(|r| r.data().map(|d| d.ip_addr()))
                            .flatten()
                            .collect::<Vec<_>>()
                    );

                    self.ip_store.write().await.insert(
                        Query::query(
                            {
                                let mut name = name.clone();
                                name.set_fqdn(true);
                                name
                            },
                            record_type,
                        ),
                        records.into(),
                    );

                    Ok(lookup)
                }
                err => err,
            }
        }
    }

    impl<T: GenericResolver + Sync + Send> From<Arc<T>> for BootstrapResolver<T> {
        fn from(resolver: Arc<T>) -> Self {
            Self::new(resolver)
        }
    }

    impl<T: GenericResolver + Sync + Send> From<&BootstrapResolver<T>> for Arc<T> {
        fn from(value: &BootstrapResolver<T>) -> Self {
            value.resolver.clone()
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        dns_url::DnsUrl,
        preset_ns::{ALIDNS_IPS, CLOUDFLARE_IPS},
    };
    use std::net::IpAddr;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_with_default() {
        let client = DnsClient::builder().build().await;
        let lookup_ip = client
            .lookup("dns.alidns.com", RecordType::A)
            .await
            .unwrap();
        assert!(lookup_ip.into_iter().any(|i| i.ip_addr()
            == Some("223.5.5.5".parse::<IpAddr>().unwrap())
            || i.ip_addr() == Some("223.6.6.6".parse::<IpAddr>().unwrap())));
    }

    async fn assert_google(client: &DnsClient) {
        let name = "dns.google";
        let addrs = client
            .lookup_ip(name)
            .await
            .unwrap()
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        // println!("name: {} addrs => {}", name, addrs);

        assert!(addrs.contains("8.8.8.8"));
    }

    async fn assert_alidns(client: &DnsClient) {
        let name = "dns.alidns.com";
        let addrs = client
            .lookup_ip(name)
            .await
            .unwrap()
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        // println!("name: {} addrs => {}", name, addrs);

        assert!(addrs.contains("223.5.5.5") || addrs.contains("223.6.6.6"));
    }

    #[tokio::test]
    #[ignore = "reason"]
    async fn test_nameserver_google_tls_resolve() {
        let dns_url = DnsUrl::from_str("tls://dns.google?enable_sni=false").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_cloudflare_resolve() {
        // todo:// support alias.
        let dns_urls = CLOUDFLARE_IPS.iter().map(DnsUrl::from).collect::<Vec<_>>();

        let client = DnsClient::builder().add_servers(dns_urls).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_cloudflare_https_resolve() {
        let dns_url = DnsUrl::from_str("https://dns.cloudflare.com/dns-query").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    #[ignore = "reason"]
    async fn test_nameserver_cloudflare_tls_resolve() {
        let dns_url = DnsUrl::from_str("tls://dns.cloudflare.com?enable_sni=false").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_quad9_tls_resolve() {
        let dns_url = DnsUrl::from_str("tls://dns.quad9.net?enable_sni=false").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_quad9_dns_url_https_resolve() {
        let dns_url = DnsUrl::from_str("https://dns.quad9.net/dns-query").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_alidns_resolve() {
        // todo:// support alias.
        let dns_urls = ALIDNS_IPS.iter().map(DnsUrl::from).collect::<Vec<_>>();

        let client = DnsClient::builder().add_servers(dns_urls).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_alidns_dns_url_https_resolve() {
        let dns_url = DnsUrl::from_str("https://dns.alidns.com/dns-query").unwrap();

        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_alidns_dns_url_tls_resolve() {
        let dns_url = DnsUrl::from_str("tls://dns.alidns.com").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_alidns_https_tls_name_with_ip_resolve() {
        let dns_url = DnsUrl::from_str("https://223.5.5.5/dns-query").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_dnspod_https_resolve() {
        let dns_url = DnsUrl::from_str("https://doh.pub/dns-query").unwrap();

        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    async fn test_nameserver_dnspod_tls_resolve() {
        let dns_url = DnsUrl::from_str("tls://dot.pub").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;

        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    #[ignore = "not available now"]
    async fn test_nameserver_adguard_https_resolve() {
        let dns_url = DnsUrl::from_str("https://dns.adguard-dns.com/dns-query").unwrap();

        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }

    #[tokio::test]
    #[ignore = "not available now"]
    async fn test_nameserver_adguard_quic_resolve() {
        let dns_url = DnsUrl::from_str("quic://dns.adguard-dns.com").unwrap();
        let client = DnsClient::builder().add_server(dns_url).build().await;
        assert_google(&client).await;
        assert_alidns(&client).await;
    }
}
