#![warn(missing_docs)]
#![crate_name = "streamtools"]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

//! Additional stream combinators.
//!
//! ## Feature flags
//!
//! - `tokio-time`: Enables combinators which depend on the tokio crate and its time feature, in particular:
//!   - [`sample_by_duration`](crate::StreamTools::sample_by_duration)
//!   - [`sample_by_interval`](crate::StreamTools::sample_by_interval)
//! - `test-util`: Exposes utilities for testing streams, in particular:
//!   - [`delay_items`](crate::test_util::delay_items)
//!   - [`record_delay`](crate::StreamTools::record_delay)
#![doc(html_root_url = "https://docs.rs/streamtools/0.7.3/")]

use futures::{stream::Map, Stream};

mod fast_forward;
mod flatten_switch;
mod outer_waker;
mod sample;

#[cfg(feature = "tokio-time")]
mod throttle_last;

#[cfg(feature = "test-util")]
mod record_delay;

/// Utilities for testing streams
#[cfg(feature = "test-util")]
pub mod test_util;

pub use fast_forward::*;
pub use flatten_switch::*;
pub use sample::*;

#[cfg(feature = "tokio-time")]
pub use throttle_last::*;

#[cfg(feature = "test-util")]
pub use record_delay::*;

/// An extension trait for the [`Stream`] trait that provides a variety of
/// convenient combinator functions.
///
/// [`Stream`]: crate::Stream
/// [futures]: https://docs.rs/futures
/// [futures-StreamExt]: https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html
pub trait StreamTools: Stream {
    /// Fast-forwards to the latest value on the underlying stream by polling the underyling until it is [`Pending`].
    ///
    /// When the underlying stream terminates, this stream will yield the last value on the underlying
    /// stream, if it has not already been yielded.
    ///
    /// This behaves like a [`WatchStream`] but can be applied to arbitrary streams without requiring a channel.
    ///
    /// [`Pending`]: std::task::Poll#variant.Pending
    /// [`WatchStream`]: https://docs.rs/tokio-stream/latest/tokio_stream/wrappers/struct.WatchStream.html
    fn fast_forward(self) -> FastForward<Self>
    where
        Self: Sized,
    {
        let stream = FastForward::new(self);
        assert_stream::<Self::Item, _>(stream)
    }

    /// Flattens a stream of streams into just one continuous stream. Polls only the latest
    /// inner stream.
    ///
    /// The stream terminates when the outer stream terminates.
    ///
    /// This mirrors the behaviour of the [Switch](https://reactivex.io/documentation/operators/switch.html) operator in [ReactiveX](https://reactivex.io/).
    fn flatten_switch(self) -> FlattenSwitch<Self>
    where
        Self::Item: Stream,
        Self: Sized,
    {
        let stream = FlattenSwitch::new(self);
        assert_stream::<<Self::Item as Stream>::Item, _>(stream)
    }

    /// Maps a stream like [`StreamExt::map`] but flattens nested [Stream]s using [`flatten_switch`](Self::flatten_switch).
    ///
    /// [`StreamExt::map`]: futures::StreamExt
    fn flat_map_switch<U, F>(self, f: F) -> FlattenSwitch<Map<Self, F>>
    where
        U: Stream + Unpin,
        F: FnMut(Self::Item) -> U,
        Self::Item: Stream,
        Self: Sized,
    {
        let stream = FlattenSwitch::new(futures::StreamExt::map(self, f));
        assert_stream::<U::Item, _>(stream)
    }

    /// Samples values from the stream when the sampler yields.
    ///
    /// The stream terminates when either the input stream or the sampler stream terminate.
    ///
    /// If no value was seen on the input stream when the sampler yields, nothing will be yielded.
    ///
    /// This mirrors the behaviour of the [Sample](https://reactivex.io/documentation/operators/sample.html) operator in [ReactiveX](https://reactivex.io/).
    fn sample<S: Stream>(self, sampler: S) -> Sample<Self, S>
    where
        Self: Sized,
    {
        let stream = Sample::new(self, sampler);
        assert_stream(stream)
    }

    /// Samples values from the stream at intervals of length `duration`. This is a convenience method which invokes [`sample_by_interval`](StreamTools::sample_by_interval).
    ///
    /// The stream terminates when the input stream terminates.
    ///
    /// Uses the default [`MissedTickBehavior`] to create an [`Interval`]. If another is needed, then configure it on an [`Interval`] and
    /// use [`sample_by_interval`](StreamTools::sample_by_interval) instead of this method.
    ///
    /// This mirrors the behaviour of the [Sample](https://reactivex.io/documentation/operators/sample.html) operator in [ReactiveX](https://reactivex.io/).
    ///
    /// [`MissedTickBehavior`]: https://docs.rs/tokio/latest/tokio/time/enum.MissedTickBehavior.html
    /// [`Interval`]: https://docs.rs/tokio/latest/tokio/time/struct.Interval.html
    #[cfg(feature = "tokio-time")]
    fn sample_by_duration(
        self,
        duration: std::time::Duration,
    ) -> Sample<Self, tokio_stream::wrappers::IntervalStream>
    where
        Self: Sized,
    {
        self.sample_by_interval(tokio::time::interval(duration))
    }

    /// Samples values from the stream at intervals. This is a convenience method which invokes [`sample`](StreamTools::sample).
    ///
    /// The stream terminates when the input stream terminates.
    ///
    /// This mirrors the behaviour of the [Sample](https://reactivex.io/documentation/operators/sample.html) operator in [ReactiveX](https://reactivex.io/).
    #[cfg(feature = "tokio-time")]
    fn sample_by_interval(
        self,
        interval: tokio::time::Interval,
    ) -> Sample<Self, tokio_stream::wrappers::IntervalStream>
    where
        Self: Sized,
    {
        let sampler = tokio_stream::wrappers::IntervalStream::new(interval);
        self.sample(sampler)
    }

    /// Throttles values from the stream at intervals of length `duration`, skipping all but the last value seen in each interval.
    ///
    /// Note that this behaves exactly the same as applying [`fast_forward`] followed by tokio's [`throttle`].
    ///
    /// The stream terminates after the input stream terminates and any pending timeout expires for the throttling.
    ///
    /// [`fast_forward`]: Self::fast_forward
    /// [`throttle`]: https://docs.rs/tokio-stream/latest/tokio_stream/trait.StreamExt.html#method.throttle
    #[cfg(feature = "tokio-time")]
    fn throttle_last<'a>(self, duration: std::time::Duration) -> ThrottleLast<Self>
    where
        Self: Sized + Send + 'a,
    {
        ThrottleLast::new(duration, self)
    }

    /// Records the duration relative to the time this method was called at which each
    /// item is yielded from the stream.
    #[cfg(feature = "test-util")]
    fn record_delay(self) -> RecordDelay<Self>
    where
        Self: Sized,
    {
        RecordDelay::new(self)
    }
}

impl<T: Stream> StreamTools for T {}

// Just a helper function to ensure the streams we're returning all have the
// right implementations.
pub(crate) fn assert_stream<T, S>(stream: S) -> S
where
    S: Stream<Item = T>,
{
    stream
}
