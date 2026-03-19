use std::time::Duration;

use futures::{FutureExt, Stream, StreamExt, stream};

/// Creates a stream from an iterator where items are delayed by the specified amount
pub fn delay_items<T>(items: impl IntoIterator<Item = (Duration, T)>) -> impl Stream<Item = T> {
    let start_time = tokio::time::Instant::now();
    stream::iter(items).flat_map(move |(duration, value)| {
        let delayed = tokio::time::sleep_until(start_time + duration).map(|_| value);
        stream::once(delayed)
    })
}
