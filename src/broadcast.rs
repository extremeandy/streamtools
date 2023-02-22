use std::{
    pin::Pin,
    sync::{Arc, Weak},
    task::{ready, Poll},
};

use futures::{
    stream::{BoxStream, FusedStream},
    FutureExt, Stream, StreamExt,
};
use parking_lot::RwLock;
use pin_project_lite::pin_project;
use tokio::sync::broadcast::{self, error::SendError, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::ReusableBoxFuture;

pin_project! {
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct Broadcast<S>
    where
        S: Stream,
    {
        state: Arc<RwLock<ReusableBoxFuture<'static, Option<(BoxStream<'static, S::Item>, Arc<Sender<S::Item>>)>>>>,

        #[pin]
        stream: Option<BroadcastStream<S::Item>>,

        tx: Weak<Sender<S::Item>>
    }
}

impl<S> Broadcast<S>
where
    S: Stream + Send + 'static,
    S::Item: Clone + Send + 'static,
{
    pub fn new(stream: S, capacity: usize) -> Self {
        // Box::pin to avoid Unpin bound on S. We might be able to avoid this but it's probably not worth the effort!
        let stream = stream.boxed();
        let (tx, rx) = broadcast::channel(capacity);
        let tx = Arc::new(tx);

        let tx_weak = Arc::downgrade(&tx);

        let state = Arc::new(RwLock::new(ReusableBoxFuture::new(make_future(stream, tx))));

        let stream = Some(BroadcastStream::new(rx));

        Self {
            state,
            stream,
            tx: tx_weak,
        }
    }
}

async fn make_future<S>(
    mut stream: S,
    tx: Arc<Sender<S::Item>>,
) -> Option<(S, Arc<Sender<S::Item>>)>
where
    S: Stream + Unpin,
{
    let Some(value) = stream.next().await else {
        return None;
    };

    tx.send(value)
        .map_err(|_| SendError(())) // Avoid need for std::fmt::Debug on S::Item
        .expect("receiver cannot have been dropped, since the caller is expected to own one");

    Some((stream, tx))
}

impl<S> Stream for Broadcast<S>
where
    S: Stream + Send + 'static,
    S::Item: Clone + Send + 'static,
{
    type Item = S::Item;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let Some(mut stream) = this.stream.as_mut().as_pin_mut() else {
            // Clone always succeeds, but it doesn't mean we will have a stream, as the sender
            // may have hung up. Also, we set the stream to None once we terminate, to ensure
            // we don't poll the underlying stream again.
            return Poll::Ready(None);
        };

        match stream.poll_next_unpin(cx) {
            Poll::Ready(ready) => match ready {
                Some(value) => match value {
                    Ok(value) => {
                        return Poll::Ready(Some(value));
                    }
                    Err(_) => {
                        // We lagged!? Log an error?
                        // TODO: CHECK THIS!
                        this.stream.set(None);
                        return Poll::Ready(None);
                    }
                },
                None => {
                    this.stream.set(None);
                    return Poll::Ready(None);
                }
            },
            Poll::Pending => {}
        }

        // If we are here then we've exhausted the broadcast stream. Let's see about fetching
        // another value from the underlying stream.
        let mut future = this.state.write();

        let (inner_stream, tx) = match ready!(future.poll_unpin(cx)) {
            Some(inner) => inner,
            None => {
                this.stream.set(None);
                return Poll::Ready(None);
            }
        };

        future.set(make_future(inner_stream, tx));
        drop(future);

        match ready!(stream.poll_next(cx)) {
            Some(value) => match value {
                Ok(value) => Poll::Ready(Some(value)),
                Err(_) => {
                    // We lagged!? Log an error?
                    // TODO: CHECK THIS!
                    this.stream.set(None);
                    Poll::Ready(None)
                }
            },
            None => {
                this.stream.set(None);
                Poll::Ready(None)
            }
        }
    }
}

impl<S> Clone for Broadcast<S>
where
    S: Stream,
    S::Item: Clone + Send + 'static,
{
    fn clone(&self) -> Self {
        let Self {
            state,
            stream: _,
            tx,
        } = self;

        let stream = tx.upgrade().map(|tx| BroadcastStream::new(tx.subscribe()));

        Self {
            state: Arc::clone(state),
            tx: self.tx.clone(),
            stream,
        }
    }
}

impl<S> FusedStream for Broadcast<S>
where
    S: Stream + Send + Unpin + 'static,
    S::Item: Clone + Send + 'static,
{
    fn is_terminated(&self) -> bool {
        self.stream.is_none()
    }
}

#[cfg(test)]
mod tests {
    use futures::SinkExt;
    use tokio_test::{assert_pending, assert_ready_eq};

    use super::*;

    #[tokio::test]
    async fn test_broadcast() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx, rx) = futures::channel::mpsc::unbounded();

        let mut stream = Broadcast::new(rx, 5);

        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(1).await.unwrap();
        //tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(2).await.unwrap();

        let mut stream_cloned = stream.clone();

        tx.send(3).await.unwrap();

        //tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(2));
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(3));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        // We cloned the stream after value 2 was sent, but because we hadn't polled any of the streams, it's still
        // in the buffer. So is 3.
        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), Some(2));
        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), Some(3));
        assert_pending!(stream_cloned.poll_next_unpin(&mut cx));

        drop(stream); // Drop the original stream... it shouldn't affect the cloned stream.

        // Send a value and then terminate the stream
        tx.send(4).await.unwrap();

        //tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), Some(4)); // We still see the last value even though the stream was terminated
        assert_pending!(stream_cloned.poll_next_unpin(&mut cx));

        // Send a value and then terminate the stream
        tx.send(5).await.unwrap();
        drop(tx);
        //tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), Some(5)); // We still see the last value even though the stream was terminated
        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), None);
        assert_ready_eq!(stream_cloned.poll_next_unpin(&mut cx), None); // Should continue to return None if polled again

        // What if we clone once the stream has completed?
        let mut stream_cloned_again = stream_cloned.clone();
        assert_ready_eq!(stream_cloned_again.poll_next_unpin(&mut cx), None);
    }
}
