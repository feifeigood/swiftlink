use super::TrieNode;
use consts::*;

#[rustfmt::skip]
mod consts {
    pub const WILDCARD         :&str = "*";
    pub const DOT_WILDCARD     :&str = "";
    pub const COMPLEX_WILDCARD :&str = "+";
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum DomainTrieError {
    #[error("Invalid domain `{0}`")]
    InvalidDomain(String),
}

/// DomainTrie contains the main logic for adding and searching nodes for domain segments.
/// support wildcard domain (e.g *.google.com)
#[derive(Debug)]
pub struct DomainTrie<T>
where
    T: Clone,
{
    root: TrieNode<T>,
}

impl<T: Clone> Default for DomainTrie<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> DomainTrie<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        DomainTrie {
            root: TrieNode::new(),
        }
    }

    /// adds a node to the domain trie.
    /// examples:
    /// - www.example.com
    /// - *.example.com
    /// - subdomain.*.example.com
    /// - .example.com
    /// - +.example.com
    pub fn insert(&mut self, domain: String, data: T) -> Result<(), DomainTrieError> {
        let mut parts = valid_and_split_domain(domain)?;

        if parts[0] == COMPLEX_WILDCARD {
            self.insert_inner(&parts.as_slice()[1..], data.to_owned());
            parts[0] = DOT_WILDCARD.to_string();
        }

        self.insert_inner(&parts.as_slice(), data);

        Ok(())
    }

    fn insert_inner(&mut self, parts: &[String], data: T) {
        let mut current_node = &mut self.root;

        for part in parts.iter().rev() {
            let next_node = current_node
                .children
                .entry(part.to_owned())
                .or_insert(TrieNode::new());
            current_node = next_node;
        }

        current_node.data = Some(data);
    }

    pub fn search(&self, doamin: String) -> Option<T> {
        if let Ok(mut parts) = valid_and_split_domain(doamin) {
            if !parts[0].is_empty() {
                parts.reverse();
                return self.search_inner(&self.root, parts.as_slice());
            }
        }

        None
    }

    fn search_inner(&self, node: &TrieNode<T>, parts: &[String]) -> Option<T> {
        if parts.len() == 0 {
            return node.data.clone();
        }

        if let Some(c) = node.children.get(&parts[0]) {
            if let Some(n) = self.search_inner(c, &parts[1..]) {
                return Some(n);
            }
        }

        if let Some(c) = node.children.get(WILDCARD) {
            if let Some(n) = self.search_inner(c, &parts[1..]) {
                return Some(n);
            }
        }

        node.children
            .get(DOT_WILDCARD)
            .and_then(|node| node.data.clone())
    }
}

