//! Implementation of the `StreamTools::merge_join_by` combinator.

use either_or_both::EitherOrBoth;
use futures::stream::Fuse;
use futures::Stream;
use futures::StreamExt;
use pin_project_lite::pin_project;
use std::{
    cmp::{self, Ordering},
    pin::Pin,
    task::{Context, Poll},
};

pin_project! {
  /// Stream for [`StreamTools::merge_join_by()`].
  #[must_use = "streams do nothing unless polled"]
  pub (crate) struct MergeJoinBy<L, R, F>
  where
    L: Stream,
    R: Stream,
    // Not allowing any funny business with `FnMut`.
    F: Fn(&L::Item, &R::Item) -> Ordering
  {
    #[pin]
    left: Fuse<L>,
    #[pin]
    right: Fuse<R>,
    left_queued: Option<L::Item>,
    right_queued: Option<R::Item>,
    comparison: F,
  }
}

impl<L, R, F> MergeJoinBy<L, R, F>
where
    L: Stream,
    R: Stream,
    F: Fn(&L::Item, &R::Item) -> Ordering,
{
    pub(crate) fn new(left_stream: L, right_stream: R, comparison: F) -> Self {
        Self {
            // Fusing the streams because they might not be the same length.
            left: left_stream.fuse(),
            right: right_stream.fuse(),
            left_queued: None,
            right_queued: None,
            comparison,
        }
    }
}

// Implementation inspired by `futures::Zip`
impl<L, R, F> Stream for MergeJoinBy<L, R, F>
where
    L: Stream,
    R: Stream,
    // Not allowing any funny business with `FnMut`.
    F: Fn(&L::Item, &R::Item) -> Ordering,
{
    type Item = EitherOrBoth<L::Item, R::Item>;

    // Implementation is inspired by `Zip` in the `futures` crate:
    // https://docs.rs/futures-util/0.3.31/src/futures_util/stream/stream/zip.rs.html#71-102
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // Poll the streams as necessary.
        if this.left_queued.is_none() {
            match this.left.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => *this.left_queued = Some(item),
                Poll::Ready(None) | Poll::Pending => {}
            }
        }
        if this.right_queued.is_none() {
            match this.right.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => *this.right_queued = Some(item),
                Poll::Ready(None) | Poll::Pending => {}
            }
        }

        // The actual logic
        if this.left_queued.is_some() && this.right_queued.is_some() {
            // This is the non-trivial case where we actually do a comparison.
            match (this.comparison)(
                this.left_queued.as_ref().unwrap(),
                this.right_queued.as_ref().unwrap(),
            ) {
                Ordering::Less => {
                    let just_left = EitherOrBoth::Left(this.left_queued.take().unwrap());
                    Poll::Ready(Some(just_left))
                }
                Ordering::Equal => {
                    let both = EitherOrBoth::Both(
                        this.left_queued.take().unwrap(),
                        this.right_queued.take().unwrap(),
                    );
                    Poll::Ready(Some(both))
                }
                Ordering::Greater => {
                    let just_right = EitherOrBoth::Right(this.right_queued.take().unwrap());
                    Poll::Ready(Some(just_right))
                }
            }
        } else if this.left_queued.is_some() {
            if this.right.is_done() {
                let just_left = EitherOrBoth::Left(this.left_queued.take().unwrap());
                Poll::Ready(Some(just_left))
            } else {
                Poll::Pending // Still waiting for the other one.
            }
        } else if this.right_queued.is_some() {
            if this.left.is_done() {
                let just_right = EitherOrBoth::Right(this.right_queued.take().unwrap());
                Poll::Ready(Some(just_right))
            } else {
                Poll::Pending // Still waiting for the other one.
            }
        }
        // Both queueds are None from here on.
        else if this.left.is_done() && this.right.is_done() {
            // All done
            Poll::Ready(None)
        } else {
            // Waiting for both, or only one if the other stream is done already.
            Poll::Pending
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_queued_len = usize::from(self.left_queued.is_some());
        let right_queued_len = usize::from(self.right_queued.is_some());

        let (left_lower, left_upper) = self.left.size_hint();
        let (right_lower, right_upper) = self.right.size_hint();

        let left_total_lower = left_lower.saturating_add(left_queued_len);
        let right_total_lower = right_lower.saturating_add(right_queued_len);

        // Achieved when all the comparisons turn out equal and we just yield Boths.
        let lower = cmp::max(left_total_lower, right_total_lower);

        let upper = match (left_upper, right_upper) {
            (Some(l_upper), Some(r_upper)) => {
                // The upper limit is reached when none of the comparisons turn out equal.
                // I.e. we never yield Both.
                // The limit is then just the sum of upper limits,
                // with the possibility of overflowing usize.
                l_upper
                    .checked_add(left_queued_len)
                    .and_then(|left_total_upper| left_total_upper.checked_add(r_upper))
                    .and_then(|all_but_right_queue| {
                        all_but_right_queue.checked_add(right_queued_len)
                    })
            }
            // Either or both of the limits are unknown,
            // thus can't compute their sums either.
            _ => None,
        };

        (lower, upper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::StreamTools;
    use futures::{executor::block_on_stream, stream};
    use quickcheck::TestResult;
    use quickcheck_macros::quickcheck;
    use std::collections::BTreeSet;

    #[quickcheck]
    fn merge_of_sorteds_is_sorted(left: BTreeSet<isize>, right: BTreeSet<isize>) -> TestResult {
        // BtreeSets are conveniently already sorted.
        // They don't contain duplicates though so that's not covered.
        let left_stream = stream::iter(left);
        let right_stream = stream::iter(right);

        let stream = left_stream.merge_join_by(right_stream, Ord::cmp);

        let sorted = block_on_stream(stream)
            // Remove the EitherOrs for sortedness check.
            .flat_map(|either_or_both| {
                either_or_both.into_iter() // Rather convenient I must admit.
            })
            .is_sorted();

        TestResult::from_bool(sorted)
    }

    #[quickcheck]
    fn size_hints_dont_lie(left: Vec<isize>, right: Vec<isize>) -> bool {
        let expected_lower = cmp::max(left.len(), right.len());
        let expected_upper = left.len().checked_add(right.len()); // Sum

        let left_stream = stream::iter(left);
        let right_stream = stream::iter(right);

        let stream = left_stream.merge_join_by(right_stream, Ord::cmp);

        let (lower, upper) = stream.size_hint();

        assert_eq!(expected_lower, lower);
        assert_eq!(expected_upper, upper);

        let actual_size = block_on_stream(stream).count();

        lower <= actual_size && upper.is_none_or(|limit| actual_size <= limit)
    }
}
