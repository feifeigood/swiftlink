use std::io;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub mod socks;

mod association;

pub struct ServerHandle(JoinSet<io::Result<()>>, CancellationToken);

impl ServerHandle {
    pub async fn shutdown_gracefully(&mut self) {
        self.1.cancel();
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.0.abort_all();
    }
}
