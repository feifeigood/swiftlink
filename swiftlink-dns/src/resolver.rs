use std::sync::Arc;

use enum_dispatch::enum_dispatch;
use swiftlink_infra::net::ConnectOpts;

use crate::{
    client::DnsClient,
    error::LookupError,
    libdns::{
        proto::{
            error::ProtoResult,
            op::Query,
            rr::{rdata::opt::ClientSubnet, Record, RecordType},
        },
        resolver::{config::ResolverOpts, lookup::Lookup, lookup_ip::LookupIp, IntoName, Name, TryParseIp},
    },
    DnsConfig, MAX_TTL,
};

#[derive(Clone)]
pub struct LookupOptions {
    pub record_type: RecordType,
    pub client_subnet: Option<ClientSubnet>,
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            record_type: RecordType::A,
            client_subnet: Default::default(),
        }
    }
}

impl From<RecordType> for LookupOptions {
    fn from(record_type: RecordType) -> Self {
        Self {
            record_type,
            ..Default::default()
        }
    }
}

#[async_trait::async_trait]
pub trait GenericResolver {
    fn options(&self) -> &ResolverOpts;

    /// Lookup any RecordType
    ///
    /// # Arguments
    ///
    /// * `name` - name of the record to lookup, if name is not a valid domain name, an error will be returned
    /// * `record_type` - type of record to lookup, all RecordData responses will be filtered to this type
    ///
    /// # Returns
    ///
    ///  A future for the returned Lookup RData
    async fn lookup<N: IntoName + Send, O: Into<LookupOptions> + Send + Clone>(
        &self,
        name: N,
        options: O,
    ) -> Result<Lookup, LookupError>;
}

#[async_trait::async_trait]
pub trait GenericResolverExt {
    /// Generic lookup for any RecordType
    ///
    /// # Arguments
    ///
    /// * `name` - name of the record to lookup, if name is not a valid domain name, an error will be returned
    /// * `record_type` - type of record to lookup, all RecordData responses will be filtered to this type
    ///
    /// # Returns
    ///
    //  A future for the returned Lookup RData
    // async fn lookup<N: IntoName + Send>(
    //     &self,
    //     name: N,
    //     record_type: RecordType,
    // ) -> Result<Lookup, ResolveError>;

    /// Performs a dual-stack DNS lookup for the IP for the given hostname.
    ///
    /// See the configuration and options parameters for controlling the way in which A(Ipv4) and AAAA(Ipv6) lookups will be performed. For the least expensive query a fully-qualified-domain-name, FQDN, which ends in a final `.`, e.g. `www.example.com.`, will only issue one query. Anything else will always incur the cost of querying the `ResolverConfig::domain` and `ResolverConfig::search`.
    ///
    /// # Arguments
    /// * `host` - string hostname, if this is an invalid hostname, an error will be returned.
    async fn lookup_ip<N: IntoName + TryParseIp + Send>(&self, host: N) -> Result<LookupIp, LookupError>;
}

#[async_trait::async_trait]
impl<T> GenericResolverExt for T
where
    T: GenericResolver + Sync,
{
    /// * `host` - string hostname, if this is an invalid hostname, an error will be returned.
    async fn lookup_ip<N: IntoName + TryParseIp + Send>(&self, host: N) -> Result<LookupIp, LookupError> {
        let mut finally_ip_addr: Option<Record> = None;
        let maybe_ip = host.try_parse_ip();
        let maybe_name: ProtoResult<Name> = host.into_name();

        // if host is a ip address, return directly.
        if let Some(ip_addr) = maybe_ip {
            let name = maybe_name.clone().unwrap_or_default();
            let record = Record::from_rdata(name.clone(), MAX_TTL, ip_addr.clone());

            // if ndots are greater than 4, then we can't assume the name is an IpAddr
            //   this accepts IPv6 as well, b/c IPv6 can take the form: 2001:db8::198.51.100.35
            //   but `:` is not a valid DNS character, so technically this will fail parsing.
            //   TODO: should we always do search before returning this?
            if self.options().ndots > 4 {
                finally_ip_addr = Some(record);
            } else {
                let query = Query::query(name, ip_addr.record_type());
                let lookup = Lookup::new_with_max_ttl(query, Arc::from([record]));
                return Ok(lookup.into());
            }
        }

        let name = match (maybe_name, finally_ip_addr.as_ref()) {
            (Ok(name), _) => name,
            (Err(_), Some(ip_addr)) => {
                // it was a valid IP, return that...
                let query = Query::query(ip_addr.name().clone(), ip_addr.record_type());
                let lookup = Lookup::new_with_max_ttl(query, Arc::from([ip_addr.clone()]));
                return Ok(lookup.into());
            }
            (Err(err), None) => {
                return Err(err.into());
            }
        };

        // TODO: search hosts first

        let strategy = self.options().ip_strategy;

        use crate::libdns::resolver::config::LookupIpStrategy::*;

        match strategy {
            Ipv4Only => self.lookup(name.clone(), RecordType::A).await,
            Ipv6Only => self.lookup(name.clone(), RecordType::AAAA).await,
            Ipv4AndIpv6 => {
                use futures_util::future::{select, Either};
                match select(
                    self.lookup(name.clone(), RecordType::A),
                    self.lookup(name.clone(), RecordType::AAAA),
                )
                .await
                {
                    Either::Left((res, _)) => res,
                    Either::Right((res, _)) => res,
                }
            }
            Ipv6thenIpv4 => match self.lookup(name.clone(), RecordType::AAAA).await {
                Ok(lookup) => Ok(lookup),
                Err(_err) => self.lookup(name.clone(), RecordType::A).await,
            },
            Ipv4thenIpv6 => match self.lookup(name.clone(), RecordType::A).await {
                Ok(lookup) => Ok(lookup),
                Err(_err) => self.lookup(name.clone(), RecordType::AAAA).await,
            },
        }
        .map(|lookup| lookup.into())
    }
}

/// Abstract DNS resolver
#[async_trait::async_trait]
#[enum_dispatch]
pub trait IDnsResolver {
    async fn lookup_ip<N: IntoName + TryParseIp + Send>(&self, host: N) -> Result<LookupIp, LookupError>;
}

#[derive(Debug, Clone)]
pub struct DnsResolver {
    client: Arc<DnsClient>,
}

impl Into<Arc<DnsClient>> for DnsResolver {
    fn into(self) -> Arc<DnsClient> {
        self.client.to_owned()
    }
}

pub async fn build_dns_resolver(dns: &DnsConfig, connect_opts: &ConnectOpts) -> DnsResolver {
    if !dns.enabled() {
        let client: Arc<DnsClient> = Arc::new(DnsClient::builder().build().await);
        return DnsResolver { client };
    }

    let servers = dns.servers();
    let proxies = dns.proxies().clone();

    let mut builder = DnsClient::builder();
    builder = builder.add_servers(servers.to_vec());

    builder = builder.with_connect_opts(connect_opts.clone());

    if let Some(subnet) = dns.edns_client_subnet() {
        builder = builder.with_client_subnet(subnet);
    }

    builder = builder.with_proxies(proxies);

    let client = Arc::new(builder.build().await);

    DnsResolver { client }
}
