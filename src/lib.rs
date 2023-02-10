#![warn(missing_docs)]
#![crate_name = "streamtools"]

//! Additional stream combinators.
//!
//! ## Feature flags
//!
//! - `tokio-time`: Enables combinators which depend on the tokio crate and its time feature, in particular:
//!   - [`sample_by_duration`](crate::StreamTools::sample_by_duration)
//!   - [`sample_by_interval`](crate::StreamTools::sample_by_interval)
#![doc(html_root_url = "https://docs.rs/streamtools/0.6.0/")]

use futures::Stream;

mod fast_forward;
mod flatten_switch;
mod outer_waker;
mod sample;

#[cfg(test)]
mod test_util;

pub use fast_forward::*;
pub use flatten_switch::*;
pub use sample::*;

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
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio-time")))]
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
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio-time")))]
    fn sample_by_interval(
        self,
        interval: tokio::time::Interval,
    ) -> Sample<Self, tokio_stream::wrappers::IntervalStream>
    where
        Self: Sized,
    {
        let sampler = tokio_stream::wrappers::IntervalStream::new(interval);
        Self::sample(self, sampler)
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
