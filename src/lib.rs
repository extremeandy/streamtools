use futures::Stream;

mod fast_forward;
mod flatten_switch;
mod stream_ext;

pub use fast_forward::*;
pub use flatten_switch::*;
pub use stream_ext::*;

// Just a helper function to ensure the streams we're returning all have the
// right implementations.
pub(crate) fn assert_stream<T, S>(stream: S) -> S
where
    S: Stream<Item = T>,
{
    stream
}
