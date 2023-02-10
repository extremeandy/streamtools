use crate::Stream;
use futures::{Future, StreamExt};
use tokio::time::{Duration, Instant, Sleep};

use std::pin::Pin;
use std::task::{self, ready, Poll};

use pin_project_lite::pin_project;

pin_project! {
    /// Stream for the [`throttle_last`](crate::StreamTools::throttle_last) method.
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct ThrottleLast<S> {
        #[pin]
        delay: Sleep,
        duration: Duration,

        // Set to true when `delay` has returned ready, but `stream` hasn't.
        has_delayed: bool,

        // The stream to throttle
        #[pin]
        inner: Option<S>,
    }
}

impl<S> ThrottleLast<S> {
    pub(super) fn new(duration: Duration, stream: S) -> Self {
        Self {
            delay: tokio::time::sleep_until(Instant::now() + duration),
            duration,
            has_delayed: true,
            inner: Some(stream),
        }
    }
}

impl<S> Stream for ThrottleLast<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let Some(mut inner) = this.inner.as_mut().as_pin_mut() else {
            // Last time we polled, the inner stream terminated, but we yielded a value.
            // If we are here then it's time to terminate.
            return Poll::Ready(None)
        };

        let dur = *this.duration;

        if !*this.has_delayed && !dur.is_zero() {
            ready!(this.delay.as_mut().poll(cx));
            *this.has_delayed = true;
        }

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
            Some(value) => {
                if !dur.is_zero() {
                    this.delay.reset(Instant::now() + dur);
                }

                *this.has_delayed = false;

                Poll::Ready(Some(value))
            }
            None => match this.inner.as_pin_mut() {
                Some(_) => Poll::Pending, // The stream didn't terminate yet, so we must be pending
                None => Poll::Ready(None), // The stream did terminate and there was no value seen, so we are done.
            },
        }
    }
}

#[cfg(feature = "test-util")]
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;

    use crate::{test_util::delay_items, StreamTools, ThrottleLast};

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn test_throttle_last() {
        let delays = vec![
            0, 1000, 2000, 2999, 3000, 3001, 4000, 7000, 8000, 8999, 9500, 10500, 15001, 15500,
        ]
        .into_iter()
        .map(|delay_ms| (Duration::from_millis(delay_ms), delay_ms));
        let stream = delay_items(delays);

        let results = ThrottleLast::new(Duration::from_millis(1500), stream)
            .record_delay()
            .collect::<Vec<_>>()
            .await;

        let expected_results = vec![
            (Duration::ZERO, 0),
            (Duration::from_millis(1500), 1000),
            (Duration::from_millis(3000), 3000),
            (Duration::from_millis(4500), 4000),
            (Duration::from_millis(7000), 7000),
            (Duration::from_millis(8500), 8000),
            (Duration::from_millis(10000), 9500),
            (Duration::from_millis(11500), 10500),
            (Duration::from_millis(15001), 15001),
            (Duration::from_millis(16501), 15500),
        ];

        assert_eq!(expected_results, results);
    }
}
