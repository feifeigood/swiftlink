use std::str::FromStr;

use ipnet::IpNet;
use serde::Deserialize;
use serde_with::{serde_as, DeserializeFromStr};
use tracing::warn;

use swiftlink_infra::{parse, Listener};

use crate::dns_url::{DnsUrl, DnsUrlParamExt};

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
#[serde_as]
pub struct Config {
    /// dns server bind ip and port, default dns server port is 53, support binding multi ip and port
    binds: Vec<Listener>,

    /// tcp connection idle timeout
    ///
    /// tcp-idle-time [second]
    tcp_idle_time: Option<u64>,

    /// remote dns server list
    nameservers: Vec<NameServerInfo>,

    /// check /etc/hosts file before dns request (only works for unix like OS)
    use_hosts_file: bool,

    /// edns client subnet
    ///
    /// ```
    /// example:
    ///   edns-client-subnet [ip/subnet]
    ///   edns-client-subnet 192.168.1.1/24
    ///   edns-client-subnet 8::8/56
    /// ```
    edns_client_subnet: Option<IpNet>,
}

impl Config {
    pub fn binds(&self) -> &[Listener] {
        &self.binds
    }

    pub fn tcp_idle_time(&self) -> u64 {
        self.tcp_idle_time.unwrap_or(120)
    }
}

#[derive(DeserializeFromStr, Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameServerInfo {
    /// the nameserver url.
    pub url: DnsUrl,

    /// set server to group, use with nameserver /domain/group.
    pub group: Vec<String>,

    /// result must exist edns RR, or discard result.
    pub check_edns: bool,

    /// set as bootstrap dns server
    pub bootstrap_dns: bool,

    /// use proxy to connect to server.
    pub proxy: Option<String>,

    /// exclude this server from default group.
    pub exclude_default_group: bool,

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
            let mut exclude_default_group = false;
            let mut group = vec![];
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
                        "-exclude-default-group" | "--exclude-default-group" => {
                            exclude_default_group = true
                        }
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
                        "-group" | "--group" => {
                            group.push(parts.next().expect("group name").to_string())
                        }
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
                group,
                exclude_default_group,
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
            group: vec![],
            exclude_default_group: false,
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

    use hickory_resolver::config::Protocol;
    use swiftlink_infra::IListener;

    use super::*;

    #[test]
    fn test_config_bind_with_device() {
        let cfg_str = r#"
        binds = ["0.0.0.0:4453@eth0"]
        "#;

        let cfg: Config = toml::from_str(&cfg_str).unwrap();

        assert_eq!(cfg.binds().len(), 1);

        let bind = cfg.binds().get(0).unwrap();

        assert_eq!(bind.sock_addr(), "0.0.0.0:4453".parse().unwrap());

        assert_eq!(bind.device(), Some("eth0"));
    }

    #[test]
    fn test_config_nameserver() {
        let cfg_str = r#"
        servers = ["https://223.5.5.5/dns-query -bootstrap-dns"]
        "#;

        let cfg: Config = toml::from_str(&cfg_str).unwrap();

        assert_eq!(cfg.nameservers.len(), 1);

        let server = cfg.nameservers.get(0).unwrap();
        assert_eq!(server.url.proto(), &Protocol::Https);
        assert_eq!(server.url.to_string(), "https://223.5.5.5/dns-query");
        assert_eq!(server.bootstrap_dns, true);
    }

    #[test]
    fn test_config_dns_client_subnet() {
        let cfg_str = r#"
        edns_client_subnet = "192.168.1.1/24"
        "#;

        let cfg: Config = toml::from_str(&cfg_str).unwrap();

        assert!(cfg.edns_client_subnet.is_some());

        let edns_client_subnet = cfg.edns_client_subnet.unwrap();
        assert!(edns_client_subnet.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert_eq!(Ok(edns_client_subnet.netmask()), "255.255.255.0".parse());
    }
}
