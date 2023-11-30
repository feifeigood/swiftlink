use std::{
    fmt::Debug,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use enum_dispatch::enum_dispatch;

use crate::log::*;
use crate::trie::domain_trie::DomainTrie;

use cachefile::CacheFileStore;
use memory::MemoryStore;

mod cachefile;
mod memory;

#[enum_dispatch]
pub trait IFakeIPStore {
    // get_fakeip returns fake ip mapping by key, whatever key is host or ip.
    fn get_fakeip<K: AsRef<[u8]>>(&mut self, k: K, ipv6: bool) -> Option<Vec<u8>>;
    // put_fakeip puts fake ip mapping into store.
    fn put_fakeip(&mut self, host: &str, ip: IpAddr) -> io::Result<()>;
    // delete_fakeip deletes fake ip mapping from store.
    fn delete_fakeip(&mut self, host: &str, ip: IpAddr) -> io::Result<()>;
    // exists returns if fake ip mapping exists.
    fn exists(&mut self, ip: IpAddr) -> bool;
}

#[enum_dispatch(IFakeIPStore)]
#[derive(Debug)]
pub enum FakeIPStore {
    CacheFile(CacheFileStore),
    Memory(MemoryStore),
}

pub struct Config {
    // IPNet is the ip range that will be returned fake ip.
    pub ipnet: ipnet::Ipv4Net,

    // IPNet6 is the ip6 range that will be returned fake ip.
    pub ipnet6: ipnet::Ipv6Net,

    // Whitelist is a domain list that will be skipped return fake ip.
    pub whitelist: Option<DomainTrie<()>>,

    // Size sets the maximum number of dns records to memory store
    // and dose not work If persist is true.
    pub size: usize,

    // Persist will save the dns record to disk.
    // Size will not work and record will be fully stored.
    pub persist: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ipnet: "198.18.0.0/15".parse().unwrap(),
            ipnet6: "2001:db8::/32".parse().unwrap(),
            whitelist: None,
            size: 65536,
            persist: false,
        }
    }
}

pub struct FakeDns {
    offset: u32,
    total: u32,
    ipnet: ipnet::Ipv4Net,
    ipnet6: ipnet::Ipv6Net,
    whitelist: Option<DomainTrie<()>>,
    store: FakeIPStore,
}

impl FakeDns {
    pub fn new(config: Config) -> Self {
        // generally ipv4 less than ipv6, so we choose the smaller one to set the total of available fake ip.
        let mut total = 1 << (config.ipnet.max_prefix_len() - config.ipnet.prefix_len());
        if total <= 2 {
            panic!("ipnet is too small");
        }
        // reserve 2 for gateway and broadcast
        total = total - 2;

        let store = if config.persist {
            match CacheFileStore::new() {
                Ok(store) => FakeIPStore::CacheFile(store),
                Err(_) => {
                    warn!("Failed to create cachefile, fallback to memory");
                    FakeIPStore::Memory(MemoryStore::new(config.size))
                }
            }
        } else {
            FakeIPStore::Memory(MemoryStore::new(config.size))
        };

        let fakedns = FakeDns {
            offset: 0,
            total: total as u32,
            ipnet: config.ipnet,
            ipnet6: config.ipnet6,
            whitelist: config.whitelist,
            store,
        };

        debug!("create fakedns: {:?}", fakedns);

        fakedns
    }

    pub fn lookup_ip(&mut self, host: &str, ipv6: bool) -> Option<IpAddr> {
        let entity = self.store.get_fakeip(host.as_bytes(), ipv6);
        match entity {
            Some(entity) => {
                let ip = String::from_utf8(entity).unwrap_or_default();
                IpAddr::from_str(ip.as_str()).ok()
            }
            None => {
                let (ip4, ip6) = self.get(host);
                Some(if !ipv6 {
                    IpAddr::V4(ip4)
                } else {
                    IpAddr::V6(ip6)
                })
            }
        }
    }

    pub fn lookup_host(&mut self, ip: IpAddr) -> Option<String> {
        let ipv6 = ip.is_ipv6();
        self.store
            .get_fakeip(ip.to_string().as_bytes(), ipv6)
            .map(|entity| String::from_utf8(entity).unwrap_or_default())
    }

    /// check if ip is already in fakeip mapping
    pub fn exist(&mut self, ip: IpAddr) -> bool {
        self.store.exists(ip)
    }

