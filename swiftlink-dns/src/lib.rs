pub use dns_conf::Config;
pub use dns_handle::{DnsHandler, DnsHandlerBuilder, DnsRequestHandle};
pub use dns_server::DnsServerHandler;
pub use hickory_server::ServerFuture;

mod dns;
mod dns_client;
mod dns_conf;
mod dns_error;
mod dns_handle;
mod dns_server;
mod dns_url;
mod preset_ns;
mod proxy;
mod rustls;
