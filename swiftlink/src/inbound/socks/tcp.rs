use std::{
    io::{self},
    net::SocketAddr,
    sync::Arc,
};
use tokio::{
    io::BufReader,
    net::{self, TcpStream},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

use swiftlink_infra::{auth::Authenticator, log::*};
use swiftlink_transport::socks4;

use crate::context::InboundConnection;

pub async fn serve_tcp(
    listener: net::TcpListener,
    shutdown: CancellationToken,
    authenticator: Arc<Option<Authenticator>>,
    tcp_in: mpsc::Sender<InboundConnection>,
) -> io::Result<()> {
    loop {
        let (tcp_stream, src_addr) = tokio::select! {
            tcp_stream = listener.accept() => match tcp_stream {
                Ok((t,s))=> (t,s),
                Err(e) => {
                    debug!("error receiving TCP tcp_stream error: {}", e);
                    continue;
                }
            },
            _= shutdown.cancelled() => {
                // A graceful shutdown is initiated. Break out of the loop.
                break;
            }
        };

        let authenticator = authenticator.clone();
        let tcp_in = tcp_in.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_tcp_client(tcp_stream, src_addr, authenticator, tcp_in).await {
                error!("socks tcp client handler error: {}", e);
            }
        });
    }

    Ok(())
}

async fn handle_tcp_client(
    tcp_stream: TcpStream,
    src_addr: SocketAddr,
    authenticator: Arc<Option<Authenticator>>,
    tcp_in: mpsc::Sender<InboundConnection>,
) -> io::Result<()> {
    let mut version_buffer = [0u8; 1];
    let n = tcp_stream.peek(&mut version_buffer).await?;
    if n == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }

    match version_buffer[0] {
        0x04 => socks4_impl::handle_client(tcp_stream, src_addr, authenticator, tcp_in).await,
        0x05 => socks5_impl::handle_client(tcp_stream, src_addr, authenticator, tcp_in).await,
        version => {
            error!("unsupported socks version: {:x}", version);
            Err(io::Error::new(io::ErrorKind::Other, "unsupported socks version"))
        }
    }
}

mod socks4_impl {

    use super::*;

    use crate::context::Metadata;

    use swiftlink_transport::{
        socks4::{Command, HandshakeRequest, HandshakeResponse, ResultCode},
        socks5::Address,
    };

    pub async fn handle_client(
        stream: TcpStream,
        peer_addr: SocketAddr,
        authenticator: Arc<Option<Authenticator>>,
        tcp_in: mpsc::Sender<InboundConnection>,
    ) -> io::Result<()> {
        // 1. Handshake

        // NOTE: Wraps it with BufReader for reading NULL terminated information in HandshakeRequest
        let mut s = BufReader::new(stream);
        let handshake_req = match HandshakeRequest::read_from(&mut s).await {
            Ok(r) => r,
            Err(socks4::Error::IoError(ref err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                trace!("socks4 handshake early eof. peer: {}", peer_addr);
                return Ok(());
            }
            Err(err) => {
                error!("socks4 handshake error: {}", err);
                return Err(err.into());
            }
        };

        trace!("socks4 {:?} peer: {}", handshake_req, peer_addr);

        // 2. Authentication
        if let Some(authenticator) = authenticator.as_ref() {
            if !authenticator.verify(String::from_utf8_lossy(&handshake_req.user_id).trim(), "") {
                let handshake_rsp = HandshakeResponse::new(ResultCode::RequestRejectedDifferentUserId);
                handshake_rsp.write_to(&mut s).await?;
                return Ok(());
            }
        }

        match handshake_req.cd {
            Command::Connect => {
                debug!("CONNECT {}", handshake_req.dst);

                let handshake_rsp = HandshakeResponse::new(ResultCode::RequestGranted);
                handshake_rsp.write_to(&mut s).await?;

                let mut meta = Metadata::default();
                meta.inbound_tag = "SOCKS4".to_string();
                meta.source = Address::SocketAddress(peer_addr);
                meta.target = handshake_req.dst.into();

                if let Err(err) = tcp_in.send(InboundConnection::with_context(s.into_inner(), meta)).await {
                    error!("failed to send inbound connection to tcp_in: {}", err);
                }

                Ok(())
            }
            Command::Bind => {
                warn!("BIND is not supported");

                let handshake_rsp = HandshakeResponse::new(ResultCode::RequestRejectedOrFailed);
                handshake_rsp.write_to(&mut s).await?;

                Ok(())
            }
        }
    }
}

mod socks5_impl {

    use std::str;

    use crate::context::Metadata;

    use super::*;

    use swiftlink_transport::socks5::{
        self, Address, Command, HandshakeRequest, HandshakeResponse, PasswdAuthRequest, PasswdAuthResponse,
        TcpRequestHeader, TcpResponseHeader,
    };

