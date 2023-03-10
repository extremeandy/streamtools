use futures::task;
use futures::Stream;
use pin_project_lite::pin_project;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::outer_waker::OuterWaker;

pin_project! {
    /// Stream for the [`flatten_switch`](crate::StreamTools::flatten_switch) method.
    #[must_use = "streams do nothing unless polled"]
    pub struct FlattenSwitch<St>
    where
        St: Stream,
        St::Item: Stream
    {
        #[pin]
        outer: St,

        outer_waker: Arc<OuterWaker>,

        #[pin]
        inner: Option<<St as Stream>::Item>
    }
}

impl<St> FlattenSwitch<St>
where
    St: Stream,
    St::Item: Stream,
{
    pub(super) fn new(stream: St) -> Self {
        Self {
            outer: stream,
            outer_waker: Arc::default(),
            inner: None,
        }
    }
}

impl<St> Stream for FlattenSwitch<St>
where
    St: Stream,
    St::Item: Stream,
{
    type Item = <St::Item as Stream>::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut this = self.project();

        // We can avoid polling the outer stream if its waker has not been woken since
        // we were last polled
        let outer_ready = this.outer_waker.set_parent_waker(cx.waker().clone());
        if outer_ready {
            let waker = task::waker(Arc::clone(this.outer_waker));
            let mut cx = Context::from_waker(&waker);
            while let Poll::Ready(inner) = this.outer.as_mut().poll_next(&mut cx) {
                match inner {
                    Some(inner) => this.inner.set(Some(inner)),
                    None => {
                        // Terminate when the outer stream terminates
                        return Poll::Ready(None);
                    }
                }
            }
        };

        match this.inner.as_mut().as_pin_mut() {
            Some(inner) => match inner.poll_next(cx) {
                Poll::Ready(value) => match value {
                    Some(value) => Poll::Ready(Some(value)),
                    None => {
                        // If the inner stream terminated, clear it so we don't poll it again.
                        // This is important because some Streams don't support being polled again after
                        // termination, e.g. stream::unfold.
                        this.inner.set(None);

                        // The inner stream can terminate but we don't terminate until the outer stream ends.
                        Poll::Pending
                    }
                },

                // Waiting on inner stream to emit next
                Poll::Pending => Poll::Pending,
            },

            // We are still waiting for the first inner stream to be emitted by the outer
            None => Poll::Pending,
        }
    }
}

impl<S> std::fmt::Debug for FlattenSwitch<S>
where
    S: Stream + std::fmt::Debug,
    S::Item: Stream + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlattenSwitch")
            .field("stream", &self.outer)
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::future;

    use futures::{stream, FutureExt, StreamExt};
    use parking_lot::Mutex;
    use tokio_test::{assert_pending, assert_ready_eq};

    use super::*;

    pin_project! {
        struct MockStream<S: Stream> {
            #[pin]
            inner: S,
            polled: Arc<Mutex<bool>>
        }
    }

    impl<S: Stream> Stream for MockStream<S> {
        type Item = S::Item;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            let this = self.project();
            let result = this.inner.poll_next(cx);

            *this.polled.lock() = true;

            result
        }
    }

    #[tokio::test]
    async fn test_flatten_switch() {
        use futures::{channel::mpsc, SinkExt, StreamExt};
        use tokio::sync::broadcast::{self, error::SendError};
        use tokio_stream::wrappers::BroadcastStream;

        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (tx_inner1, rx_inner1) = broadcast::channel(32);
        let (tx_inner2, rx_inner2) = broadcast::channel(32);
        let (tx_inner3, rx_inner3) = broadcast::channel(32);
        let (mut tx, rx) = mpsc::unbounded();

        let outer_polled = Arc::new(Mutex::new(false));

        let take_outer_polled = || -> bool {
            let mut guard = outer_polled.lock();
            std::mem::replace(&mut guard, false)
        };

        let assert_outer_polled = || assert!(take_outer_polled());
        let assert_outer_not_polled = || assert!(!take_outer_polled());

        let outer_stream = MockStream {
            inner: rx,
            polled: Arc::clone(&outer_polled),
        };

        let mut switch_stream = FlattenSwitch::new(outer_stream);

        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_polled();

        tx.send(
            BroadcastStream::new(rx_inner1)
                .map(|r: Result<_, _>| r.unwrap())
                .boxed(),
        )
        .await
        .unwrap();

        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_polled();

        tx_inner1.send(10).unwrap();
        assert_eq!(
            switch_stream.poll_next_unpin(&mut cx),
            Poll::Ready(Some(10))
        );
        assert_outer_not_polled(); // Outer stream didn't change so shouldn't be polled
        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_not_polled(); // Outer stream didn't change so shouldn't be polled

        tx_inner1.send(20).unwrap();
        assert_eq!(
            switch_stream.poll_next_unpin(&mut cx),
            Poll::Ready(Some(20))
        );
        assert_outer_not_polled();

        tx.send(
            BroadcastStream::new(rx_inner2)
                .map(|r: Result<_, _>| r.unwrap())
                .boxed(),
        )
        .await
        .unwrap();

        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_polled();

        // We expect trying to send to the first inner stream to fail because
        // rx_inner1 should have been dropped by SwitchStream once we started
        // listening to rx_inner2.
        matches!(tx_inner1.send(30), Err(SendError(_)));
        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_not_polled(); // Outer stream didn't change so shouldn't be polled

        // This should not cause the result stream to terminate.
        // We only terminate on the outer stream terminating.
        drop(tx_inner2);
        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Pending);
        assert_outer_not_polled(); // Outer stream didn't change so shouldn't be polled

        tx.send(
            BroadcastStream::new(rx_inner3)
                .map(|r: Result<_, _>| r.unwrap())
                .boxed(),
        )
        .await
        .unwrap();

        tx_inner3.send(100).unwrap();
        assert_eq!(
            switch_stream.poll_next_unpin(&mut cx),
            Poll::Ready(Some(100))
        );
        assert_outer_polled();

        tx_inner3.send(110).unwrap();
        assert_eq!(
            switch_stream.poll_next_unpin(&mut cx),
            Poll::Ready(Some(110))
        );
        assert_outer_not_polled(); // Outer stream didn't change so shouldn't be polled

        drop(tx);
        assert_eq!(switch_stream.poll_next_unpin(&mut cx), Poll::Ready(None));
        assert_outer_polled();
    }

    #[tokio::test]
    async fn test_inner_not_polled_twice_after_termination() {
        let inner_polled = Arc::new(Mutex::new(false));

        let take_inner_polled = || -> bool {
            let mut guard = inner_polled.lock();
            std::mem::replace(&mut guard, false)
        };

        let assert_inner_polled = || assert!(take_inner_polled());
        let assert_inner_not_polled = || assert!(!take_inner_polled());

        let first_inner = MockStream {
            inner: stream::once(future::ready(1)),
            polled: Arc::clone(&inner_polled),
        };

        // Outer stream consists of first_inner which emits one value and then completes, but never yields any further streams and is permanently
        // pending for the 2nd stream.
        let outer_stream =
            stream::once(future::ready(first_inner)).chain(future::pending().into_stream());

        let mut stream = FlattenSwitch::new(outer_stream);

        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_inner_polled();
        assert_pending!(stream.poll_next_unpin(&mut cx));
        assert_inner_polled();
        assert_pending!(stream.poll_next_unpin(&mut cx));
        assert_inner_not_polled();
    }
}
