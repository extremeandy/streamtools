use futures::Stream;
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for the [`record_delay`](crate::StreamTools::record_delay) method.
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct RecordDelay<S> {
        start: tokio::time::Instant,
        #[pin]
        inner: S
    }
}

impl<S> RecordDelay<S> {
    pub(super) fn new(stream: S) -> Self {
        let now = tokio::time::Instant::now();
        Self {
            start: now,
            inner: stream,
        }
    }
}

impl<S> Stream for RecordDelay<S>
where
    S: Stream,
{
    type Item = (std::time::Duration, S::Item);

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.project();
        this.inner
            .poll_next(cx)
            .map(move |value| value.map(|value| (this.start.elapsed(), value)))
    }
}
