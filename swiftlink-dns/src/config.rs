use std::{collections::HashMap, str::FromStr, sync::Arc};

use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use serde::Deserialize;
use serde_with::{serde_as, DeserializeFromStr};

use swiftlink_infra::{log::warn, parse, Listener};

use crate::{
    dns_url::{DnsUrl, DnsUrlParamExt},
    proxy::ProxyConfig,
};

#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
#[serde_as]
pub struct DnsConfig {
    /// dns server enable
    enable: bool,

    /// dns server bind ip and port, default dns server port is 53
    listen: Listener,

    /// remote dns server list
    #[serde(rename = "nameserver")]
    servers: Vec<NameServerInfo>,

    /// edns client subnet
    ///
    /// ```
    /// example:
    ///   edns-client-subnet [ip/subnet]
    ///   edns-client-subnet 192.168.1.1/24
    ///   edns-client-subnet 8::8/56
    /// ```
    edns_client_subnet: Option<IpNet>,

    fake_ip: bool,
    fake_ip_size: Option<usize>,
    fake_ip_persist: bool,
    fake_ip_range: Option<Ipv4Net>,
    fake_ip6_range: Option<Ipv6Net>,

    /// The proxy server for upstream querying.
    #[serde_as(as = "Arc<HashMap<_,DisplayFromStr>>")]
    proxy_servers: Arc<HashMap<String, ProxyConfig>>,
}

impl DnsConfig {
    pub fn enabled(&self) -> bool {
        self.enable
    }

    pub fn listen(&self) -> Listener {
        let listener = self.listen.clone();
        if listener.sock_addr().port() <= 0 {
            listener.sock_addr().set_port(53);
        }

        listener
    }

    pub fn servers(&self) -> &[NameServerInfo] {
        &self.servers
    }

    pub fn proxies(&self) -> &Arc<HashMap<String, ProxyConfig>> {
        &self.proxy_servers
    }

    #[inline]
    pub fn edns_client_subnet(&self) -> Option<IpNet> {
        self.edns_client_subnet
    }

    #[inline]
    pub fn fakeip(&self) -> bool {
        self.fake_ip
    }

    #[inline]
    pub fn fakeip_size(&self) -> Option<usize> {
        self.fake_ip_size
    }

    #[inline]
    pub fn fakeip_persist(&self) -> bool {
        self.fake_ip_persist
    }

    #[inline]
    pub fn fakeip_range(&self) -> (Option<Ipv4Net>, Option<Ipv6Net>) {
        (self.fake_ip_range, self.fake_ip6_range)
    }
}

#[derive(DeserializeFromStr, Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameServerInfo {
    /// the nameserver url.
    pub url: DnsUrl,

    /// result must exist edns RR, or discard result.
    pub check_edns: bool,

    /// set as bootstrap dns server
    pub bootstrap_dns: bool,

    /// use proxy to connect to server.
    pub proxy: Option<String>,

    /// edns client subnet
    ///
    /// ```
    /// example:
    ///   edns-client-subnet [ip/subnet]
    ///   edns-client-subnet 192.168.1.1/24
    ///   edns-client-subnet 8::8/56
    /// ```
    pub edns_client_subnet: Option<IpNet>,
}

#[derive(Debug, thiserror::Error)]
pub enum NameServerParseErr {
    #[error("invalid dns url {0}")]
    InvalidDnsUrl(String),
}

impl FromStr for NameServerInfo {
    type Err = NameServerParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = parse::split_options(s, ' ');

