use std::{io, net::SocketAddr};

use tokio::net::TcpStream;

use crate::net::{sys::connect_tcp_with_opts_impl, ConnectOpts};

/// Connects to address
pub async fn connect_tcp_with_opts(addr: SocketAddr, opts: &ConnectOpts) -> io::Result<TcpStream> {
    connect_tcp_with_opts_impl(addr, opts).await
}
