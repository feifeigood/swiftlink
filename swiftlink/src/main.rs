#![allow(dead_code)]

use std::{env, path::PathBuf};

use cli::*;

use swiftlink_infra::log::{self, info};

use crate::app::App;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod app;
mod cli;
mod config;
mod context;
mod dispatcher;
mod error;
mod inbound;
mod outbound;
mod route;
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
            Commands::Run { conf, home_dir, .. } => {
                // TODO: pid file

                let home_dir = home_dir
                    .unwrap_or(dirs::home_dir().expect("Failed to get homedir"))
                    .join(".config")
                    .join("swiftlink");

                run_server(conf.unwrap_or(home_dir.join("swiftlink.toml")), home_dir);
            }
        }
    }
}

fn run_server(conf: PathBuf, home_dir: PathBuf) {
    App::new(conf, home_dir)
        .expect("Failed to create swiftlink app")
        .bootstrap();

    info!("{} {} shutdown", NAME, version());
}
