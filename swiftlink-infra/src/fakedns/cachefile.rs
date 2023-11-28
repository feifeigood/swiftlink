use std::{io, net::IpAddr};

use crate::cachefile::CacheFile;

use super::IFakeIPStore;

#[derive(Debug)]
pub struct CacheFileStore {
    store: &'static CacheFile,
}

impl CacheFileStore {
    pub fn new() -> Self {
        Self {
            store: CacheFile::global(),
        }
    }
}

impl IFakeIPStore for CacheFileStore {
    fn get_fakeip<K: AsRef<[u8]>>(&mut self, k: K, ipv6: bool) -> Option<Vec<u8>> {
        self.store.get_fakeip(k, ipv6)
    }

    fn put_fakeip(&mut self, host: &str, ip: IpAddr) -> io::Result<()> {
        self.store.put_fakeip(host.into(), ip)
    }

    fn delete_fakeip(&mut self, host: &str, ip: IpAddr) -> io::Result<()> {
        self.store.delete_fakeip(host.into(), ip)
    }

    fn exists(&mut self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => self.store.get_fakeip(ip.to_string(), false).is_some(),
            IpAddr::V6(ip) => self.store.get_fakeip(ip.to_string(), true).is_some(),
        }
    }
}
