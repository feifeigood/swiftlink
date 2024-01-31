use anyhow::{bail, Context};
use byte_unit::Byte;
use cfg_if::cfg_if;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use swiftlink_dns::DnsConfig;
use swiftlink_infra::{auth::Authenticator, file_mode::FileMode, log::info};

#[derive(Deserialize, Default)]
pub struct Config {
    port: u16,
    socks_port: u16,
    interface_name: Option<String>,
    ipv6_first: bool,

    log_level: Option<String>,
    log_file: Option<PathBuf>,
    log_file_mode: Option<FileMode>,
    log_filter: Option<String>,
    log_max_file_size: Option<Byte>,
    log_files: Option<u64>,

    #[serde(default, deserialize_with = "deserialize::from_str_to_auth")]
    authentication: Option<Authenticator>,

    #[serde(default, deserialize_with = "deserialize::from_str_to_rule")]
    rules: Option<Vec<Rule>>,

    dns: DnsConfig,

    proxies: Option<Vec<Proxy>>,

    // Hold source path for config reload
    #[serde(skip)]
    source_conf_path: PathBuf,
}

impl Config {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Configuration file not found")
        }

        let contents = fs::read_to_string(path)?;
        let mut cfg = Self::load(&contents)?;
        cfg.source_conf_path = path.to_owned();

        Ok(cfg)
    }

    fn load(contents: &str) -> anyhow::Result<Self> {
        toml::de::from_str(contents).with_context(|| "Failed to load config".to_string())
    }

    pub fn summary(&self) {
        // TODO: print config summary
        info!("Using configuration file: {:?}", self.source_conf_path);
    }

    #[inline]
    pub fn port(&self) -> u16 {
        self.port
    }

    #[inline]
    pub fn socks_port(&self) -> u16 {
        self.socks_port
    }

    #[inline]
    pub fn authentication(&self) -> Option<&Authenticator> {
        self.authentication.as_ref()
    }

    #[inline]
    pub fn log_enabled(&self) -> bool {
        self.log_max_files() > 0
    }

    pub fn log_level(&self) -> tracing::Level {
        use tracing::Level;
        match self.log_level.as_deref().unwrap_or("info") {
            "tarce" => Level::TRACE,
            "debug" => Level::DEBUG,
            "info" | "notice" => Level::INFO,
            "warn" => Level::WARN,
            "error" | "fatal" => Level::ERROR,
            _ => Level::ERROR,
        }
    }

    pub fn log_file(&self) -> PathBuf {
        match self.log_file.as_ref() {
            Some(f) => f.to_owned(),
            None => {
                cfg_if! {
                    if #[cfg(target_os = "windows")] {
                        let mut path = std::env::temp_dir();
                        path.push("swiftlink");
                        path.push("swiftlink.log");
                        path
                    } else {
                        PathBuf::from(r"/var/log/swiftlink/swiftlink.log")
                    }
                }
            }
        }
    }

    #[inline]
    pub fn log_file_mode(&self) -> u32 {
        self.log_file_mode.map(|m| *m).unwrap_or(0o640)
    }

    #[inline]
    pub fn log_filter(&self) -> Option<&str> {
        self.log_filter.as_deref()
    }

    #[inline]
    pub fn log_size(&self) -> u64 {
        use byte_unit::n_kb_bytes;
        self.log_max_file_size
            .unwrap_or(Byte::from_bytes(n_kb_bytes(128)))
            .get_bytes()
    }

    #[inline]
    pub fn log_max_files(&self) -> u64 {
        self.log_files.unwrap_or(2)
    }

    #[inline]
    pub fn dns(&self) -> Arc<DnsConfig> {
        Arc::new(self.dns.clone())
    }

    #[inline]
    pub fn interface_name(&self) -> Option<&str> {
        self.interface_name.as_deref()
    }

    #[inline]
    pub fn proxies(&self) -> Option<&Vec<Proxy>> {
        self.proxies.as_ref()
    }
}

#[derive(Deserialize, Default)]
pub struct Proxy {
    pub name: String,
    #[serde(rename = "type")]
    pub protocol: String,

    // common field
    pub server: Option<String>,
    pub port: Option<u16>,

    // TODO: shadowsocks

    // shadowsocks, trojan
    pub password: Option<String>,

    // trojan
    pub sni: Option<String>,

    // tls
    #[serde(default)]
    pub skip_cert_verify: bool,
}

#[derive(Debug)]
pub struct Rule {
    pub tp: String,
    pub payload: String,
    pub target: String,
    pub params: Vec<String>,
}

impl FromStr for Rule {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts = s.split(',').map(str::to_string).collect::<Vec<_>>();

        let (tp, payload, target, params) = match parts.len() {
            2 => (parts[0].clone(), "".into(), parts[1].clone(), vec![]),
            3 => (parts[0].clone(), parts[1].clone(), parts[2].clone(), vec![]),
            n if n >= 4 => (parts[0].clone(), parts[1].clone(), parts[2].clone(), parts[2..n].into()),
            _ => return Err(format!("invalid rule: {}", s)),
        };

        Ok(Self {
            tp,
            payload,
            target,
            params,
        })
    }
}

mod deserialize {
    use serde::{de, Deserialize, Deserializer};
    use swiftlink_infra::auth::AuthUser;

    use super::*;

    pub(super) fn from_str_to_rule<'de, D>(deserializer: D) -> Result<Option<Vec<Rule>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Vec<String> = Vec::deserialize(deserializer)?;

        match raw
            .iter()
            .map(|x| x.parse::<Rule>())
            .collect::<Result<Vec<Rule>, String>>()
        {
            Ok(x) => Ok(Some(x)),
            Err(s) => Err(de::Error::custom(format!("deserialize rule error: {}", s))),
        }
    }

    pub(super) fn from_str_to_auth<'de, D>(deserializer: D) -> Result<Option<Authenticator>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Vec<String> = Vec::deserialize(deserializer)?;

        match raw
            .iter()
            .map(|x| x.parse::<AuthUser>())
            .collect::<Result<Vec<AuthUser>, String>>()
        {
            Ok(users) => Ok(Some(Authenticator::new(users))),
            Err(s) => Err(de::Error::custom(format!("deserialize authentication error: {}", s))),
        }
    }
}
