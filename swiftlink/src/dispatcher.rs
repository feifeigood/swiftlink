use tokio::io::{AsyncRead, AsyncWrite};

use crate::context::Metadata;

pub struct Dispatcher {}

impl Dispatcher {
    pub async fn dispatch_stream<S>(&self, mut metadata: Metadata, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    {
        todo!("dispatch stream")
    }
}
