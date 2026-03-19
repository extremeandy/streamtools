//! Implementation of `StreamTools::try_count()`.

use futures::TryStream;
use pin_project_lite::pin_project;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pin_project! {
    /// Future for [`StreamTools::try_count()`].
    #[must_use = "streams do nothing unless polled"]
    pub (crate) struct TryCount<S>
    where
        S: TryStream,
    {
        #[pin]
        stream: S,
        accumulator: usize,
    }
}

impl<S> TryCount<S>
where
    S: TryStream,
{
    pub(crate) fn new(try_stream: S) -> Self {
        Self {
            stream: try_stream,
            // Start counting at 0.
            accumulator: 0,
        }
    }
}

// Just need to implement Future as this consumes the Stream.
impl<S> Future for TryCount<S>
where
    S: TryStream,
{
    type Output = Result<usize, S::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.stream.as_mut().try_poll_next(cx) {
                // Count Ok and loop.
                Poll::Ready(Some(Ok(_))) => *this.accumulator += 1,
                // Short circuit on errors.
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                // Reached the end. Return the count.
                Poll::Ready(None) => return Poll::Ready(Ok(*this.accumulator)),
                // Nothing new at this moment, back to the executor.
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
