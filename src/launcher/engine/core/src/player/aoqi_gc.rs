//! Cooperative GC scheduling, including low-allocation bitmap workloads.
//! A single root/object trace is indivisible; the budget is not a real-time limit.
use gc_arena::{Arena, Collect, Rootable, arena::CollectionPhase};
use std::time::Duration;
use web_time::Instant;

const PERIODIC_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn slice_duration() -> Duration {
    let parsed = std::env::var("AOQI_GC_SLICE_MS").ok().and_then(|v| v.parse().ok());
    Duration::from_secs_f64(aoqi_recovered_core::runtime_config::gc_slice_ms(parsed) / 1000.0)
}

pub(super) fn sleep_factor() -> f64 {
    let parsed = std::env::var("AOQI_GC_SLEEP_FACTOR").ok().and_then(|v| v.parse().ok());
    aoqi_recovered_core::runtime_config::gc_sleep_factor(parsed)
}

pub(super) struct Scheduler {
    last_completed: Instant,
    last_slice: Option<Instant>,
    pub completed_cycles: u64,
    pub periodic_cycles: u64,
    pub slices: u64,
    pub steps: u64,
    pub total_ns: u64,
    pub max_slice_ns: u64,
    pub over_budget_slices: u64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self { last_completed: Instant::now(), last_slice: None, completed_cycles: 0, periodic_cycles: 0,
            slices: 0, steps: 0, total_ns: 0, max_slice_ns: 0, over_budget_slices: 0 }
    }
}

impl Scheduler {
    pub(super) fn collect_for_frame<R>(&mut self, arena: &mut Arena<R>, budget: Duration, frame_interval: Duration)
    where
        R: for<'gc> Rootable<'gc>,
        for<'gc> <R as Rootable<'gc>>::Root: Sized + Collect<'gc>,
    {
        if !budget.is_zero() && self.claim_frame(Instant::now(), frame_interval) {
            self.drive(arena, budget, self.last_completed.elapsed() >= PERIODIC_INTERVAL);
        }
    }

    fn claim_frame(&mut self, now: Instant, interval: Duration) -> bool {
        if self.last_slice.is_some_and(|last| now.duration_since(last) < interval) { return false; }
        self.last_slice = Some(now);
        true
    }

