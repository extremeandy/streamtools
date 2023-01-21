use futures::{future::Either, stream, Stream, StreamExt};
use std::{convert, future, hash::Hash, iter};

use crate::SnapshotWithUpdates;

impl<Key, Value, Snapshot, Updates> SnapshotWithUpdates<Snapshot, Updates>
where
    Key: Clone + Hash + Eq,
    Value: Clone,
    Snapshot: IntoIterator<Item = (Key, Value)>,
    Updates: Stream,
    Updates::Item: IntoIterator<Item = (Key, Option<Value>)>,
{
    pub fn join_with_optional_parent<ParentKey, ParentValue, ParentSnapshot, ParentUpdates>(
        self,
        other: SnapshotWithUpdates<ParentSnapshot, ParentUpdates>,
        f: impl FnMut(&Key) -> ParentKey,
    ) -> SnapshotWithUpdates<
        impl IntoIterator<Item = ((ParentKey, Key), (Option<ParentValue>, Value))>,
        impl Stream<
            Item = impl IntoIterator<Item = ((ParentKey, Key), Option<(Option<ParentValue>, Value)>)>,
        >,
    >
    where
        ParentKey: Clone + Hash + Eq,
        Snapshot::Item: Clone,
        ParentSnapshot: IntoIterator<Item = (ParentKey, ParentValue)>,
        ParentValue: Clone,
        ParentUpdates: Stream,
        ParentUpdates::Item: IntoIterator<Item = (ParentKey, Option<ParentValue>)>,
    {
        let (snapshot, updates) = self.into_inner();
        let (parent_snapshot, parent_updates) = other.into_inner();

        let initial_state = JoinWithParentState::new(parent_snapshot, snapshot, f);

        let initial_snapshot = initial_state.snapshot.clone().into_iter().flat_map(
            |(parent_key, (parent, inner_key_to_inner))| {
                inner_key_to_inner
                    .into_iter()
                    .map(move |(key, value)| ((parent_key.clone(), key), (parent.clone(), value)))
            },
        );

        let parent_updates = parent_updates
            .map(Either::Left)
            .map(Some)
            .chain(stream::once(future::ready(None)));

        let updates = updates
            .map(Either::Right)
            .map(Some)
            .chain(stream::once(future::ready(None)));

        let merged = tokio_stream::StreamExt::merge(parent_updates, updates);
        let combined_updates = merged
            .scan(initial_state, |state, next| {
                let Some(next) = next else {
                    return future::ready(None);
                };

                let result = state.ingest_update_optional_parent(next);

                let result = if result.len() == 0 {
                    None
                } else {
                    Some(result)
                };

                future::ready(Some(result))
            })
            .flat_map(stream::iter);

        SnapshotWithUpdates::new(initial_snapshot, combined_updates)
    }

    pub fn join_with_parent<ParentKey, ParentValue, ParentSnapshot, ParentUpdates>(
        self,
        other: SnapshotWithUpdates<ParentSnapshot, ParentUpdates>,
        f: impl FnMut(&Key) -> ParentKey,
    ) -> SnapshotWithUpdates<
        impl IntoIterator<Item = ((ParentKey, Key), (ParentValue, Value))>,
        impl Stream<Item = impl IntoIterator<Item = ((ParentKey, Key), Option<(ParentValue, Value)>)>>,
    >
    where
        ParentKey: Clone + Hash + Eq,
        Snapshot::Item: Clone,
        ParentSnapshot: IntoIterator<Item = (ParentKey, ParentValue)>,
        ParentValue: Clone,
        ParentUpdates: Stream,
        ParentUpdates::Item: IntoIterator<Item = (ParentKey, Option<ParentValue>)>,
    {
        let (snapshot, updates) = self.into_inner();
        let (parent_snapshot, parent_updates) = other.into_inner();

        let initial_state = JoinWithParentState::new(parent_snapshot, snapshot, f);

        let initial_snapshot: Vec<_> = initial_state
            .snapshot
            .clone()
            .into_iter()
            .flat_map(|(parent_key, (parent, inner_key_to_inner))| {
                parent
                    .map(move |parent| {
                        inner_key_to_inner.into_iter().map(move |(key, value)| {
                            ((parent_key.clone(), key), (parent.clone(), value))
                        })
                    })
                    .into_iter()
                    .flat_map(convert::identity)
            })
            .collect();

        let parent_updates = parent_updates
            .map(Either::Left)
            .map(Some)
            .chain(stream::once(future::ready(None)));

        let updates = updates
            .map(Either::Right)
            .map(Some)
            .chain(stream::once(future::ready(None)));

        let merged = tokio_stream::StreamExt::merge(parent_updates, updates);
        let combined_updates = merged
            .scan(initial_state, |state, next| {
                let Some(next) = next else {
                    return future::ready(None);
                };

                let result = state.ingest_update(next);

                let result = if result.len() == 0 {
                    None
                } else {
                    Some(result)
                };

                future::ready(Some(result))
            })
            .flat_map(stream::iter);

        SnapshotWithUpdates::new(initial_snapshot, combined_updates)
    }
}

