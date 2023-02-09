use futures::Stream;
use pin_project_lite::pin_project;

pub fn record_delay<S>(stream: S) -> RecordDelay<S>
where
    S: Stream,
{
    let now = tokio::time::Instant::now();
    RecordDelay {
        from: now,
        inner: stream,
    }
}

pub trait StreamExt: Stream {
    fn record_delay(self) -> RecordDelay<Self>
    where
        Self: Sized,
    {
        record_delay(self)
    }
}

impl<S> StreamExt for S where S: Stream {}

pin_project! {
    #[derive(Debug)]
    pub struct RecordDelay<S> {
        from: tokio::time::Instant,
        #[pin]
        inner: S
    }
}

impl<S> Stream for RecordDelay<S>
where
    S: Stream,
{
    type Item = (tokio::time::Duration, S::Item);

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.project();
        this.inner
            .poll_next(cx)
            .map(move |value| value.map(|value| (this.from.elapsed(), value)))
    }
}
