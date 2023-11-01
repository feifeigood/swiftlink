#![allow(dead_code)]

use std::{env, path::PathBuf};

use cli::*;
use swiftlink_infra::log;

use crate::app::App;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod app;
mod cli;
mod conf;
mod error;
mod rt;

/// The app name
const NAME: &str = "swiftlink";

/// Returns a version as specified in Cargo.toml
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn main() {
    Cli::parse().run();
}

impl Cli {
    #[inline]
    pub fn run(self) {
        let _guard = log::default();

        match self.command {
            Commands::Run { conf, .. } => {
                // TODO: pid file
                run_server(conf.unwrap_or(env::current_dir().unwrap().join("swiftlink.toml")));
            }
        }
    }
}

fn run_server(conf: PathBuf) {
    App::new(conf)
        .expect("Failed to create swiftlink app")
        .bootstrap();

    log::info!("{} {} shutdown", NAME, version());
}