    pub async fn handle_client(
        mut stream: TcpStream,
        peer_addr: SocketAddr,
        authenticator: Arc<Option<Authenticator>>,
        tcp_in: mpsc::Sender<InboundConnection>,
    ) -> io::Result<()> {
        // 1. Handshake

        let handshake_req = match HandshakeRequest::read_from(&mut stream).await {
            Ok(r) => r,
            Err(socks5::Error::IoError(ref err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                trace!("socks5 handshake early eof. peer: {}", peer_addr);
                return Ok(());
            }
            Err(err) => {
                error!("socks5 handshake error: {}", err);
                return Err(err.into());
            }
        };

        trace!("socks5 {:?}", handshake_req);

        // 2. Authentication
        check_auth(&mut stream, &handshake_req, authenticator).await?;

        // 3. Fetch headers

        let header = match TcpRequestHeader::read_from(&mut stream).await {
            Ok(r) => r,
            Err(err) => {
                error!("failed to get TCP request header: {}, peer: {}", err, peer_addr);
                let rh = TcpResponseHeader::new(err.as_reply(), Address::SocketAddress(peer_addr));
                rh.write_to(&mut stream).await?;
                return Err(err.into());
            }
        };

        trace!("socks5 {:?} peer: {}", header, peer_addr);

        // 4. Handle command

        let addr = header.address;

        match header.command {
            Command::TcpConnect => {
                debug!("CONNECT {}", addr);

                let mut meta = Metadata::default();
                meta.inbound_tag = "SOCKS5".to_string();
                meta.source = Address::SocketAddress(peer_addr);
                meta.target = addr;

                if let Err(err) = tcp_in.send(InboundConnection::with_context(stream, meta)).await {
                    error!("failed to send inbound connection to tcp_in: {}", err);
                }

                Ok(())
            }
            Command::UdpAssociate => {
                debug!("UDP ASSOCIATE from {}", addr);

                // TODO:

                todo!()
            }
            Command::TcpBind => {
                warn!("BIND is not supported");
                let rh = TcpResponseHeader::new(socks5::Reply::CommandNotSupported, addr);
                rh.write_to(&mut stream).await?;

                Ok(())
            }
        }
    }

    async fn check_auth(
        stream: &mut TcpStream,
        handshake_req: &HandshakeRequest,
        authenticator: Arc<Option<Authenticator>>,
    ) -> io::Result<()> {
        for method in handshake_req.methods.iter() {
            match *method {
                socks5::SOCKS5_AUTH_METHOD_PASSWORD => {
                    let resp = HandshakeResponse::new(socks5::SOCKS5_AUTH_METHOD_PASSWORD);
                    trace!("reply handshake {:?}", resp);
                    resp.write_to(stream).await?;

                    return check_auth_password(stream, authenticator).await;
                }
                socks5::SOCKS5_AUTH_METHOD_NONE => {
                    if !authenticator.is_some() {
                        let resp = HandshakeResponse::new(socks5::SOCKS5_AUTH_METHOD_NONE);
                        trace!("reply handshake {:?}", resp);
                        resp.write_to(stream).await?;

                        return Ok(());
                    }
                    trace!("none authentication method is not allowed");
                }
                _ => {
                    trace!("unsupported authentication method {}", method);
                }
            }
        }

        let resp = HandshakeResponse::new(socks5::SOCKS5_AUTH_METHOD_NOT_ACCEPTABLE);
        resp.write_to(stream).await?;

        trace!("reply handshake {:?}", resp);

        Err(io::Error::new(
            io::ErrorKind::Other,
            "currently swiftlink does not support authentication",
        ))
    }

    async fn check_auth_password(stream: &mut TcpStream, authenticator: Arc<Option<Authenticator>>) -> io::Result<()> {
        const PASSWORD_AUTH_STATUS_FAILURE: u8 = 255;

        // Read initiation negociation

        let req = match PasswdAuthRequest::read_from(stream).await {
            Ok(i) => i,
            Err(err) => {
                let rsp = PasswdAuthResponse::new(err.as_reply().as_u8());
                let _ = rsp.write_to(stream).await;

                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Username/Password Authentication Initial request failed: {err}"),
                ));
            }
        };

        let uname = match str::from_utf8(&req.uname) {
            Ok(i) => i,
            Err(_) => {
                let rsp = PasswdAuthResponse::new(PASSWORD_AUTH_STATUS_FAILURE);
                let _ = rsp.write_to(stream).await;

                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Username/Password Authentication Initial request uname contains invaid characters",
                ));
            }
        };

        let passwd = match str::from_utf8(&req.passwd) {
            Ok(u) => u,
            Err(..) => {
                let rsp = PasswdAuthResponse::new(PASSWORD_AUTH_STATUS_FAILURE);
                let _ = rsp.write_to(stream).await;

                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Username/Password Authentication Initial request passwd contains invaid characters",
                ));
            }
        };

        if let Some(authenticator) = authenticator.as_ref() {
            if authenticator.verify(uname, passwd) {
                trace!(
                    "socks5 authenticated with Username/Password method, user: {}, password: {}",
                    uname,
                    passwd
                );

                let rsp = PasswdAuthResponse::new(0);
                rsp.write_to(stream).await?;

                return Ok(());
            } else {
                let rsp = PasswdAuthResponse::new(PASSWORD_AUTH_STATUS_FAILURE);
                let _ = rsp.write_to(stream).await;

                error!(
                    "socks5 rejected Username/Password user: {}, password: {}",
                    uname, passwd
                );

                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Username/Password Authentication failed, user: {uname}, password: {passwd}"),
                ));
            }
        }

        // No authentication required
        Ok(())
    }
}
