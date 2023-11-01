use std::net::{AddrParseError, SocketAddr};

pub fn split_options(opt: &str, pat: char) -> impl Iterator<Item = &str> {
    opt.split(pat).filter(|p| !p.is_empty())
}

pub fn parse_sock_addrs(addr: &str) -> Result<SocketAddr, AddrParseError> {
    let addr = addr.trim();
    if let Some(port) = addr.to_lowercase().strip_prefix("localhost:") {
        format!("127.0.0.1:{}", port).as_str().parse()
    } else if addr.starts_with(':') {
        format!("0.0.0.0{}", addr).as_str().parse()
    } else {
        addr.parse()
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn test_addr_parse() {
        assert_eq!(
            parse_sock_addrs("localhost:123"),
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 123))
        );
        assert_eq!(
            parse_sock_addrs("0.0.0.0:123"),
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 123))
        );
        assert_eq!(
            parse_sock_addrs(":123"),
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 123))
        );
        assert_eq!(
            parse_sock_addrs("[::1]:123"),
            "[::1]:123".parse::<SocketAddr>()
        );
        assert_eq!(
            parse_sock_addrs("[::]:123"),
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 123))
        );
    }
}
