use ruffle_render::backend::ShapeHandle;
use web_time::{Duration, Instant};

pub(crate) const GPU_CACHE_IDLE: Duration = Duration::from_secs(30);

/// The scale ratio threshold beyond which a graphic or a drawing will be retessellated.
const RETESSELLATION_SCALE_THRESHOLD: f32 = 2.0;

/// The inverse of the retessellation scale threshold
/// Used to avoid computing the inverse repeatedly when comparing scale ratios against the threshold.
const RETESSELLATION_SCALE_THRESHOLD_INVERSE: f32 = 1.0 / RETESSELLATION_SCALE_THRESHOLD;

/// The maximum number of retessellated shapes to cache per original shape.
const RETESSELLATION_CACHE_SIZE: usize = 4;

#[derive(Clone, Debug)]
pub(crate) struct TessellationCache {
    entries: [Option<(f32, ShapeHandle, Instant)>; RETESSELLATION_CACHE_SIZE],
    len: usize,
}

/// A cache for tessellated shapes at different scales, using a simple LRU eviction policy.
/// (LRU index: 0, MRU index: len - 1)
///
/// The cache stores a fixed number of entries, and when it is full, the least recently
/// used entry is evicted to make room for new entries.
///
/// This is used to avoid retessellating shapes at similar scales multiple times, which can be expensive.
impl TessellationCache {
    /// Creates a new, empty tessellation cache.
    pub(crate) fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            len: 0,
        }
    }

    /// Finds the cached shape handle with the closest scale to the target scale.
    ///
    /// If the closest scale is NOT within the retessellation threshold,
    /// then `None` is returned, indicating that the shape should be retessellated at the target scale.
    pub(crate) fn find_near_and_touch(&mut self, target_scale: f32) -> Option<ShapeHandle> {
        let mut best_index = None;
        let mut best_deviation = f32::INFINITY;

        for (index, entry) in self.entries[..self.len].iter().enumerate() {
            if let Some((cached_scale, _, _)) = entry {
                let ratio = f32::abs(target_scale / cached_scale);

                // Check if the cached scale is within the retessellation threshold of the target scale.
                if ratio <= RETESSELLATION_SCALE_THRESHOLD
                    && ratio >= RETESSELLATION_SCALE_THRESHOLD_INVERSE
                {
                    // Choose the cached entry with the smallest deviation from the target scale.
                    let deviation = f32::abs(ratio - 1.0);

                    if deviation < best_deviation {
                        best_deviation = deviation;
                        best_index = Some(index);
                    }
                }
            }
        }

        // If we found a suitable cached entry, move it to the most recently used position.
        best_index.map(|index| self.touch_entry(index))
    }

    /// Inserts a new cached shape handle with the given scale.
    ///
    /// If the cache is full, the least recently used entry will be evicted to make room for the new entry.
    /// We assume that the caller has already checked that the new entry is not too similar to existing entries.
    pub(crate) fn insert(&mut self, scale: f32, handle: ShapeHandle) {
        if self.len < RETESSELLATION_CACHE_SIZE {
            // If the cache is not full, simply add the new entry at the end.
            self.entries[self.len] = Some((scale, handle, Instant::now()));
            self.len += 1;
            return;
        }

        // If the cache is full, evict the least recently used entry.
        for i in 1..self.len {
            self.entries[i - 1] = self.entries[i].take();
        }

        self.entries[self.len - 1] = Some((scale, handle, Instant::now()));
    }

    pub(crate) fn expire_idle(&mut self, now: Instant) -> usize {
        let old_len = self.len;
        let mut kept = 0;
        for index in 0..old_len {
            let entry = self.entries[index].take().unwrap();
            if now.saturating_duration_since(entry.2) < GPU_CACHE_IDLE {
                self.entries[kept] = Some(entry);
                kept += 1;
            }
        }
        self.len = kept;
        old_len - kept
    }

    /// Returns the number of cached entries currently stored in the cache.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Moves the entry at the given index to the most recently used position and returns its shape handle.
    fn touch_entry(&mut self, index: usize) -> ShapeHandle {
        self.entries[index].as_mut().unwrap().2 = Instant::now();
        if index == self.len - 1 {
            // The entry is already the most recently used, so we can return its handle directly.
            return self.entries[index]
                .as_ref()
                .expect("tessellation cache entry exists")
                .1
                .clone();
        }

        let entry = self.entries[index].take().expect("entry exists");

        for i in index + 1..self.len {
            self.entries[i - 1] = self.entries[i].take();
        }

        // Move touched entry to MRU position (end)
        self.entries[self.len - 1] = Some(entry.clone());
        entry.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[derive(Debug)]
    struct Shape;
    impl ruffle_render::backend::ShapeHandleImpl for Shape {}

    #[test]
    fn idle_mesh_cache_preserves_recent_scales_and_inflight_owners() {
        let mut cache = TessellationCache::new();
        let old = ShapeHandle(Arc::new(Shape));
        let owner = Arc::downgrade(&old.0);
        cache.insert(1.0, old.clone());
        cache.insert(8.0, ShapeHandle(Arc::new(Shape)));
        cache.entries[0].as_mut().unwrap().2 = Instant::now() - GPU_CACHE_IDLE;
        assert_eq!(cache.expire_idle(Instant::now()), 1);
        assert_eq!(cache.len(), 1);
        assert!(cache.find_near_and_touch(1.0).is_none());
        assert!(owner.upgrade().is_some(), "a live command must retain its mesh");
        drop(old);
        assert!(owner.upgrade().is_none());
        cache.insert(1.0, ShapeHandle(Arc::new(Shape)));
        assert!(cache.find_near_and_touch(1.0).is_some(), "an evicted scale can be rebuilt");
        assert!(cache.find_near_and_touch(8.0).is_some());
    }

    #[test]
    fn using_a_mesh_refreshes_idle_deadline() {
        let mut cache = TessellationCache::new();
        cache.insert(1.0, ShapeHandle(Arc::new(Shape)));
        cache.entries[0].as_mut().unwrap().2 = Instant::now() - GPU_CACHE_IDLE;
        assert!(cache.find_near_and_touch(1.1).is_some());
        assert_eq!(cache.expire_idle(Instant::now()), 0);
        assert_eq!(cache.expire_idle(Instant::now() + GPU_CACHE_IDLE), 1);
    }
}