        if let Some(Ok(mut url)) = parts.next().map(DnsUrl::from_str) {
            let mut bootstrap_dns = false;
            let mut check_edns = false;
            let mut edns_client_subnet = None;
            let mut proxy = None;

            while let Some(part) = parts.next() {
                if part.is_empty() {
                    continue;
                }
                if part.starts_with('-') {
                    match part.trim_end_matches(':') {
                        "-bootstrap-dns" | "--bootstrap-dns" => bootstrap_dns = true,
                        "-host-name" | "--host-name" => {
                            if let Some(host_name) =
                                Some(parts.next().expect("host name").to_string())
                            {
                                if host_name == "-" {
                                    url.set_sni_off(true);
                                } else {
                                    url.set_host(&host_name);
                                }
                            }
                        }
                        "-check-edns" | "--check-edns" => check_edns = true,
                        "-proxy" | "--proxy" => {
                            proxy = Some(parts.next().expect("proxy name").to_string())
                        }
                        "-subnet" | "--subnet" => {
                            edns_client_subnet =
                                parts.next().expect("edns client subnet").parse().ok()
                        }
                        _ => warn!("unknow nameserver options {}", part),
                    }
                } else {
                    warn!("ignore: {}", part);
                }
            }

            Ok(Self {
                url,
                check_edns,
                bootstrap_dns,
                proxy,
                edns_client_subnet,
            })
        } else {
            Err(NameServerParseErr::InvalidDnsUrl(s.into()))
        }
    }
}

impl From<DnsUrl> for NameServerInfo {
    fn from(url: DnsUrl) -> Self {
        Self {
            url,
            bootstrap_dns: false,
            check_edns: false,
            proxy: None,
            edns_client_subnet: None,
        }
    }
}

#[cfg(test)]
mod tests {

    use std::net::{IpAddr, Ipv4Addr};

    use crate::libdns::resolver::config::Protocol;

    use crate::proxy::ProxyProtocol;

    use super::*;

    #[test]
    fn test_config_listen() {
        let cfg_str = r#"
        listen = "0.0.0.0:4453"
        "#;

        let cfg: DnsConfig = toml::from_str(&cfg_str).unwrap();

        assert_eq!(cfg.listen().sock_addr(), "0.0.0.0:4453".parse().unwrap());
    }

    #[test]
    fn test_config_nameserver() {
        let cfg_str = r#"
        servers = ["https://223.5.5.5/dns-query -bootstrap-dns -proxy mysocks5"]
        "#;

        let cfg: DnsConfig = toml::from_str(&cfg_str).unwrap();

        assert_eq!(cfg.servers.len(), 1);

        let server = cfg.servers.get(0).unwrap();
        assert_eq!(server.url.proto(), &Protocol::Https);
        assert_eq!(server.url.to_string(), "https://223.5.5.5/dns-query");
        assert_eq!(server.bootstrap_dns, true);
    }

    #[test]
    fn test_config_dns_client_subnet() {
        let cfg_str = r#"
        edns_client_subnet = "192.168.1.1/24"
        "#;

        let cfg: DnsConfig = toml::from_str(&cfg_str).unwrap();

        assert!(cfg.edns_client_subnet.is_some());

        let edns_client_subnet = cfg.edns_client_subnet.unwrap();
        assert!(edns_client_subnet.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert_eq!(Ok(edns_client_subnet.netmask()), "255.255.255.0".parse());
    }

    #[test]
    fn test_config_proxy_server() {
        let cfg_str = r#"
        [proxy_servers]
        mysocks5proxy = "socks5://user:pass@1.2.3.4:1080"
        myhttpproxy = "http://user:pass@1.2.3.4:3128"
        "#;

        let cfg: DnsConfig = toml::from_str(&cfg_str).unwrap();

        let proxies = cfg.proxies();

        assert!(proxies.len() == 2);
        assert_eq!(
            proxies.get("mysocks5proxy").unwrap().proto,
            ProxyProtocol::Socks5
        );
        assert_eq!(
            proxies.get("mysocks5proxy").unwrap().username,
            Some("user".to_string())
        );
        assert_eq!(
            proxies.get("mysocks5proxy").unwrap().password,
            Some("pass".to_string())
        );
        assert_eq!(
            proxies.get("mysocks5proxy").unwrap().server,
            "1.2.3.4:1080".parse().unwrap()
        );

        assert_eq!(
            proxies.get("myhttpproxy").unwrap().proto,
            ProxyProtocol::Http
        );
    }
}
