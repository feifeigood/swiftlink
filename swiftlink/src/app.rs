use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use futures_util::future::join_all;
use tokio::{runtime::Runtime, sync::RwLock};

use crate::{conf::Config, error::Error, rt};
use swiftlink_dns::{DnsHandler, DnsHandlerBuilder, DnsServerHandler, ServerFuture};
use swiftlink_infra::{bind_to, log, tcp, udp, IListener, Listener};

pub struct App {
    cfg: RwLock<Arc<Config>>,
    dns_handler: RwLock<Arc<DnsHandler>>,
    listener_map: Arc<RwLock<HashMap<Listener, ServerTasks>>>,
    runtime: Runtime,
    guard: AppGuard,
}

impl App {
    pub fn new(conf: PathBuf) -> anyhow::Result<Self> {
        let cfg = Arc::new(
            Config::load_from_file(&conf)
                .with_context(|| format!("Error while loading config file: {:?}", conf))?,
        );

        let guard = {
            let log_guard = if cfg.log_enabled() {
                Some(log::init_global_default(
                    cfg.log_file(),
                    cfg.log_level(),
                    cfg.log_filter(),
                    cfg.log_size(),
                    cfg.log_max_files(),
                    cfg.log_file_mode().into(),
                ))
            } else {
                None
            };

            AppGuard { log_guard }
        };

        cfg.summary();

        let runtime = rt::build();

        let dns_server_handler = create_dns_server_handler(cfg.clone(), &runtime);

        Ok(Self {
            cfg: RwLock::new(cfg),
            dns_handler: RwLock::new(Arc::new(dns_server_handler)),
            listener_map: Default::default(),
            runtime,
            guard,
        })
    }

    pub fn bootstrap(self) {
        // Raise `nofile` limit on Linux/MacOS
        fdlimit::raise_fd_limit();

        self.runtime.block_on(self.register_listeners());

        log::info!("awaiting connections...");

        log::info!("server starting up");

        let listeners = self.listener_map.clone();

        let shutdown_timeout = Duration::from_secs(5);
        self.runtime.block_on(async move {
            let _ = swiftlink_infra::signal::shutdown().await;

            let mut listeners = listeners.write().await;
            let shutdown_tasks = listeners.iter_mut().map(|(_, server)| async move {
                match server.shutdown(shutdown_timeout).await {
                    Ok(_) => (),
                    Err(err) => log::warn!("{:?}", err),
                }
            });

            join_all(shutdown_tasks).await;
        });

        self.runtime.shutdown_timeout(shutdown_timeout);
    }

    async fn register_listeners(&self) {
        let cfg = self.cfg.read().await;

        let listener_map = self.listener_map.clone();

        // create local dns server
        {
            let dns_conf = cfg.dns();
            let listeners = dns_conf.binds();
            for listener in listeners {
                match create_dns_server(self, listener).await {
                    Ok(server) => {
                        if let Some(mut prev_server) =
                            listener_map.write().await.insert(listener.clone(), server)
                        {
                            tokio::spawn(async move {
                                let _ = prev_server.shutdown(Duration::from_secs(5)).await;
                            });
                        }
                    }
                    Err(err) => {
                        log::error!("{}", err);
                    }
                }
            }
        }

        // TODO: create socks server
        // TODO: create http server
        // TODO: create tun listener
        // TODO: create admin api server
    }
}

async fn create_dns_server(app: &App, listener: &Listener) -> Result<ServerTasks, Error> {
    let handler = app.dns_handler.read().await.clone();

    let server_handler = DnsServerHandler::new(handler, listener.server_opts().clone());

    let cfg = app.cfg.read().await.dns();

    let tcp_idle_time = cfg.tcp_idle_time();

    let server = match listener {
        Listener::Udp(listener) => {
            let udp_socket = bind_to(udp, listener.sock_addr(), listener.device(), "UDP");
            let mut server = ServerFuture::new(server_handler);
            server.register_socket(udp_socket);
            ServerTasks::Dns(server)
        }
        Listener::Tcp(listener) => {
            let tcp_listener = bind_to(tcp, listener.sock_addr(), listener.device(), "TCP");
            let mut server = ServerFuture::new(server_handler);
            server.register_listener(tcp_listener, Duration::from_secs(tcp_idle_time));

            ServerTasks::Dns(server)
        }
    };

    Ok(server)
}

fn create_dns_server_handler(cfg: Arc<Config>, runtime: &Runtime) -> DnsHandler {
    let _guard = runtime.enter();

    let builder = DnsHandlerBuilder::new();

    // TODO: add handle

    builder.build(Arc::new(cfg.dns()))
}

struct AppGuard {
    log_guard: Option<tracing::dispatcher::DefaultGuard>,
}

enum ServerTasks {
    Dns(ServerFuture<DnsServerHandler>),
}

impl ServerTasks {
    async fn shutdown(&mut self, _shutdown_timeout: Duration) -> Result<(), anyhow::Error> {
        match self {
            ServerTasks::Dns(s) => {
                let _ = s.shutdown_gracefully().await;
                Ok(())
            }
        }
    }
}