struct JoinWithParentState<ParentKey, ParentValue, Key, Value, F> {
    snapshot: im::HashMap<ParentKey, (Option<ParentValue>, im::HashMap<Key, Value>)>,
    f: F,
}

impl<ParentKey, ParentValue, Key, Value, F>
    JoinWithParentState<ParentKey, ParentValue, Key, Value, F>
where
    ParentKey: Clone + Hash + Eq,
    ParentValue: Clone,
    Key: Clone + Hash + Eq,
    Value: Clone,
    F: FnMut(&Key) -> ParentKey,
{
    fn new(
        parents: impl IntoIterator<Item = (ParentKey, ParentValue)>,
        children: impl IntoIterator<Item = (Key, Value)>,
        mut f: F,
    ) -> Self {
        use im::HashMap;

        let mut snapshot = parents
            .into_iter()
            .map(|(key, value)| (key, (Some(value), HashMap::new())))
            .collect::<HashMap<_, _>>();

        for (child_key, child_value) in children {
            let parent_key = f(&child_key);
            let (_, children_state) = snapshot
                .entry(parent_key)
                .or_insert_with(|| (None, HashMap::new()));

            children_state.insert(child_key, child_value);
        }

        Self { snapshot, f }
    }

    fn ingest_update<'a>(
        &mut self,
        update: Either<
            impl IntoIterator<Item = (ParentKey, Option<ParentValue>)>,
            impl IntoIterator<Item = (Key, Option<Value>)>,
        >,
    ) -> Vec<((ParentKey, Key), Option<(ParentValue, Value)>)> {
        use im::HashMap;

        match update {
            Either::Left(parents) => parents
                .into_iter()
                .flat_map(|(parent_key, parent)| {
                    let (parent_state, children_state) = self
                        .snapshot
                        .entry(parent_key.clone())
                        .or_insert_with(|| (None, HashMap::new()));

                    let changed = match &parent {
                        Some(left) => {
                            _ = parent_state.insert(left.to_owned());
                            true
                        }
                        None => parent_state.take().is_some(),
                    };

                    if changed {
                        itertools::Either::Left(children_state.clone().into_iter().map(
                            move |(child_key, child)| {
                                let result = parent.as_ref().map(|parent| (parent.clone(), child));

                                ((parent_key.clone(), child_key), result)
                            },
                        ))
                    } else {
                        itertools::Either::Right(iter::empty())
                    }
                })
                .collect(),
            Either::Right(children) => children
                .into_iter()
                .flat_map(|(child_key, child)| {
                    let parent_key = (self.f)(&child_key);

                    let (parent_state, children_state) = self
                        .snapshot
                        .entry(parent_key.clone())
                        .or_insert_with(|| (None, HashMap::new()));

                    let changed = match &child {
                        Some(child) => {
                            children_state.insert(child_key.clone(), child.to_owned());
                            true
                        }
                        None => children_state.remove(&child_key).is_some(),
                    };

                    return changed.then(|| {
                        let result = match (parent_state, child) {
                            (Some(parent), Some(child)) => Some((parent.to_owned(), child)),
                            _ => None,
                        };

                        ((parent_key, child_key), result)
                    });
                })
                .collect(),
        }
    }

    fn ingest_update_optional_parent<'a>(
        &mut self,
        update: Either<
            impl IntoIterator<Item = (ParentKey, Option<ParentValue>)>,
            impl IntoIterator<Item = (Key, Option<Value>)>,
        >,
    ) -> Vec<((ParentKey, Key), Option<(Option<ParentValue>, Value)>)> {
        use im::HashMap;

        match update {
            Either::Left(parents) => parents
                .into_iter()
                .flat_map(|(key, parent)| {
                    let (parent_state, children_state) = self
                        .snapshot
                        .entry(key.clone())
                        .or_insert_with(|| (None, HashMap::new()));

                    match &parent {
                        Some(parent) => {
                            _ = parent_state.insert(parent.to_owned());
                        }
                        None => {
                            _ = parent_state.take();
                        }
                    };

                    children_state
                        .clone()
                        .into_iter()
                        .map(move |(child_key, child)| {
                            let result = (parent.clone(), child);
                            ((key.clone(), child_key), Some(result))
                        })
                })
                .collect(),
            Either::Right(children) => children
                .into_iter()
                .map(|(child_key, child)| {
                    let parent_key = (self.f)(&child_key);

                    let (parent_state, children_state) = self
                        .snapshot
                        .entry(parent_key.clone())
                        .or_insert_with(|| (None, HashMap::new()));

                    match &child {
                        Some(right) => {
                            _ = children_state.insert(child_key.clone(), right.to_owned());
                        }
                        None => {
                            _ = children_state.remove(&child_key);
                        }
                    };

                    let result = child.map(|child| (parent_state.clone(), child));

                    ((parent_key, child_key), result)
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use futures::{channel::mpsc, SinkExt, StreamExt};
    use tokio_test::{assert_pending, assert_ready};

    use super::*;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn test_join_with_parent() {
        type ParentKey = i32;
        type ChildKey = (ParentKey, i64);

        struct ParentEntity {
            key: ParentKey,
            value: &'static str,
        }

        impl ParentEntity {
            fn into_inner(self) -> (ParentKey, &'static str) {
                (self.key, self.value)
            }
        }

        struct ChildEntity {
            key: ChildKey,
            value: &'static str,
        }

        impl ChildEntity {
            fn into_inner(self) -> (ChildKey, &'static str) {
                (self.key, self.value)
            }
        }

        let parent_snapshot = vec![
            ParentEntity {
                key: 1,
                value: "PARENT_1",
            },
            ParentEntity {
                key: 2,
                value: "PARENT_2",
            },
        ]
        .into_iter()
        .map(ParentEntity::into_inner);

        let children_snapshot = vec![
            ChildEntity {
                key: (1, 1011),
                value: "CHILD_1011_PARENT_1",
            },
            ChildEntity {
                key: (2, 1021),
                value: "CHILD_1021_PARENT_2",
            },
            ChildEntity {
                key: (2, 1022),
                value: "CHILD_1022_PARENT_2",
            },
            ChildEntity {
                key: (3, 1031),
                value: "CHILD_1031_PARENT_3",
            },
        ]
        .into_iter()
        .map(ChildEntity::into_inner);

        enum Update {
            Parents(Vec<(ParentKey, Option<&'static str>)>),
            Children(Vec<(ChildKey, Option<&'static str>)>),
        }

        let expected_initial_snapshot = vec![
            ((1, (1, 1011_i64)), ("PARENT_1", "CHILD_1011_PARENT_1")),
            ((2, (2, 1021_i64)), ("PARENT_2", "CHILD_1021_PARENT_2")),
            ((2, (2, 1022_i64)), ("PARENT_2", "CHILD_1022_PARENT_2")),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

        let (mut parent_tx, parent_rx) = mpsc::unbounded();
        let parent_snapshot_with_updates = SnapshotWithUpdates::new(parent_snapshot, parent_rx);

        let (mut child_tx, child_rx) = mpsc::unbounded();
        let children_snapshot_with_updates = SnapshotWithUpdates::new(children_snapshot, child_rx);

        let result = children_snapshot_with_updates
            .join_with_parent(parent_snapshot_with_updates, |child_key| child_key.0);

        let (snapshot, updates) = result.into_inner();
        let mut updates = updates
            .map(|updates| updates.into_iter().collect::<Vec<_>>())
            .boxed();
        let snapshot = snapshot.into_iter().collect::<HashMap<_, _>>();

        assert_eq!(expected_initial_snapshot, snapshot);

        struct UpdateTestCase {
            update: Either<
                Vec<(ParentKey, Option<&'static str>)>,
                Vec<(ChildKey, Option<&'static str>)>,
            >,
            expected_result: Option<Vec<((i32, (i32, i64)), Option<(&'static str, &'static str)>)>>,
        }

        let test_cases = vec![
            UpdateTestCase {
                update: Either::Left(vec![(1, Some("PARENT_1_UPDATE_1"))]),
                expected_result: Some(vec![(
                    (1, (1, 1011)),
                    Some(("PARENT_1_UPDATE_1", "CHILD_1011_PARENT_1")),
                )]),
            },
            UpdateTestCase {
                update: Either::Left(vec![(2, Some("PARENT_2_UPDATE_1"))]),
                expected_result: Some(vec![
                    (
                        (2, (2, 1021)),
                        Some(("PARENT_2_UPDATE_1", "CHILD_1021_PARENT_2")),
                    ),
                    (
                        (2, (2, 1022)),
                        Some(("PARENT_2_UPDATE_1", "CHILD_1022_PARENT_2")),
                    ),
                ]),
            },
            UpdateTestCase {
                update: Either::Left(vec![(3, Some("PARENT_3"))]),
                expected_result: Some(vec![(
                    (3, (3, 1031)),
                    Some(("PARENT_3", "CHILD_1031_PARENT_3")),
                )]),
            },
            UpdateTestCase {
                update: Either::Left(vec![(2, None)]),
                expected_result: Some(vec![((2, (2, 1021)), None), ((2, (2, 1022)), None)]),
            },
            // Deleted twice... it's already deleted so nothing should be emitted.
            UpdateTestCase {
                update: Either::Left(vec![(2, None)]),
                expected_result: None,
            },
            UpdateTestCase {
                update: Either::Right(vec![((1, 1012), Some("CHILD_1012_PARENT_1"))]),
                expected_result: Some(vec![(
                    (1, (1, 1012)),
                    Some(("PARENT_1_UPDATE_1", "CHILD_1012_PARENT_1")),
                )]),
            },
            UpdateTestCase {
                update: Either::Right(vec![((1, 1011), None)]),
                expected_result: Some(vec![((1, (1, 1011)), None)]),
            },
            // Deleted twice... it's already deleted so nothing should be emitted.
            UpdateTestCase {
                update: Either::Right(vec![((1, 1011), None)]),
                expected_result: None,
            },
            // PARENT_4 has no children... yet
            UpdateTestCase {
                update: Either::Left(vec![(4, Some("PARENT_4"))]),
                expected_result: None,
            },
            // Now PARENT_4 has children, and we also modified CHILD_1031
            UpdateTestCase {
                update: Either::Right(vec![
                    ((4, 1041), Some("CHILD_1041_PARENT_4")),
                    ((4, 1042), Some("CHILD_1042_PARENT_4")),
                    ((3, 1031), Some("CHILD_1031_PARENT_3_UPDATE_1")),
                ]),
                expected_result: Some(vec![
                    ((4, (4, 1041)), Some(("PARENT_4", "CHILD_1041_PARENT_4"))),
                    ((4, (4, 1042)), Some(("PARENT_4", "CHILD_1042_PARENT_4"))),
                    (
                        (3, (3, 1031)),
                        Some(("PARENT_3", "CHILD_1031_PARENT_3_UPDATE_1")),
                    ),
                ]),
            },
        ];

        let waker = futures::task::noop_waker_ref();
        let mut cx = std::task::Context::from_waker(&waker);

        for (index, tc) in test_cases.into_iter().enumerate() {
            let UpdateTestCase {
                update,
                expected_result,
            } = tc;

            match update {
                Either::Left(parents) => parent_tx.send(parents).await.unwrap(),
                Either::Right(children) => child_tx.send(children).await.unwrap(),
            }

            let Some(mut expected_result) = expected_result else {
                assert_pending!(updates.poll_next_unpin(&mut cx));
                continue;
            };

            let update = assert_ready!(updates.poll_next_unpin(&mut cx));

            let Some(update) = update else {
                panic!("Expected Some for update");
            };

            expected_result.sort_by_key(|r| r.0);

            let mut actual_result = update.into_iter().collect::<Vec<_>>();
            actual_result.sort_by_key(|r| r.0);

            assert_eq!(actual_result, expected_result, "TestCase {index} failed")
        }
    }
}
