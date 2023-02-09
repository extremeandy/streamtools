use std::{pin::Pin, task::Poll};

use futures::{stream::FusedStream, Stream, StreamExt};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for the [`fast_forward`](crate::StreamTools::fast_forward) method.
    #[must_use = "streams do nothing unless polled"]
    pub struct FastForward<S> {
        #[pin]
        inner: Option<S>
    }
}

impl<S> FastForward<S> {
    pub(super) fn new(stream: S) -> Self {
        Self {
            inner: Some(stream),
        }
    }
}

impl<S> Stream for FastForward<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut this = self.project();

        let Some(mut inner) = this.inner.as_mut().as_pin_mut() else {
            // Last time we polled, the inner stream terminated, but we yielded a value.
            // If we are here then it's time to terminate.
            return Poll::Ready(None)
        };

        let mut last_value = None;

        while let Poll::Ready(ready) = inner.poll_next_unpin(cx) {
            match ready {
                Some(value) => {
                    last_value = Some(value);
                }
                None => {
                    // Clear inner so that if we poll again we will _definitely_ return Poll::Ready(None)
                    this.inner.set(None);
                    break;
                }
            }
        }

        match last_value {
            Some(value) => Poll::Ready(Some(value)),
            None => match this.inner.as_pin_mut() {
                Some(_) => Poll::Pending, // The stream didn't terminate yet, so we must be pending
                None => Poll::Ready(None), // The stream did terminate and there was no value seen, so we are done.
            },
        }
    }
}

impl<S> FusedStream for FastForward<S>
where
    S: Stream,
{
    fn is_terminated(&self) -> bool {
        self.inner.is_none()
    }
}

impl<S> std::fmt::Debug for FastForward<S>
where
    S: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlattenSwitch")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use futures::{stream, SinkExt};
    use tokio_test::{assert_pending, assert_ready_eq};

    use super::*;

    #[tokio::test]
    async fn test_fast_forward() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx, rx) = futures::channel::mpsc::unbounded();

        let mut stream = FastForward::new(rx);

        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(1).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(2).await.unwrap(); // This value gets skipped
        tx.send(3).await.unwrap();

        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(3));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        // Send a value and then terminate the stream
        tx.send(4).await.unwrap();
        drop(tx);

        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(4)); // We still see the last value even though the stream was terminated
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None);
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None); // Should continue to return None if polled again
    }

    #[tokio::test]
    async fn test_fast_forward_empty_stream() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let mut stream = FastForward::new(stream::empty::<()>());
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None);
    }

    #[tokio::test]
    async fn test_fast_forward_drop_before_polled() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx, rx) = futures::channel::mpsc::unbounded();

        let mut stream = FastForward::new(rx);

        tx.send(1).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        drop(tx); // Terminate the stream without sending any more values
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None);
    }
}