    fn drive<R>(&mut self, arena: &mut Arena<R>, budget: Duration, periodic_due: bool)
    where
        R: for<'gc> Rootable<'gc>,
        for<'gc> <R as Rootable<'gc>>::Root: Sized + Collect<'gc>,
    {
        if budget.is_zero() { return; }
        if arena.collection_phase() == CollectionPhase::Sleeping && arena.metrics().allocation_debt() <= 0.0 {
            if !periodic_due { return; }
            self.periodic_cycles = self.periodic_cycles.saturating_add(1);
        }
        let start = Instant::now();
        self.slices += 1;
        while start.elapsed() < budget {
            // Once started, a cycle continues even if marking credits cancel its
            // debt. No artificial debt is added or removed, and live roots remain
            // subject to the ordinary GC trace and weak-reference rules.
            arena.collect_step();
            self.steps += 1;
            if arena.collection_phase() == CollectionPhase::Sleeping {
                self.completed_cycles = self.completed_cycles.saturating_add(1);
                self.last_completed = Instant::now();
                break;
            }
        }
        let elapsed = start.elapsed();
        let ns = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        self.total_ns = self.total_ns.saturating_add(ns);
        self.max_slice_ns = self.max_slice_ns.max(ns);
        self.over_budget_slices += u64::from(elapsed > budget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_arena::{Gc, GcWeak};
    use crate::bitmap::bitmap_data::BitmapData;

    #[test]
    fn repeated_host_wakeups_share_one_frame_collection_slot() {
        let mut scheduler = Scheduler::default();
        let start = Instant::now();
        let interval = Duration::from_micros(16_667);
        assert!(scheduler.claim_frame(start, interval));
        for us in [0, 1, 4000, 8000, 16000] {
            assert!(!scheduler.claim_frame(start + Duration::from_micros(us), interval));
        }
        assert!(scheduler.claim_frame(start + interval, interval));
        assert!(!scheduler.claim_frame(start + interval, interval));
    }

    #[test]
    fn player_updates_defer_collection_until_the_frame_boundary() {
        let player = crate::PlayerBuilder::new().build();
        let mut player = player.lock().unwrap();
        for _ in 0..10 {
            player.update(|context| { let _ = Gc::new(context.gc_context, 42u32); });
        }
        assert_eq!(player.gc_memory_stats().slices, 0);
        player.tick(ruffle_common::duration::FloatDuration::from_millis(17.0));
        assert!(player.gc_memory_stats().slices <= 1);
    }

    #[test]
    fn zero_budget_neither_starts_nor_advances_collection() {
        let mut arena = Arena::<Rootable![Vec<Gc<'_, u32>>]>::new(|mc| {
            (0..10000).map(|v| Gc::new(mc, v)).collect()
        });
        let mut scheduler = Scheduler::default();
        let debt = arena.metrics().allocation_debt();
        scheduler.drive(&mut arena, Duration::ZERO, true);
        assert_eq!(arena.collection_phase(), CollectionPhase::Sleeping);
        assert_eq!(arena.metrics().allocation_debt(), debt);
        arena.collect_step();
        let phase = arena.collection_phase();
        scheduler.drive(&mut arena, Duration::ZERO, true);
        assert_eq!(arena.collection_phase(), phase);
        assert_eq!(scheduler.completed_cycles, 0);
        assert_eq!(scheduler.periodic_cycles, 0);
    }

    #[test]
    fn active_cycle_finishes_without_debt_or_new_allocations() {
        let mut arena = Arena::<Rootable![Vec<GcWeak<'_, u32>>]>::new(|mc| {
            (0..100).map(|v| Gc::downgrade(Gc::new(mc, v))).collect()
        });
        arena.metrics().adjust_debt(-1000.0);
        arena.collect_step();
        assert_ne!(arena.collection_phase(), CollectionPhase::Sleeping);
        assert_eq!(arena.metrics().allocation_debt(), 0.0);
        let mut scheduler = Scheduler::default();
        for _ in 0..100 {
            scheduler.drive(&mut arena, Duration::from_millis(10), false);
            if scheduler.completed_cycles == 1 { break; }
        }
        assert_eq!(scheduler.completed_cycles, 1);
        arena.mutate(|_, root| assert!(root.iter().all(|ptr| ptr.is_dropped())));
    }

    #[test]
    fn periodic_cycle_releases_a_large_bitmap_with_zero_object_debt() {
        type BitmapArena = Arena<Rootable![(Option<Gc<'_, BitmapData<'_>>>, GcWeak<'_, BitmapData<'_>>)]>;
        let mut arena = BitmapArena::new(|mc| {
            let bitmap = Gc::new(mc, BitmapData::new(mc, 1024, 1024, true, 0));
            (Some(bitmap), Gc::downgrade(bitmap))
        });
        arena.finish_cycle();
        arena.mutate_root(|_, root| root.0 = None);
        assert_eq!(arena.metrics().allocation_debt(), 0.0);
        let mut scheduler = Scheduler::default();
        scheduler.drive(&mut arena, Duration::from_millis(10), false);
        assert_eq!(scheduler.completed_cycles, 0);
        for _ in 0..100 {
            scheduler.drive(&mut arena, Duration::from_millis(10), true);
            if scheduler.completed_cycles == 1 { break; }
        }
        assert_eq!(scheduler.completed_cycles, 1);
        assert_eq!(scheduler.periodic_cycles, 1);
        arena.mutate(|_, root| assert!(root.1.is_dropped()));
    }
}
