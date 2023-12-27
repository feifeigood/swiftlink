use anyhow::Context;
use fast_socks5::{client::Socks5Stream, util::target_addr::ToTargetAddr, AuthenticationMethod, Socks5Command};
use serde_with::DeserializeFromStr;
use std::{
    fmt::{Display, Write},
    io,
    net::{AddrParseError, SocketAddr},
    ops::{Deref, DerefMut},
    pin::Pin,
    str::FromStr,
};
use tokio::net::TcpStream as TokioTcpStream;

use thiserror::Error;
use url::{ParseError, Url};

use swiftlink_infra::{
    log::info,
    net::{tcp::crate_tcp_stream_with_opts, ConnectOpts},
};

pub async fn connect_tcp(
    server_addr: SocketAddr,
    proxy: Option<&ProxyConfig>,
    opts: &ConnectOpts,
) -> io::Result<TcpStream> {
    let target_addr = server_addr.ip().to_string();
    let target_port = server_addr.port();

    let create_tcp_stream =
        |server_addr: SocketAddr| async move { crate_tcp_stream_with_opts(server_addr, opts).await };

    match proxy {
        Some(proxy) => match proxy.proto {
            ProxyProtocol::Socks5 => {
                let tcp = create_tcp_stream(server_addr).await?;
                let auth = {
                    if proxy.username.is_some() {
                        let auth = AuthenticationMethod::Password {
                            username: proxy.username.as_deref().map(|s| s.to_owned()).unwrap_or_default(),
                            password: proxy.password.as_deref().map(|s| s.to_owned()).unwrap_or_default(),
                        };
                        Some(auth)
                    } else {
                        None
                    }
                };

                let socks5stream = upgrade_to_socks5stream(tcp, auth, target_addr, target_port).await;

                socks5stream
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
                    .map(TcpStream::Proxy)
            }
            ProxyProtocol::Http => {
                use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};

                let mut tcp = create_tcp_stream(server_addr).await?;

                if let Some(user) = proxy.username.as_deref() {
                    http_connect_tokio_with_basic_auth(
                        &mut tcp,
                        &target_addr,
                        target_port,
                        user,
                        proxy.password.as_deref().unwrap_or(""),
                    )
                    .await
                } else {
                    http_connect_tokio(&mut tcp, &target_addr, target_port).await
                }
                .map_err(from_http_err)?;

                Ok(TcpStream::Tokio(tcp))
            }
        },
        None => create_tcp_stream(server_addr).await.map(TcpStream::Tokio),
    }
}

async fn upgrade_to_socks5stream(
    socket: TokioTcpStream,
    auth: Option<AuthenticationMethod>,
    target_addr: String,
    target_port: u16,
) -> anyhow::Result<Socks5Stream<TokioTcpStream>> {
    info!("Connected @ {}", &socket.peer_addr()?);

    // Specify the target, here domain name, dns will be resolved on the server side
    let target_addr = (target_addr.as_str(), target_port)
        .to_target_addr()
        .context("Can't convert address to TargetAddr format")?;

    // upgrade the TcpStream to Socks5Stream
    let mut socks_stream = Socks5Stream::use_stream(socket, auth, Default::default()).await?;
    socks_stream.request(Socks5Command::TCPConnect, target_addr).await?;

    Ok(socks_stream)
}

fn from_http_err(err: async_http_proxy::HttpError) -> io::Error {
    match err {
        async_http_proxy::HttpError::IoError(io) => io,
        err => io::Error::new(io::ErrorKind::ConnectionRefused, err),
    }
}

pub enum TcpStream {
    Tokio(TokioTcpStream),
    Proxy(Socks5Stream<TokioTcpStream>),
}

impl Deref for TcpStream {
    type Target = TokioTcpStream;

    fn deref(&self) -> &Self::Target {
        match self {
            TcpStream::Tokio(tcp) => tcp,
            TcpStream::Proxy(proxy) => proxy.get_socket_ref(),
        }
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            TcpStream::Tokio(tcp) => tcp,
            TcpStream::Proxy(proxy) => proxy.get_socket_mut(),
        }
    }
}

