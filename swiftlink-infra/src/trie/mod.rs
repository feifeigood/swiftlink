use std::collections::HashMap;

pub mod domain_trie;

#[derive(Debug)]
pub struct TrieNode<T> {
    data: Option<T>,
    children: HashMap<String, TrieNode<T>>,
}

impl<T> TrieNode<T> {
    fn new() -> Self {
        TrieNode {
            data: None,
            children: HashMap::new(),
        }
    }
}