    /// return if host should be skip
    pub fn should_skipped(&self, host: String) -> bool {
        if let Some(ref whitelist) = self.whitelist {
            return whitelist.search(host).is_some();
        }

        false
    }

    /// check if ip is fake ip
    pub fn is_fake_ip(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => self.ipnet.contains(&ip),
            IpAddr::V6(ip) => self.ipnet6.contains(&ip),
        }
    }

    /// allocates a fake ip4 and ip6 for host.
    fn get(&mut self, host: &str) -> (Ipv4Addr, Ipv6Addr) {
        let current = self.offset;
        loop {
            let ip4 = gen_next_ipv4(&self.ipnet, self.offset);
            let ip6 = gen_next_ipv6(&self.ipnet6, self.offset);
            if !self.store.exists(ip4.into()) && !self.store.exists(ip6.into()) {
                break;
            }

            self.offset = (self.offset + 1) % self.total;
            // if offset is equal to current, it means that all fake ip is used.
            if self.offset == current {
                self.offset = (self.offset + 1) % (self.total);
                let ip4 = gen_next_ipv4(&self.ipnet, self.offset);
                let ip6 = gen_next_ipv6(&self.ipnet6, self.offset);
                _ = self.store.delete_fakeip(host, ip4.into());
                _ = self.store.delete_fakeip(host, ip6.into());
                break;
            }
        }

        let ip4 = gen_next_ipv4(&self.ipnet, self.offset);
        let ip6 = gen_next_ipv6(&self.ipnet6, self.offset);

        _ = self.store.put_fakeip(host, ip4.into());
        _ = self.store.put_fakeip(host, ip6.into());

        trace!("allocated fake ip mapping: {} -> ({}, {})", host, ip4, ip6);

        (ip4, ip6)
    }
}

impl Debug for FakeDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeDns")
            .field("ipnet", &self.ipnet)
            .field("ipnet6", &self.ipnet6)
            .field("store", &self.store)
            .finish()
    }
}

fn gen_next_ipv4(ipnet: &ipnet::Ipv4Net, offset: u32) -> Ipv4Addr {
    let mut ip: u32 = ipnet.network().into();
    // skip gateway and broadcast
    ip += 2;
    ip += offset;
    ip.into()
}

fn gen_next_ipv6(ipnet: &ipnet::Ipv6Net, offset: u32) -> Ipv6Addr {
    let mut ip: u128 = ipnet.network().into();
    // skip gateway and broadcast
    ip += 2;
    ip += offset as u128;
    ip.into()
}

#[cfg(test)]
mod tests {

    use ipnet::Ipv4Net;
    use rand::Rng;

    use crate::cachefile::CacheFile;

    use super::*;

