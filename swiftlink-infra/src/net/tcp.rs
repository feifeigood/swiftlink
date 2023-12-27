use std::{io, net::SocketAddr};

use tokio::net::TcpStream;

use crate::net::{sys::create_tcp_stream_impl, ConnectOpts};

/// Dials a TCP stream with the given options
pub async fn crate_tcp_stream_with_opts(server_addr: SocketAddr, conn_opts: &ConnectOpts) -> io::Result<TcpStream> {
    create_tcp_stream_impl(server_addr, conn_opts).await
}
