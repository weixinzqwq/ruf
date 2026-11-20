//! Reclaim regenerable display textures independently of the display object's GC lifetime.
use ruffle_render::bitmap::BitmapCacheHandle;
use std::sync::{Arc, Weak};
use std::collections::VecDeque;

struct Entry {
    handle: Weak<BitmapCacheHandle>,
    bytes: u64,
    last_used: u64,
}

#[derive(Default)]
pub(crate) struct BitmapCachePool {
    entries: VecDeque<Entry>,
    frame: u64,
    pub reserved_bytes: u64,
    pub pressure_evictions: u64,
    pub idle_evictions: u64,
    pub pressure_entries_examined: u64,
}

impl BitmapCachePool {
    pub fn register(&mut self, handle: &Arc<BitmapCacheHandle>, bytes: u64) {
        self.entries.push_back(Entry { handle: Arc::downgrade(handle), bytes, last_used: self.frame });
        self.reserved_bytes += bytes;
    }

    fn refresh(&mut self) {
        self.entries.retain_mut(|entry| {
            let Some(handle) = entry.handle.upgrade().filter(|handle| handle.is_valid()) else {
                self.reserved_bytes -= entry.bytes;
                return false;
            };
            if handle.was_used() { entry.last_used = self.frame; }
            true
        });
    }

    pub fn make_room(&mut self, bytes: u64, budget: u64) -> bool {
        if bytes > budget { return false; }
        if self.reserved_bytes.saturating_add(bytes) <= budget { return true; }
        // end_frame orders the queue once. Do not rescan/sort thousands of
        // caches for every allocation when the budget is full. Recheck use at
        // eviction time: an entry may have been drawn since that ordering.
        for _ in 0..self.entries.len() {
            if self.reserved_bytes.saturating_add(bytes) <= budget { break; }
            let mut entry = self.entries.pop_front().unwrap();
            self.pressure_entries_examined += 1;
            let Some(handle) = entry.handle.upgrade().filter(|handle| handle.is_valid()) else {
                self.reserved_bytes -= entry.bytes;
                continue;
            };
            if handle.was_used() { entry.last_used = self.frame; }
            if self.frame.saturating_sub(entry.last_used) > 1 {
                handle.evict();
                self.reserved_bytes -= entry.bytes;
                self.pressure_evictions += 1;
            } else {
                self.entries.push_back(entry);
            }
        }
        self.reserved_bytes.saturating_add(bytes) <= budget
    }

    pub fn end_frame(&mut self) {
        self.refresh();
        self.entries.retain(|entry| {
            if self.frame.saturating_sub(entry.last_used) >= 1800 {
                if let Some(handle) = entry.handle.upgrade() { handle.evict(); }
                self.reserved_bytes -= entry.bytes;
                self.idle_evictions += 1;
                false
            } else { true }
        });
        self.entries.make_contiguous().sort_by_key(|entry| entry.last_used);
        self.frame += 1;
    }
}

pub(crate) fn budget_bytes(width: u32, height: u32, dx12: bool) -> u64 {
    let pixels = u64::from(width) * u64::from(height) * 4;
    // A render-attachment texture on the committed DX12 path costs at least
    // a 64 KiB allocation. This is a budget charge, not a residency measurement.
    if dx12 { pixels.div_ceil(65536) * 65536 } else { pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruffle_render::bitmap::{BitmapHandle, BitmapHandleImpl};

    #[derive(Debug)]
    struct TestTexture;
    impl BitmapHandleImpl for TestTexture {}
    fn texture() -> Arc<BitmapCacheHandle> {
        BitmapCacheHandle::new(BitmapHandle(Arc::new(TestTexture)))
    }

    #[test]
    fn pressure_reclaims_cold_cache_but_preserves_recent_and_in_flight_handles() {
        let mut pool = BitmapCachePool::default();
        let cold = texture();
        let in_flight = cold.get().unwrap();
        let weak_texture = Arc::downgrade(&in_flight.0);
        pool.register(&cold, 64);
        pool.end_frame();
        pool.end_frame();
        let hot = texture();
        pool.register(&hot, 64);
        assert!(pool.make_room(64, 128));
        assert!(!cold.is_valid());
        assert!(cold.get().is_none());
        assert!(hot.is_valid());
        assert!(weak_texture.upgrade().is_some());
        drop(in_flight);
        assert!(weak_texture.upgrade().is_none());
        assert_eq!(pool.pressure_evictions, 1);
        assert_eq!(pool.reserved_bytes, 64);
    }

    #[test]
    fn current_and_previous_frames_are_never_evicted_for_a_new_request() {
        let mut pool = BitmapCachePool::default();
        let handle = texture();
        pool.register(&handle, 64);
        assert!(!pool.make_room(1, 64));
        pool.end_frame();
        assert!(!pool.make_room(1, 64));
        pool.end_frame();
        handle.get().unwrap();
        assert!(!pool.make_room(1, 64));
        assert!(handle.is_valid());
    }

    #[test]
    fn idle_cache_expires_while_display_owner_is_still_alive() {
        let mut pool = BitmapCachePool::default();
        let handle = texture();
        pool.register(&handle, 64);
        for _ in 0..1800 { pool.end_frame(); }
        assert!(handle.is_valid());
        pool.end_frame();
        assert!(!handle.is_valid());
        assert_eq!((pool.idle_evictions, pool.reserved_bytes), (1, 0));
    }

    #[test]
    fn registry_does_not_keep_a_discarded_display_cache_alive() {
        let mut pool = BitmapCachePool::default();
        let handle = texture();
        pool.register(&handle, 64);
        drop(handle);
        assert!(pool.make_room(64, 64));
        assert_eq!(pool.reserved_bytes, 0);
        assert!(pool.entries.is_empty());
    }

    #[test]
    fn dx12_budget_accounts_for_small_committed_textures() {
        assert_eq!(budget_bytes(1, 1, true), 65536);
        assert_eq!(budget_bytes(128, 128, true), 65536);
        assert_eq!(budget_bytes(129, 128, true), 131072);
        assert_eq!(budget_bytes(1, 1, false), 4);
    }

    #[test]
    fn pressure_checks_newly_used_entries_without_rescanning_the_whole_registry() {
        let mut pool = BitmapCachePool::default();
        let owners: Vec<_> = (0..1024).map(|_| texture()).collect();
        for handle in &owners { pool.register(handle, 64); }
        pool.end_frame(); pool.end_frame();
        // Use after end_frame's ordering: this must still protect the entry.
        owners[0].get().unwrap();
        let mut new_owners = Vec::new();
        for _ in 0..512 {
            assert!(pool.make_room(64, 1024 * 64));
            let handle = texture();
            pool.register(&handle, 64);
            new_owners.push(handle);
        }
        assert!(owners[0].is_valid());
        assert!(new_owners.iter().all(|handle| handle.is_valid()));
        assert_eq!(pool.pressure_evictions, 512);
        assert_eq!(pool.pressure_entries_examined, 513);
        assert_eq!(pool.reserved_bytes, 1024 * 64);
    }

    #[test]
    fn dropping_a_protected_owner_can_make_room_in_the_same_frame() {
        let mut pool = BitmapCachePool::default();
        let owner = texture();
        pool.register(&owner, 64);
        assert!(!pool.make_room(64, 64));
        drop(owner);
        assert!(pool.make_room(64, 64));
        assert_eq!(pool.reserved_bytes, 0);
    }
}
