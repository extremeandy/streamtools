use futures::Stream;

use super::FlattenSwitch;
use crate::assert_stream;

/// An extension trait for the [`Stream`] trait that provides a variety of
/// convenient combinator functions.
///
/// [`Stream`]: crate::Stream
/// [futures]: https://docs.rs/futures
/// [futures-StreamExt]: https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html
pub trait StreamExt: Stream {
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