/// example:
///   localhost     -> ["localhost"]
///   example.com   -> ["example", "com"]
fn valid_and_split_domain(domain: String) -> Result<Vec<String>, DomainTrieError> {
    if !domain.is_empty() && domain.ends_with('.') {
        return Err(DomainTrieError::InvalidDomain(domain));
    }

    let parts = domain.split('.').map(|x| x.into()).collect::<Vec<String>>();
    if parts.len() == 1 {
        if parts[0].is_empty() {
            return Err(DomainTrieError::InvalidDomain(domain));
        }

        return Ok(parts);
    }

    for part in parts.iter().skip(1) {
        if part.is_empty() {
            return Err(DomainTrieError::InvalidDomain(domain));
        }
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {

    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn test_domain_trie_basic() {
        let localhost = Ipv4Addr::LOCALHOST.into();

        let mut tree = DomainTrie::<IpAddr>::new();
        let domains = vec!["example.com", "google.com", "localhost"];

        for dn in domains {
            assert_eq!(tree.insert(dn.to_string(), localhost), Ok(()));
        }

        let data = tree.search(String::from("example.com"));
        assert_ne!(data, None);
        assert_eq!(data.unwrap(), localhost);
        assert_eq!(
            tree.insert(String::from(""), localhost),
            Err(DomainTrieError::InvalidDomain(String::from("")))
        );
        assert_eq!(tree.search(String::from("")), None);
        assert_eq!(tree.search(String::from("www.google.com")), None);
        assert_ne!(tree.search(String::from("localhost")), None);
    }

    #[test]
    fn test_domain_trie_wildcard() {
        let localhost = Ipv4Addr::LOCALHOST.into();

        let mut tree = DomainTrie::<IpAddr>::new();
        let domains = vec![
            "*.example.com",
            "sub.*.example.com",
            "*.dev",
            ".org",
            ".example.net",
            ".apple.*",
            "+.foo.com",
            "+.stun.*.*",
            "+.stun.*.*.*",
            "+.stun.*.*.*.*",
            "stun.l.google.com",
        ];

        for dn in domains {
            assert_eq!(tree.insert(dn.to_string(), localhost), Ok(()));
        }

        assert_ne!(tree.search(String::from("sub.example.com")), None);
        assert_ne!(tree.search(String::from("sub.foo.example.com")), None);
        assert_ne!(tree.search(String::from("test.org")), None);
        assert_ne!(tree.search(String::from("test.example.net")), None);
        assert_ne!(tree.search(String::from("test.apple.com")), None);
        assert_ne!(tree.search(String::from("test.foo.com")), None);
        assert_ne!(tree.search(String::from("foo.com")), None);
        assert_ne!(tree.search(String::from("global.stun.website.com")), None);
        assert_eq!(tree.search(String::from("foo.sub.example.com")), None);
        assert_eq!(tree.search(String::from("foo.example.dev")), None);
        assert_eq!(tree.search(String::from("example.com")), None);
    }

    #[test]
    fn test_domain_trie_priority() {
        let mut tree = DomainTrie::<IpAddr>::new();
        let domains = vec![
            (".dev", "0.0.0.1".parse::<Ipv4Addr>().unwrap().into()),
            ("example.dev", "0.0.0.2".parse::<Ipv4Addr>().unwrap().into()),
            (
                "*.example.dev",
                "0.0.0.3".parse::<Ipv4Addr>().unwrap().into(),
            ),
            (
                "test.example.dev",
                "0.0.0.4".parse::<Ipv4Addr>().unwrap().into(),
            ),
        ];

        for (dn, val) in domains {
            assert_eq!(tree.insert(dn.to_string(), val), Ok(()));
        }

        let assert_fn = |dn, want: IpAddr| {
            let data = tree.search(dn);
            assert_ne!(data, None);
            assert_eq!(data.unwrap(), want);
        };

        assert_fn(
            String::from("test.dev"),
            "0.0.0.1".parse::<Ipv4Addr>().unwrap().into(),
        );
        assert_fn(
            String::from("foo.bar.dev"),
            "0.0.0.1".parse::<Ipv4Addr>().unwrap().into(),
        );
        assert_fn(
            String::from("example.dev"),
            "0.0.0.2".parse::<Ipv4Addr>().unwrap().into(),
        );
        assert_fn(
            String::from("foo.example.dev"),
            "0.0.0.3".parse::<Ipv4Addr>().unwrap().into(),
        );
        assert_fn(
            String::from("test.example.dev"),
            "0.0.0.4".parse::<Ipv4Addr>().unwrap().into(),
        );
    }

    #[test]
    fn test_domain_trie_boundary() {
        let localhost: IpAddr = Ipv4Addr::LOCALHOST.into();
        let mut tree = DomainTrie::<IpAddr>::new();
        assert_eq!(tree.insert(String::from("*.dev"), localhost), Ok(()));

        assert_eq!(
            tree.insert(String::from("."), localhost),
            Err(DomainTrieError::InvalidDomain(String::from(".")))
        );
        assert!(tree.insert(String::from("..dev"), localhost).is_err());
        assert!(tree.insert(String::from("dev"), localhost).is_ok());
    }

    #[test]
    fn test_domain_trie_wildcard_boundary() {
        let localhost = Ipv4Addr::LOCALHOST.into();
        let mut tree = DomainTrie::<IpAddr>::new();
        assert_eq!(tree.insert(String::from("+.*"), localhost), Ok(()));
        assert_eq!(tree.insert(String::from("stun.*.*.*"), localhost), Ok(()));

        assert_ne!(tree.search(String::from("example.com")), None);
    }
}
