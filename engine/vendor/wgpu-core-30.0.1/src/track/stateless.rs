use alloc::{sync::Arc, vec::Vec};
use core::slice::Iter;

/// A tracker that holds strong references to resources.
///
/// This is only used to keep resources alive.
#[derive(Debug)]
pub(crate) struct StatelessTracker<T> {
    resources: Vec<Arc<T>>,
}

impl<T> StatelessTracker<T> {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
        }
    }

    pub(crate) fn len(&self) -> usize { self.resources.len() }

    /// Inserts a single resource into the resource tracker.
    ///
    /// Returns a reference to the newly inserted resource.
    /// (This allows avoiding a clone/reference count increase in many cases.)
    pub fn insert_single(&mut self, resource: Arc<T>) -> &Arc<T> {
        crate::aoqi_timing::count(crate::aoqi_timing::STATELESS_INSERTS, 1);
        self.resources.push(resource);
        unsafe { self.resources.last().unwrap_unchecked() }
    }
}

impl<T> Drop for StatelessTracker<T> {
    fn drop(&mut self) {
        crate::aoqi_timing::count(crate::aoqi_timing::RETIRED_STATELESS_REFS, self.resources.len());
    }
}

impl<'a, T> IntoIterator for &'a StatelessTracker<T> {
    type Item = &'a Arc<T>;
    type IntoIter = Iter<'a, Arc<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.resources.as_slice().iter()
    }
}
