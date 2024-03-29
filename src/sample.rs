use futures::{stream::FusedStream, Stream, StreamExt};
use pin_project_lite::pin_project;
use std::{
    pin::Pin,
    task::{self, Poll},
};

pin_project! {
    /// Stream for the [`sample`](crate::StreamTools::sample) method.
    #[must_use = "streams do nothing unless polled"]
    pub struct Sample<T, S>
    where
        T: Stream,
    {
        #[pin]
        inner: Option<T>,

        #[pin]
        sampler: S,

        value: Option<T::Item>
    }
}

impl<T, S> Sample<T, S>
where
    T: Stream,
{
    pub(super) fn new(stream: T, sampler: S) -> Self
    where
        S: Stream,
    {
        Self {
            inner: Some(stream),
            sampler,
            value: None,
        }
    }
}

impl<T: Stream, S: Stream> Stream for Sample<T, S> {
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let Some(mut inner) = this.inner.as_mut().as_pin_mut() else {
            // Last time we polled, the inner stream terminated, but we yielded a value.
            // If we are here then it's time to terminate.
            return Poll::Ready(None)
        };

        // Fast forward to the latest value on the inner stream
        while let Poll::Ready(ready) = inner.poll_next_unpin(cx) {
            match ready {
                Some(value) => {
                    *this.value = Some(value);
                }
                None => {
                    // Inner stream terminated. We don't immediately return Poll::Ready(None)
                    // because if the sampler is also ready we should return the last seen value.
                    this.inner.set(None);
                    break;
                }
            }
        }

        // Make sure we poll the sampler until pending so that we register the waker. We must
        // not returning Poll::Pending unless we have definitely received Poll::Pending from the sampler,
        // otherwise we will not be woken by the sampler.
        while let Poll::Ready(ready) = this.sampler.poll_next_unpin(cx) {
            match ready {
                Some(_) => match this.value.take() {
                    Some(value) => {
                        return Poll::Ready(Some(value)); // Inner stream had value -> we yield Some
                    }
                    None => {
                        // Pending on the inner stream -> we will return pending. We just need to make sure we poll
                        // the sampler until pending so we receive a wakeup.
                        continue;
                    }
                },
                None => {
                    // Drop the inner stream. If polled again we immediately return Poll::Ready(None)
                    this.inner.set(None);

                    return match this.value.take() {
                        Some(value) => Poll::Ready(Some(value)), // Sampler has terminated but we still have one last value to emit.
                        None => Poll::Ready(None), // Sampler stream terminated and no value waiting -> we terminate immediately
                    };
                }
            }
        }

        // Inner stream terminated, so we terminate
        if this.inner.is_none() {
            return Poll::Ready(None);
        }

        Poll::Pending // Sampler is Pending and inner has not terminated
    }
}

impl<T, S> FusedStream for Sample<T, S>
where
    T: Stream,
    S: Stream,
{
    fn is_terminated(&self) -> bool {
        self.inner.is_none()
    }
}

impl<T, S> std::fmt::Debug for Sample<T, S>
where
    T: Stream + std::fmt::Debug,
    T::Item: std::fmt::Debug,
    S: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("inner", &self.inner)
            .field("sampler", &self.sampler)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::future;

    use futures::{stream, SinkExt, StreamExt};

    use tokio_test::{assert_pending, assert_ready_eq};

    #[cfg(feature = "test-util")]
    use tokio_stream::wrappers::IntervalStream;

    #[cfg(feature = "test-util")]
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_sample() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx, rx) = futures::channel::mpsc::unbounded();
        let (mut tx_sampler, rx_sampler) = futures::channel::mpsc::unbounded();

        let mut stream = Sample::new(rx, rx_sampler);

        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(1).await.unwrap();
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx_sampler.send(()).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        // Now we sample but there are no new values on the underlying so the stream emits nothing
        tx_sampler.send(()).await.unwrap();
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(2).await.unwrap();
        tx.send(3).await.unwrap();
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx_sampler.send(()).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(3)); // Note we skipped 2 because when we sample, the last value was 3.
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(4).await.unwrap();
        drop(tx_sampler); // Sampler terminated
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(4)); // Sampler terminates -> immediately yield last value
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None); // Once last value is yielded, stream is terminated
    }

    #[tokio::test]
    async fn test_sample_underlying_terminates() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx, rx) = futures::channel::mpsc::unbounded();
        let (mut tx_sampler, rx_sampler) = futures::channel::mpsc::unbounded();

        let mut stream = Sample::new(rx, rx_sampler);

        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx.send(1).await.unwrap();
        assert_pending!(stream.poll_next_unpin(&mut cx));

        tx_sampler.send(()).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1));
        assert_pending!(stream.poll_next_unpin(&mut cx));

        drop(tx); // Terminate the underlying stream
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None); // Underlying has terminated
    }

    #[tokio::test]
    async fn test_sample_underlying_terminates_but_sample_yields() {
        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        let (mut tx_sampler, rx_sampler) = futures::channel::mpsc::unbounded();

        let mut stream = Sample::new(stream::once(future::ready(1)), rx_sampler);

        tx_sampler.send(()).await.unwrap();
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), Some(1)); // We take the last value on the inner that was ready when the sampler yielded ...
        assert_ready_eq!(stream.poll_next_unpin(&mut cx), None); // ... before we finally terminate.
    }

    #[cfg(feature = "test-util")]
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn test_sample_with_interval() {
        use crate::test_util::delay_items;
        use crate::StreamTools;

        let sampler = IntervalStream::new(tokio::time::interval(Duration::from_millis(1500)));

        let delays = vec![
            0, 1000, 2000, 2999, 3000, 3001, 4000, 7000, 8000, 8999, 9500, 10500, 15001, 15500,
        ]
        .into_iter()
        .map(|delay_ms| (Duration::from_millis(delay_ms), delay_ms));
        let stream = delay_items(delays);

        let results = Sample::new(stream, sampler)
            .record_delay()
            .collect::<Vec<_>>()
            .await;

        let expected_results = vec![
            (Duration::ZERO, 0),
            (Duration::from_millis(1500), 1000),
            (Duration::from_millis(3000), 3000),
            (Duration::from_millis(4500), 4000),
            (Duration::from_millis(7500), 7000),
            (Duration::from_millis(9000), 8999),
            (Duration::from_millis(10500), 10500),
            // Note the tick at 15500 is lost: the sampler isn't ready yet, and then the stream terminates.
        ];

        assert_eq!(expected_results, results);
    }
}
