use anyhow::{bail, Context};
use byte_unit::Byte;
use cfg_if::cfg_if;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use swiftlink_dns::DnsConfig;
use swiftlink_infra::{file_mode::FileMode, log};

#[derive(Deserialize, Default)]
pub struct Config {
    log_level: Option<String>,
    log_file: Option<PathBuf>,
    log_file_mode: Option<FileMode>,
    log_filter: Option<String>,
    log_max_file_size: Option<Byte>,
    log_files: Option<u64>,

    dns: Option<Arc<DnsConfig>>,

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
        log::info!("Using configuration file: {:?}", self.source_conf_path);
    }

    #[inline]
    pub fn dns(&self) -> Arc<DnsConfig> {
        self.dns.as_ref().map(|dns| dns.clone()).unwrap_or_default()
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
}
