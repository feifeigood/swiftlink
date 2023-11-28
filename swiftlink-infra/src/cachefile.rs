use std::{fmt::Debug, fs, io, net::IpAddr, path::Path, sync::Arc};

use once_cell::sync::OnceCell;
use rocksdb::{BoundColumnFamily, MultiThreaded, Transaction, TransactionDB};

#[rustfmt::skip]
mod cf {
    pub const FAKEIP:  &str = "fakeip";
    pub const FAKEIP6: &str = "fakeip6";
}

static INSTANCE: OnceCell<CacheFile> = OnceCell::new();

/// Represent the cache file based on rocksdb
pub struct CacheFile {
    db: TransactionDB<MultiThreaded>,
}

impl CacheFile {
    pub fn global() -> &'static CacheFile {
        INSTANCE
            .get()
            .expect("Cachefile should be initialized first")
    }

    pub fn with_cache_dir<P: AsRef<Path>>(cache_dir: P) -> io::Result<&'static CacheFile> {
        INSTANCE.get_or_try_init(|| -> Result<CacheFile, io::Error> {
            // create cache dir if not exists
            if !cache_dir.as_ref().exists() {
                fs::create_dir_all(&cache_dir)?;
            }

            let mut opts = rocksdb::Options::default();
            opts.set_error_if_exists(false);
            opts.create_if_missing(true);
            opts.create_missing_column_families(true);

            // limit the size of log file 5MB, and keep 10 log files
            opts.set_max_log_file_size(5 * 1024 * 1024);
            opts.set_keep_log_file_num(10);

            let txn_db_opts = rocksdb::TransactionDBOptions::default();

            let cfs = rocksdb::DB::list_cf(&opts, &cache_dir).unwrap_or(vec![]);

            let db =
                TransactionDB::open_cf(&opts, &txn_db_opts, &cache_dir, cfs).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("Failed to open Cachefile(rocksdb): {}", err),
                    )
                })?;

            // prepare column families
            _ = db.create_cf(cf::FAKEIP, &opts);
            _ = db.create_cf(cf::FAKEIP6, &opts);

            Ok(CacheFile { db })
        })
    }

    fn inner_get_cf_handle<N: AsRef<str>>(&self, name: N) -> Option<Arc<BoundColumnFamily<'_>>> {
        self.db.cf_handle(name.as_ref())
    }

    pub fn get_fakeip<K: AsRef<[u8]>>(&self, key: K, ipv6: bool) -> Option<Vec<u8>> {
        let cf_name = if ipv6 { cf::FAKEIP6 } else { cf::FAKEIP };
        if let Some(cf) = self.inner_get_cf_handle(cf_name) {
            if let Ok(res) = self.db.get_cf(&cf, key) {
                if let Some(data) = res {
                    return Some(data.to_vec());
                }
            }
        }

        None
    }

    pub fn put_fakeip(&self, host: String, ip: IpAddr) -> io::Result<()> {
        let ipv6 = ip.is_ipv6();
        let cf_name = if ipv6 { cf::FAKEIP6 } else { cf::FAKEIP };
        if let Some(cf) = self.inner_get_cf_handle(cf_name) {
            let txn_db = self.db.transaction();
            let put_kvpair = |k1: &[u8],
                              k2: &[u8],
                              txn: &Transaction<TransactionDB<MultiThreaded>>|
             -> Result<(), rocksdb::Error> {
                txn.put_cf(&cf, k1, k2)?;
                txn.put_cf(&cf, k2, k1)?;
                Ok(())
            };

            let k1 = host.clone();
            let k2 = match ip {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => ip.to_string(),
            };

            put_kvpair(&k1.as_bytes(), &k2.as_bytes(), &txn_db).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to put fakeip pair {:?} <-> {:?}: {}", host, ip, err),
                )
            })?;

            txn_db.commit().map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to commit transaction for saving fakeip pair: {}",
                        err
                    ),
                )
            })?;

            return Ok(());
        }

        Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to get fakeip column family",
        ))
    }

    pub fn delete_fakeip(&self, host: String, ip: IpAddr) -> io::Result<()> {
        let ipv6 = ip.is_ipv6();
        let cf_name = if ipv6 { cf::FAKEIP6 } else { cf::FAKEIP };
        if let Some(cf) = self.inner_get_cf_handle(cf_name) {
            let txn_db = self.db.transaction();
            let delete_kvpair = |k1: &[u8],
                                 k2: &[u8],
                                 txn: &Transaction<TransactionDB<MultiThreaded>>|
             -> Result<(), rocksdb::Error> {
                txn.delete_cf(&cf, k1)?;
                txn.delete_cf(&cf, k2)?;
                Ok(())
            };

            let k1 = host.clone();
            let k2 = match ip {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => ip.to_string(),
            };

            delete_kvpair(&k1.as_bytes(), &k2.as_bytes(), &txn_db).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to delete fakeip pair {:?} <-> {:?}: {}",
                        host, ip, err
                    ),
                )
            })?;

            txn_db.commit().map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to commit transaction for deleting fakeip pair: {}",
                        err
                    ),
                )
            })?;
        }

        Ok(())
    }
}

impl Debug for CacheFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CacheFile")
    }
}

#[cfg(test)]
mod tests {

    use std::path::PathBuf;

    use super::*;

    fn temp_cache_dir() -> PathBuf {
        std::env::temp_dir().join("swiftlink").join("cachedb")
    }

    #[test]
    fn test_cf_fakeip() {
        let cache_dir = temp_cache_dir();
        let cachefile = CacheFile::with_cache_dir(&cache_dir).expect("Failed to create cachefile");

        let ip4 = "240.0.0.1".parse::<IpAddr>().unwrap();
        let ip6 = "fddd:c5b4:ff5f:f4f0::1".parse::<IpAddr>().unwrap();
        let host = "fakednstest.swiftlink.org";

        cachefile.put_fakeip(host.into(), ip4).unwrap();
        cachefile.put_fakeip(host.into(), ip6).unwrap();

        assert_eq!(
            cachefile.get_fakeip(ip4.to_string(), false).unwrap(),
            host.as_bytes()
        );
        assert_eq!(
            cachefile.get_fakeip(ip6.to_string(), true).unwrap(),
            host.as_bytes()
        );

        _ = cachefile.delete_fakeip(host.into(), ip4);
        assert!(cachefile.get_fakeip(host, false).is_none());
        assert!(cachefile.get_fakeip(host, true).is_some());
    }
}
