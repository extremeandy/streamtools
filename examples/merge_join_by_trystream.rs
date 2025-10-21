//! How to use `StreamTools::merge_join_by()` with a `TryStream`,
//! that is, a `Stream` of `Result`s.

use either_or_both::EitherOrBoth::{self, Both};
use futures::stream::{self, StreamExt, TryStreamExt};
use std::cmp::Ordering;
use streamtools::StreamTools;

#[tokio::main]
async fn main() {
    let left = stream::iter(vec![Ok(1), Ok(3), Err(4), Ok(5)]);
    let right = stream::iter(vec![Ok(2), Ok(3), Err(3)]);

    // Instead of using your comparison function directly, you can embed it into a closure that
    // gives priority to errors. Then afterwards you can use usual try_* combinators in `futures::stream::TryStreamExt`.
    let stream = left
        .merge_join_by(right, |left_result, right_result| {
            match (left_result, right_result) {
                // Use the actual comparison function only if both are Ok.
                (Ok(asset), Ok(foreign_asset)) => Ord::cmp(asset, foreign_asset),
                // In the error cases return such ordering that the error(s) will be forwarded asap,
                // and so we can short circuit faster.
                (Err(_), Ok(_)) => Ordering::Less,
                (Ok(_), Err(_)) => Ordering::Greater,
                (Err(_), Err(_)) => Ordering::Equal,
            }
        })
        // Flip the results from inside the EitherOrs.
        // EitherOrBoth has a convenient method for just this.
        .map(EitherOrBoth::<Result<_, _>, Result<_, _>>::transpose);

    // Run the stream with short circuiting.
    let result: Result<Vec<_>, EitherOrBoth<_>> = stream.try_collect().await;

    assert_eq!(result, Err(Both(4, 3)));
}
