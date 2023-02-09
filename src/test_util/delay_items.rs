use std::time::Duration;

use futures::{stream, Stream, StreamExt};

/// Creates a stream from an iterator where items are delayed by the specified amount
pub fn delay_items<T>(items: impl IntoIterator<Item = (Duration, T)>) -> impl Stream<Item = T> {
    let initial_time = tokio::time::Instant::now();
    stream::iter(items).flat_map(move |(duration, value)| {
        let delayed = async move {
            tokio::time::sleep_until(initial_time + duration).await;
            value
        };

        stream::once(delayed)
    })
}
