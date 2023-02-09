#![warn(missing_docs)]
#![crate_name = "streamtools"]
//#![cfg_attr(not(feature = "use_std"), no_std)]

//! Additional stream combinators.
//!
//! ## Feature flags
//!
//! - `tokio-time`: Enables combinators which depend on the tokio crate and its time feature, in particular:
//!   - [`sample_by_duration`](crate::StreamExt::sample_by_duration)
//!   - [`sample_by_interval`](crate::StreamExt::sample_by_interval)
#![doc(html_root_url = "https://docs.rs/streamtools/0.4.0/")]

use futures::Stream;

mod fast_forward;
mod flatten_switch;
mod outer_waker;
mod sample;
mod stream_ext;

#[cfg(test)]
mod test_util;

pub use fast_forward::*;
pub use flatten_switch::*;
pub use sample::*;
pub use stream_ext::*;

// Just a helper function to ensure the streams we're returning all have the
// right implementations.
pub(crate) fn assert_stream<T, S>(stream: S) -> S
where
    S: Stream<Item = T>,
{
    stream
}