impl tokio::io::AsyncRead for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            TcpStream::Tokio(s) => Pin::new(s).poll_read(cx, buf),
            TcpStream::Proxy(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        match self.get_mut() {
            TcpStream::Tokio(s) => Pin::new(s).poll_write(cx, buf),
            TcpStream::Proxy(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), io::Error>> {
        match self.get_mut() {
            TcpStream::Tokio(s) => Pin::new(s).poll_flush(cx),
            TcpStream::Proxy(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), io::Error>> {
        match self.get_mut() {
            TcpStream::Tokio(s) => Pin::new(s).poll_shutdown(cx),
            TcpStream::Proxy(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

// #[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, DeserializeFromStr)]
pub struct ProxyConfig {
    pub proto: ProxyProtocol,
    pub server: SocketAddr,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Display for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.proto {
            ProxyProtocol::Socks5 => "socks5://",
            ProxyProtocol::Http => "http://",
        })?;

        if let Some(user) = self.username.as_deref() {
            f.write_str(user)?;

            if let Some(pwd) = self.password.as_deref() {
                f.write_char(':')?;
                f.write_str(pwd)?;
            }
            f.write_char('@')?;
        }

        write!(f, "{}", self.server)?;

        Ok(())
    }
}

impl FromStr for ProxyConfig {
    type Err = ProxyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = Url::from_str(s)?;

        let proto = match url.scheme() {
            "socks5" => ProxyProtocol::Socks5,
            "http" => ProxyProtocol::Http,
            scheme => return Err(ProxyParseError::UnexpectedSchema(scheme.to_string())),
        };

        let server = match url
            .socket_addrs(|| match proto {
                ProxyProtocol::Socks5 => Some(1080),
                _ => None,
            })
            .into_iter()
            .flatten()
            .next()
        {
            Some(s) => s,
            None => return Err(ParseError::InvalidDomainCharacter.into()),
        };

        let mut username = Some(url.username());
        if matches!(username, Some("")) {
            username = None;
        }

        let password = url.password();

        Ok(Self {
            proto,
            server,
            username: username.map(|s| s.to_owned()),
            password: password.map(|s| s.to_owned()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyProtocol {
    Socks5,
    Http,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ProxyParseError {
    #[error("UnexpectedSchema {0:?}")]
    UnexpectedSchema(String),
    #[error(" address parse error {0:?}")]
    Addr(#[from] AddrParseError),
    #[error("{0:?}")]
    Parse(#[from] ParseError),
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    #[test]
    fn test_parse_socks5() {
        assert_eq!(
            ProxyConfig::from_str("socks5://1.2.3.4:1080"),
            Ok(ProxyConfig {
                proto: ProxyProtocol::Socks5,
                server: "1.2.3.4:1080".parse().unwrap(),
                username: None,
                password: None
            })
        );
    }

    #[test]
    fn test_parse_socks5_with_user() {
        assert_eq!(
            ProxyConfig::from_str("socks5://user123@1.2.3.4:1080"),
            Ok(ProxyConfig {
                proto: ProxyProtocol::Socks5,
                server: "1.2.3.4:1080".parse().unwrap(),
                username: Some("user123".to_string()),
                password: None
            })
        );

        let url = Url::from_str("abc://user123@1.2.3.4:1080").unwrap();

        assert_eq!(url.username(), "user123");
        assert_eq!(url.password(), None);
    }

    #[test]
    fn test_parse_socks5_with_user_pass() {
        assert_eq!(
            ProxyConfig::from_str("socks5://user123:pass456@1.2.3.4:1080"),
            Ok(ProxyConfig {
                proto: ProxyProtocol::Socks5,
                server: "1.2.3.4:1080".parse().unwrap(),
                username: Some("user123".to_string()),
                password: Some("pass456".to_string())
            })
        );
    }

    #[test]
    fn test_parse_http() {
        assert_eq!(
            ProxyConfig::from_str("http://1.2.3.4:8080"),
            Ok(ProxyConfig {
                proto: ProxyProtocol::Http,
                server: "1.2.3.4:8080".parse().unwrap(),
                username: None,
                password: None
            })
        );
    }
}
