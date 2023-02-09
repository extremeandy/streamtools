use std::{
    sync::Arc,
    task::{Wake, Waker},
};

use parking_lot::Mutex;

#[derive(Debug, Default)]
pub struct OuterWaker {
    parent_waker: Mutex<Option<Waker>>,
}

impl OuterWaker {
    /// Returns `true` if `wake()` has been called since we last called `insert()`,
    /// OR if a waker has never been set.
    /// This indicates that the outer waker was woken and that we should poll the outer
    /// stream.
    pub fn set_parent_waker(&self, waker: Waker) -> bool {
        let mut guard = self.parent_waker.lock();
        let previous_waker = guard.replace(waker);
        drop(guard);

        previous_waker.is_none()
    }
}

impl Wake for OuterWaker {
    fn wake(self: Arc<Self>) {
        let mut guard = self.parent_waker.lock();
        let parent_waker = guard.take();
        drop(guard);

        if let Some(parent_waker) = parent_waker {
            parent_waker.wake_by_ref();
        }
    }
}
