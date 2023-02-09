use futures::Stream;

use super::{FastForward, FlattenSwitch};
use crate::assert_stream;

/// An extension trait for the [`Stream`] trait that provides a variety of
/// convenient combinator functions.
///
/// [`Stream`]: crate::Stream
/// [futures]: https://docs.rs/futures
/// [futures-StreamExt]: https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html
pub trait StreamExt: Stream {
    /// Fast-forwards to the latest value on the underlying stream by polling the underyling until it is [`Pending`].
    ///
    /// When the underlying stream terminates, this stream will yield the last value on the underlying
    /// stream, if it has not already been yielded.
    ///
    /// This behaves like a [`WatchStream`] but can be applied to arbitrary streams without requiring a channel.
    ///
    /// [`Pending`]: std::task::Poll#variant.Pending
    /// [`WatchStream`]: tokio_stream::wrappers::WatchStream
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
}

impl<T: Stream> StreamExt for T {}
