use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use futures_util::future::join_all;
use tokio::{runtime::Runtime, sync::RwLock};

use swiftlink_dns::build_dns_resolver;
use swiftlink_dns::{ServerHandle, ServerHandleBuilder};
use swiftlink_infra::{
    bind_to,
    cachefile::CacheFile,
    log::{self, *},
    net::ConnectOpts,
    udp, Listener,
};

use crate::{config::Config, context::AppContext, rt};

pub struct App {
    config: Arc<Config>,
    context: AppContext,
    listener_map: Arc<RwLock<HashMap<Listener, ServerTasks>>>,
    runtime: Runtime,
    guard: AppGuard,
}

impl App {
    pub fn new(config_path: PathBuf, home_dir: PathBuf) -> anyhow::Result<Self> {
        let config = Arc::new(
            Config::load_from_file(&config_path)
                .with_context(|| format!("Error while loading config file: {:?}", config_path))?,
        );

        let guard = {
            let log_guard = if config.log_enabled() {
                Some(log::init_global_default(
                    config.log_file(),
                    config.log_level(),
                    config.log_filter(),
                    config.log_size(),
                    config.log_max_files(),
                    config.log_file_mode().into(),
                ))
            } else {
                None
            };

            AppGuard { log_guard }
        };

        config.summary();

        // initialize cachefile
        if let Err(err) = CacheFile::with_cache_dir(home_dir.join("cachedb")) {
            warn!("Failed to initialize cachefile: {:?}", err);
        }

        let runtime = rt::build();
        let listener_map: Arc<RwLock<HashMap<Listener, ServerTasks>>> = Default::default();
        let mut context = AppContext::default();

        let mut connect_opts: ConnectOpts = Default::default();
        connect_opts.bind_interface = config.interface_name().map(|s| s.to_owned());

        {
            let dns = config.dns();
            if dns.enabled() {
                if dns.fakeip() {
                    use swiftlink_infra::fakedns::{Config, FakeDns};

                    let mut conf = Config::default();
                    conf.persist = dns.fakeip_persist();

                    // using memory cache
                    if !dns.fakeip_persist() {
                        conf.size = dns.fakeip_size().unwrap_or(2048);
                    }

                    let (ipv4_range, ipv6_range) = dns.fakeip_range();
                    if let Some(ipv4_range) = ipv4_range {
                        conf.ipnet = ipv4_range;
                    }
                    if let Some(ipv6_range) = ipv6_range {
                        conf.ipnet6 = ipv6_range;
                    }

                    // TODO: fakeip filter
                    let fakedns = Arc::new(Mutex::new(FakeDns::new(conf)));
                    context.set_fakedns(fakedns);
                }
            }

            runtime.block_on(async {
                let dns_resolver = build_dns_resolver(&dns, &connect_opts).await;

                // register local dns server
                let listener = dns.listen();
                let mut builder = ServerHandleBuilder::new(dns.clone(), dns_resolver.into());
                if let Some(fakedns) = context.fakedns() {
                    builder = builder.with_fakedns(fakedns);
                }
                let server_handle = builder.build();

                let udp_socket = bind_to(udp, listener.sock_addr(), listener.device(), "UDP");
                let mut server = swiftlink_dns::ServerFuture::new(server_handle);
                server.register_socket(udp_socket);

                if let Some(mut prev_server) = listener_map
                    .write()
                    .await
                    .insert(listener.clone(), ServerTasks::Dns(server))
                {
                    tokio::spawn(async move {
                        let _ = prev_server.shutdown(Duration::from_secs(5)).await;
                    });
                }
            });
        }

        Ok(Self {
            config,
            context,
            listener_map,
            runtime,
            guard,
        })
    }

    pub fn bootstrap(self) {
        // Raise `nofile` limit on Linux/MacOS
        fdlimit::raise_fd_limit();

        info!("server starting up");

        let listeners = self.listener_map.clone();

        let shutdown_timeout = Duration::from_secs(5);
        self.runtime.block_on(async move {
            let _ = swiftlink_infra::signal::shutdown().await;

            let mut listeners = listeners.write().await;
            let shutdown_tasks = listeners.iter_mut().map(|(_, server)| async move {
                match server.shutdown(shutdown_timeout).await {
                    Ok(_) => (),
                    Err(err) => warn!("{:?}", err),
                }
            });

            join_all(shutdown_tasks).await;
        });

        self.runtime.shutdown_timeout(shutdown_timeout);
    }
}

struct AppGuard {
    log_guard: Option<tracing::dispatcher::DefaultGuard>,
}

enum ServerTasks {
    Dns(swiftlink_dns::ServerFuture<ServerHandle>),
    // Inbound(inbound::InboundServerHandle),
}

impl ServerTasks {
    async fn shutdown(&mut self, _shutdown_timeout: Duration) -> Result<(), anyhow::Error> {
        match self {
            ServerTasks::Dns(s) => {
                let _ = s.shutdown_gracefully().await;
                Ok(())
            } // _ => Ok(()),
        }
    }
}
