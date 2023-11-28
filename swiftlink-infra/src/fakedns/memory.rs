use std::{
    fmt::Debug,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    num::NonZeroUsize,
};

use bimap::BiMap;
use lru::LruCache;

use super::IFakeIPStore;

pub struct MemoryStore {
    host2ip4: BiMap<String, Ipv4Addr>,
    host2ip6: BiMap<String, Ipv6Addr>,
    host_lru: LruCache<String, ()>,
}

impl MemoryStore {
    pub fn new(size: usize) -> Self {
        Self {
            host2ip6: BiMap::with_capacity(size),
            host2ip4: BiMap::with_capacity(size),
            host_lru: LruCache::new(NonZeroUsize::new(size).unwrap()),
        }
    }
}

impl IFakeIPStore for MemoryStore {
    fn get_fakeip<K: AsRef<[u8]>>(&mut self, k: K, ipv6: bool) -> Option<Vec<u8>> {
        let host_or_ip = String::from_utf8(k.as_ref().to_vec()).unwrap_or_default();
        // judge K whether is ip or host
        match host_or_ip.parse::<IpAddr>() {
            Ok(ip) => match ip {
                IpAddr::V4(ip) => {
                    if let Some(host) = self.host2ip4.get_by_right(&ip) {
                        // ensure host in the head of lru list
                        _ = self.host_lru.get(host);
                        return Some(host.as_bytes().to_vec());
                    }
                }
                IpAddr::V6(ip) => {
                    if let Some(host) = self.host2ip6.get_by_right(&ip) {
                        // ensure host in the head of lru list
                        _ = self.host_lru.get(host);
                        return Some(host.as_bytes().to_vec());
                    }
                }
            },
            // the K maybe is host
            Err(_) => {
                let ip: Option<IpAddr> = if !ipv6 {
                    self.host2ip4
                        .get_by_left(host_or_ip.as_str())
                        .map(|x| IpAddr::V4(x.to_owned()))
                } else {
                    self.host2ip6
                        .get_by_left(host_or_ip.as_str())
                        .map(|x| IpAddr::V6(x.to_owned()))
                };

                if let Some(ip) = ip {
                    // ensure host in the head of lru list
                    _ = self.host_lru.get(host_or_ip.as_str());
                    return Some(ip.to_string().as_bytes().to_vec());
                }
            }
        }

        None
    }

    fn put_fakeip(&mut self, host: &str, ip: IpAddr) -> io::Result<()> {
        let entry = self.host_lru.push(host.into(), ());
        // remove host due to the lru capacity
        if let Some((ref old_host, _)) = entry {
            if !host.eq_ignore_ascii_case(&old_host) {
                _ = self.host2ip4.remove_by_left(old_host);
                _ = self.host2ip6.remove_by_left(old_host);
            }
        }

        match ip {
            IpAddr::V4(ip) => _ = self.host2ip4.insert(host.into(), ip),
            IpAddr::V6(ip) => _ = self.host2ip6.insert(host.into(), ip),
        };

        Ok(())
    }

    fn delete_fakeip(&mut self, host: &str, _ip: IpAddr) -> io::Result<()> {
        _ = self.host2ip4.remove_by_left(host);
        _ = self.host2ip6.remove_by_left(host);
        _ = self.host_lru.pop(host);

        Ok(())
    }

    fn exists(&mut self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => self.host2ip4.get_by_right(&ip).is_some(),
            IpAddr::V6(ip) => self.host2ip6.get_by_right(&ip).is_some(),
        }
    }
}

impl Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Memory")
    }
}