    #[test]
    fn test_gen_next_ip() {
        let ipnet: ipnet::Ipv4Net = "198.18.0.0/15".parse().unwrap();
        let ipnet6: ipnet::Ipv6Net = "2001:db8::/32".parse().unwrap();
        assert_eq!(
            gen_next_ipv4(&ipnet, 0),
            "198.18.0.2".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(
            gen_next_ipv6(&ipnet6, 0),
            "2001:db8::2".parse::<Ipv6Addr>().unwrap()
        );
    }

    fn create_fakedns() -> FakeDns {
        let mut whitelist = DomainTrie::new();
        _ = whitelist.insert(String::from("example.com"), ());

        let mut config = Config::default();
        config.whitelist = Some(whitelist);
        config.ipnet = "198.18.0.0/29".parse().unwrap();
        config.ipnet6 = "2001:db8::/125".parse().unwrap();
        config.size = 8;

        FakeDns::new(config)
    }

    fn create_fakedns_with_minsize() -> FakeDns {
        let mut whitelist = DomainTrie::new();
        _ = whitelist.insert(String::from("example.com"), ());

        let mut config = Config::default();
        config.whitelist = Some(whitelist);
        config.ipnet = "198.18.0.0/15".parse().unwrap();
        config.ipnet6 = "2001:db8::/32".parse().unwrap();
        config.size = 2;

        FakeDns::new(config)
    }

    fn create_fakedns_with_persist() -> FakeDns {
        // prepare cache dir
        CacheFile::with_cache_dir(std::env::temp_dir().join("swiftlink").join("cachedb"))
            .expect("Failed to create cachefile");

        let mut whitelist = DomainTrie::new();
        _ = whitelist.insert(String::from("example.com"), ());

        let mut config = Config::default();
        config.whitelist = Some(whitelist);
        config.ipnet = "198.18.0.0/15".parse().unwrap();
        config.ipnet6 = "2001:db8::/32".parse().unwrap();
        config.size = 65535;
        config.persist = true;

        FakeDns::new(config)
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_fakedns_basic() {
        let mut fakedns = create_fakedns();

        assert_eq!(fakedns.should_skipped(String::from("example.com")), true);
        assert_eq!(fakedns.should_skipped(String::from("foo.bar")), false);
        assert_eq!(
            fakedns.is_fake_ip(Ipv4Addr::new(198, 18, 0, 2).into()),
            true
        );
        assert_eq!(
            fakedns.is_fake_ip(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2).into()),
            true
        );

        let foobar = fakedns.lookup_ip("foo.bar", false).unwrap();
        let barfoo = fakedns.lookup_ip("bar.foo", false).unwrap();

        let foobar6 = fakedns.lookup_ip("foo.bar", true).unwrap();
        let barfoo6 = fakedns.lookup_ip("bar.foo", true).unwrap();

        assert_eq!(foobar, Ipv4Addr::new(198, 18, 0, 2));
        assert_eq!(barfoo, Ipv4Addr::new(198, 18, 0, 3));
        assert_eq!(foobar6, "2001:db8::2".parse::<Ipv6Addr>().unwrap());
        assert_eq!(barfoo6, "2001:db8::3".parse::<Ipv6Addr>().unwrap());

        assert_eq!(fakedns.lookup_host(foobar), fakedns.lookup_host(foobar6));

        assert_eq!(fakedns.exist(foobar), true);
        assert_eq!(fakedns.exist(foobar6), true);
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_fakedns_cycle() {
        let mut fakedns = create_fakedns();
        let hosts = [
            "test1.example.com",
            "test2.example.com",
            "test3.example.com",
            "test4.example.com",
            "test5.example.com",
            "test6.example.com",
        ];

        hosts.iter().for_each(|host| {
            fakedns.lookup_ip(&host, false).unwrap();
        });

        let first = fakedns.lookup_ip("test1.example.com", false).unwrap();
        let cycled = fakedns.lookup_ip("test7.example.com", false).unwrap();
        assert_eq!(first, cycled);
    }

    #[test]
    fn test_fakedns_max_cache_size() {
        let mut fakedns = create_fakedns_with_minsize();

        let first = fakedns.lookup_ip("test1.example.com", false).unwrap();
        assert_eq!(fakedns.lookup_host(first), Some("test1.example.com".into()));

        fakedns.lookup_ip("test2.example.com", false);
        fakedns.lookup_ip("test3.example.com", false);

        let next = fakedns.lookup_ip("test1.example.com", false).unwrap();
        assert_ne!(first, next);

        assert!(!fakedns.exist(first));
    }

    #[test]
    fn test_fakedns_double_mapping() {
        let mut fakedns = create_fakedns_with_minsize();
        let foo_ip = fakedns.lookup_ip("foo.example.com", false).unwrap();
        let bar_ip = fakedns.lookup_ip("bar.example.com", false).unwrap();
        fakedns.lookup_ip("foo.example.com", false);
        let baz_ip = fakedns.lookup_ip("baz.example.com", false).unwrap();

        assert_eq!(fakedns.lookup_host(foo_ip), Some("foo.example.com".into()));
        assert_eq!(fakedns.lookup_host(bar_ip), None);
        assert_eq!(fakedns.lookup_host(baz_ip), Some("baz.example.com".into()));

        let bar_ip1 = fakedns.lookup_ip("bar.example.com", false).unwrap();
        assert_ne!(bar_ip, bar_ip1);
    }

    #[test]
    fn test_fakedns_persist() {
        let mut fakedns = create_fakedns_with_persist();
        let ipnet = "198.18.0.0/16".parse::<Ipv4Net>().unwrap();
        for i in 0..65534 {
            let host = format!("test{}.example.com", i);
            let ip = fakedns.lookup_ip(&host, false).unwrap();
            assert_eq!(fakedns.lookup_host(ip), Some(host));
        }

        let mut rnd = rand::thread_rng();
        for _ in 0..1000 {
            let num = rnd.gen_range(0..65534);
            let host = format!("test{}.example.com", num);
            let ip = fakedns.lookup_ip(&host, false).expect("ip should exists");

            if let IpAddr::V4(v4) = ip {
                assert_eq!(ipnet.contains(&v4), true);
            } else {
                panic!("ip should be ipv4");
            }
        }
    }
}
