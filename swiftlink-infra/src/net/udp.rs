use std::{io, net::SocketAddr};

use tokio::net::UdpSocket;

use crate::net::{
    sys::{bind_udp_socket_impl, create_udp_socket_impl},
    ConnectOpts,
};

/// Creates a UDP socket
pub async fn connect_udp_socket_with_opts(server_addr: SocketAddr, conn_opts: &ConnectOpts) -> io::Result<UdpSocket> {
    let socket = create_udp_socket_impl(From::from(&server_addr), conn_opts).await?;
    socket.connect(server_addr).await?;

    Ok(socket)
}

/// Bind a UDP socket with specified address
pub async fn bind_udp_socket_with_opts(bind_addr: SocketAddr, conn_opts: &ConnectOpts) -> io::Result<UdpSocket> {
    bind_udp_socket_impl(&bind_addr, conn_opts).await
}
