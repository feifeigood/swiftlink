use std::{io, net::SocketAddr};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not register {0} listener on addresss {1}, due to {2}")]
    RegisterListenerFailed(&'static str, SocketAddr, String),
    /// An underlying IO error occurred
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}
