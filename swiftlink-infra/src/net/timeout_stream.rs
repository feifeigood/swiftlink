use std::{
    io, mem,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::stream::{Stream, StreamExt};
use futures_util::FutureExt;
use tokio::time::Sleep;

/// This wraps the underlying stream with a timeout.
///
/// Any `Ok(Poll::Ready(_))` from the underlying Stream will reset the timeout.
pub struct TimeoutStream<S> {
    stream: S,
    timeout_duration: Duration,
    timeout: Option<Pin<Box<Sleep>>>,
}

impl<S> TimeoutStream<S> {
    /// Returns a new TimeoutStream
    ///
    /// # Arguments
    ///
    /// * `stream` - stream to wrap
    /// * `timeout_duration` - timeout between each request, once exceed the connection is killed
    /// * `reactor_handle` - reactor used for registering new timeouts
    pub fn new(stream: S, timeout_duration: Duration) -> Self {
        Self {
            stream,
            timeout_duration,
            timeout: None,
        }
    }

    fn timeout(timeout_duration: Duration) -> Option<Pin<Box<Sleep>>> {
        if timeout_duration > Duration::from_secs(0) {
            Some(Box::pin(tokio::time::sleep(timeout_duration)))
        } else {
            None
        }
    }
}

impl<S, I> Stream for TimeoutStream<S>
where
    S: Stream<Item = Result<I, io::Error>> + Unpin,
{
    type Item = Result<I, io::Error>;

    // shomehow insert a timeout here...
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.timeout.is_none() {
            let timeout = Self::timeout(self.timeout_duration);
            self.as_mut().timeout = timeout;
        }

        match self.stream.poll_next_unpin(cx) {
            r @ Poll::Ready(_) => {
                // reset the timeout to wait for the next request
                let timeout = if let Some(mut timeout) = Self::timeout(self.timeout_duration) {
                    // ensure that interest in the Timeout is registered
                    match timeout.poll_unpin(cx) {
                        Poll::Ready(_) => {
                            return Poll::Ready(Some(Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                format!("timeout fired immediately"),
                            ))));
                        }
                        Poll::Pending => (),
                    }
                    Some(timeout)
                } else {
                    None
                };

                drop(mem::replace(&mut self.timeout, timeout));

                r
            }
            Poll::Pending => {
                if let Some(ref mut timeout) = self.timeout {
                    match timeout.poll_unpin(cx) {
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(()) => Poll::Ready(Some(Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("nothing ready in {:?}", self.timeout_duration),
                        )))),
                    }
                } else {
                    Poll::Pending
                }
            }
        }
    }
}
