//! Cooperative GC scheduling, including low-allocation bitmap workloads.
//! A single root/object trace is indivisible; the budget is not a real-time limit.
use gc_arena::{Arena, Collect, Rootable, arena::CollectionPhase};
use ruffle_common::duration::FloatDuration;
use std::time::Duration;
use web_time::Instant;

const PERIODIC_INTERVAL: Duration = Duration::from_secs(5);
const BATTLE_FOLLOWUP_DELAY: Duration = Duration::from_millis(2200);
const BATTLE_CATCHUP_MAX: Duration = Duration::from_millis(8);
const CATCHUP_NODE_DELTA_CAP: usize = 500_000;
const STALLED_MARK_ESCALATION: Duration = Duration::from_secs(15);
const STALLED_CATCHUP_MAX: Duration = Duration::from_millis(12);
const BUSY_FRAME_CATCHUP_INTERVAL: Duration = Duration::from_millis(125);
const BATTLE_DRAIN_MAX: Duration = Duration::from_millis(16);
// MyPro counts two SWF/library registries. This engine indexes each movie once.
const FULL_FOLLOWUP_MOVIES: usize = 305;

pub(super) fn estimated_tick_work(work: Duration, tick_interval: FloatDuration, target: FloatDuration) -> Duration {
    // A late host tick can include a slow render after the preceding tick's GC.
    if tick_interval.as_millis() > 1.5 * target.as_millis() {
        work.max(target.to_std())
    } else {
        work
    }
}

#[derive(Clone, Copy)]
enum BattleWork {
    Main,
    Followup(Instant),
}

#[derive(Default)]
pub(crate) struct BattleMaintenance {
    // Identity only. Holding a strong AVM2 reference here would keep the panel alive.
    active: Option<usize>,
    pub(crate) managers_returned: Option<usize>,
    work: Option<BattleWork>,
    drain: Option<BattleGcReport>,
    pub(super) legacy_followup_only: bool,
    pub(super) entries: u64,
    pub(super) exits: u64,
    pub(super) main_cycles: u64,
    pub(super) full_followup_cycles: u64,
    pub(super) followup_slices: u64,
    pub(super) catchup_enabled: bool,
    pub(super) catchup_slices: u64,
    pub(super) combat_cycles: u64,
    pub(super) combat_dropped: u64,
    pub(super) combat_freed: u64,
}

impl BattleMaintenance {
    pub(super) fn from_config() -> Self {
        Self {
            legacy_followup_only: std::env::var("AOLA_BATTLE_CLEANUP").as_deref() == Ok("0"),
            catchup_enabled: std::env::var("AOLA_BATTLE_GC_CATCHUP").as_deref() != Ok("0"),
            ..Self::default()
        }
    }

    pub(crate) fn enter(&mut self, receiver: usize) {
        self.active = Some(receiver);
        self.work = None;
        self.drain = None;
        self.entries += 1;
        tracing::info!(target: "aola_swf_operation", operation = "battle-enter", battleId = self.entries);
    }

    pub(crate) fn finish_dispose(&mut self, previous: Option<usize>, receiver: Option<usize>,
        succeeded: bool, now: Instant, enabled: bool) {
        // Restore the enclosing call's marker even after an AVM2 exception.
        let managers = std::mem::replace(&mut self.managers_returned, previous);
        if !succeeded || managers.is_none() || managers != receiver { return; }
        self.exits += 1;
        // The tail of dispose can already have constructed the next fight.
        let exited_active = self.active == receiver;
        if exited_active {
            self.active = None;
            self.work = enabled.then(|| if self.legacy_followup_only {
                BattleWork::Followup(now + BATTLE_FOLLOWUP_DELAY)
            } else {
                BattleWork::Main
            });
        }
        tracing::info!(target: "aola_swf_operation", operation = "battle-exit", exits = self.exits,
            battleId = exited_active.then_some(self.entries));
    }

    pub(super) fn pending(&self) -> bool {
        self.work.is_some()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

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
    cycle_baseline: Option<(Instant, usize)>,
    last_catchup: Option<Instant>,
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
        Self { last_completed: Instant::now(), last_slice: None, cycle_baseline: None, last_catchup: None,
            completed_cycles: 0, periodic_cycles: 0,
            slices: 0, steps: 0, total_ns: 0, max_slice_ns: 0, over_budget_slices: 0 }
    }
}

pub(super) struct BattleGcReport {
    work: BattleWork,
    full: bool,
    registered_movies: usize,
    weak_pruned_before: u64,
    nodes_before: usize,
    nodes_after: usize,
    dropped: u64,
    freed: u64,
    cycles: u64,
    gc_time: Duration,
}

impl BattleGcReport {
    pub(super) fn finish_maintenance(self, battle: &mut BattleMaintenance, remaining_movies: usize, weak_pruned: u64,
        maintenance_time: Duration) {
        let operation = match self.work {
            BattleWork::Main => {
                // Leave the full delay after the post-GC library and audio maintenance as well.
                if let Some(BattleWork::Followup(deadline)) = &mut battle.work {
                    *deadline += maintenance_time;
                }
                "battle-main-gc"
            }
            BattleWork::Followup(_) => "battle-followup-gc",
        };
        tracing::info!(target: "aola_swf_operation", operation, battleId = battle.entries,
            mode = if self.full { "incremental-drain" } else { "slice" },
            registeredMovies = self.registered_movies as u64, registeredMoviesAfter = remaining_movies as u64,
            nodesBefore = self.nodes_before as u64, nodesAfter = self.nodes_after as u64,
            dropped = self.dropped, freed = self.freed, cyclesCompleted = self.cycles,
            weakEntriesPruned = weak_pruned - self.weak_pruned_before, gcUs = self.gc_time.as_micros() as u64,
            maintenanceUs = maintenance_time.as_micros() as u64);
    }
}

impl Scheduler {
    fn catchup_budget(&mut self, base: Duration, interval: Duration, work: Duration, now: Instant,
        nodes: usize, ready_ephemerons: bool) -> Duration {
        let (since, initial_nodes) = *self.cycle_baseline.get_or_insert((now, nodes));
        let age = now.saturating_duration_since(since);
        let delta = initial_nodes.div_ceil(4).min(CATCHUP_NODE_DELTA_CAP).max(1);
        if age < PERIODIC_INTERVAL || (nodes.saturating_sub(initial_nodes) < delta && !ready_ephemerons) {
            return base;
        }
        let target = if age >= STALLED_MARK_ESCALATION {
            (base * 3).min(interval * 2 / 3).min(STALLED_CATCHUP_MAX)
        } else {
            (base * 2).min(interval / 2).min(BATTLE_CATCHUP_MAX)
        };
        if target <= base { return base; }
        // Keep saturated ticks on the normal budget between occasional recovery slices.
        if work.saturating_add(target) > interval && self.last_catchup.is_some_and(|last|
            now.saturating_duration_since(last) < BUSY_FRAME_CATCHUP_INTERVAL)
        {
            return base;
        }
        self.last_catchup = Some(now);
        target
    }

    pub(super) fn collect_for_frame<R>(
        &mut self, arena: &mut Arena<R>, budget: Duration, frame_interval: Duration, frame_work: Duration,
        battle: &mut BattleMaintenance, now: Instant, inventory: impl FnOnce(&Arena<R>) -> (usize, u64),
    ) -> Option<BattleGcReport>
    where
        R: for<'gc> Rootable<'gc>,
        for<'gc> <R as Rootable<'gc>>::Root: Sized + Collect<'gc>,
    {
        if budget.is_zero() || !self.claim_frame(now, frame_interval) { return None; }
        let work = match battle.work {
            Some(BattleWork::Main) => BattleWork::Main,
            Some(BattleWork::Followup(due)) if now >= due => BattleWork::Followup(due),
            _ => {
                let metrics = arena.metrics().clone();
                let combat = battle.is_active().then(|| (
                    metrics.lifetime_dropped(), metrics.lifetime_freed(), self.completed_cycles,
                ));
                let phase = arena.collection_phase();
                let effective_budget = if battle.catchup_enabled && phase != CollectionPhase::Sleeping {
                    self.catchup_budget(budget, frame_interval, frame_work, now, metrics.total_gc_count(),
                        phase == CollectionPhase::Marking && arena.mark_backlog().2 > 0)
                } else {
                    budget
                };
                self.drive(arena, effective_budget, now, false);
                if let Some((dropped, freed, cycles)) = combat {
                    battle.catchup_slices += u64::from(effective_budget > budget);
                    battle.combat_cycles += self.completed_cycles - cycles;
                    battle.combat_dropped += metrics.lifetime_dropped() - dropped;
                    battle.combat_freed += metrics.lifetime_freed() - freed;
                }
                return None;
            }
        };
        let metrics = arena.metrics().clone();
        let mut report = battle.drain.take().unwrap_or_else(|| {
            let (registered_movies, weak_pruned_before) = inventory(arena);
            let full = matches!(work, BattleWork::Main)
                || (!battle.legacy_followup_only && registered_movies > FULL_FOLLOWUP_MOVIES);
            BattleGcReport { work, full, registered_movies, weak_pruned_before,
                nodes_before: metrics.total_gc_count(), nodes_after: 0, dropped: 0, freed: 0,
                cycles: 0, gc_time: Duration::ZERO }
        });
        let dropped_before = metrics.lifetime_dropped();
        let freed_before = metrics.lifetime_freed();
        let previous_cycles = self.completed_cycles;
        let previous_steps = self.steps;
        let drain_budget = if report.full {
            // Direct run_frame callers do not specify a host frame interval.
            let limit = if frame_interval.is_zero() { budget } else { frame_interval * 4 / 5 };
            limit.min(BATTLE_DRAIN_MAX)
        } else {
            budget
        };
        let start = Instant::now();
        // Complete a full pass across frames using the same arena state.
        self.drive(arena, drain_budget, now, true);
        let gc_time = start.elapsed();
        report.gc_time += gc_time;
        report.dropped += metrics.lifetime_dropped() - dropped_before;
        report.freed += metrics.lifetime_freed() - freed_before;
        report.cycles += self.completed_cycles - previous_cycles;
        if self.steps == previous_steps || (report.full && report.cycles == 0) {
            battle.drain = Some(report);
            return None;
        }
        report.nodes_after = metrics.total_gc_count();
        battle.work = match work {
            BattleWork::Main => {
                battle.main_cycles += 1;
                Some(BattleWork::Followup(now + gc_time + BATTLE_FOLLOWUP_DELAY))
            }
            BattleWork::Followup(_) => {
                battle.followup_slices += 1;
                battle.full_followup_cycles += u64::from(report.full);
                None
            }
        };
        Some(report)
    }

    fn claim_frame(&mut self, now: Instant, interval: Duration) -> bool {
        if self.last_slice.is_some_and(|last| now.duration_since(last) < interval) { return false; }
        self.last_slice = Some(now);
        true
    }

    fn drive<R>(&mut self, arena: &mut Arena<R>, budget: Duration, now: Instant, battle_due: bool)
    where
        R: for<'gc> Rootable<'gc>,
        for<'gc> <R as Rootable<'gc>>::Root: Sized + Collect<'gc>,
    {
        if budget.is_zero() { return; }
        if arena.collection_phase() == CollectionPhase::Sleeping {
            self.cycle_baseline = None;
            self.last_catchup = None;
            if arena.metrics().allocation_debt() <= 0.0 {
                let periodic_due = now.saturating_duration_since(self.last_completed) >= PERIODIC_INTERVAL;
                if !periodic_due && !battle_due { return; }
                self.periodic_cycles = self.periodic_cycles.saturating_add(u64::from(!battle_due));
            }
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
                self.last_completed = now + start.elapsed();
                self.cycle_baseline = None;
                self.last_catchup = None;
                break;
            }
        }
        self.record_time(start.elapsed(), budget);
    }

    fn record_time(&mut self, elapsed: Duration, budget: Duration) {
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
    fn catchup_requires_age_growth_and_spaces_out_busy_ticks() {
        let base = Duration::from_millis(4);
        let target = Duration::from_millis(8);
        let frame = Duration::from_millis(17);
        let start = Instant::now();
        let due = start + PERIODIC_INTERVAL;
        let mut scheduler = Scheduler::default();
        assert_eq!(scheduler.catchup_budget(base, frame, frame, start, 100, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, frame, due - frame, 130, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, frame, due, 124, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, frame, due, 125, false), target);
        assert_eq!(scheduler.catchup_budget(base, frame, frame, due + frame, 130, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, frame, due + BUSY_FRAME_CATCHUP_INTERVAL, 130, false), target);
        let next = due + BUSY_FRAME_CATCHUP_INTERVAL + frame;
        let target_frame = FloatDuration::from_millis(17.0);
        let light_tick = Duration::from_millis(2);
        let late = estimated_tick_work(light_tick, FloatDuration::from_millis(26.0), target_frame);
        assert_eq!(scheduler.catchup_budget(base, frame, late, next, 130, false), base);
        let on_time = estimated_tick_work(light_tick, FloatDuration::from_millis(17.0), target_frame);
        assert_eq!(scheduler.catchup_budget(base, frame, on_time, next, 130, false), target);
    }

    #[test]
    fn catchup_triggers_on_large_heap_without_waiting_for_25_percent() {
        let mut scheduler = Scheduler::default();
        let base = Duration::from_millis(4);
        let frame = Duration::from_millis(20);
        let start = Instant::now();
        let due = start + PERIODIC_INTERVAL;
        let nodes = 20_000_000;
        assert_eq!(scheduler.catchup_budget(base, frame, Duration::ZERO, start, nodes, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, Duration::ZERO, due, nodes + 499_999, false), base);
        assert_eq!(scheduler.catchup_budget(base, frame, Duration::ZERO, due, nodes + 500_000, false), BATTLE_CATCHUP_MAX);
        assert_eq!(scheduler.catchup_budget(base, frame, Duration::ZERO, due, nodes, true), BATTLE_CATCHUP_MAX);
        assert_eq!(scheduler.catchup_budget(base, frame, Duration::ZERO, start + STALLED_MARK_ESCALATION, nodes, true),
            STALLED_CATCHUP_MAX);
    }

    #[test]
    fn battle_catchup_sweeps_discarded_opening_assets_while_other_objects_keep_changing() {
        catchup_sweeps_discarded_opening_assets(true);
    }

    #[test]
    fn non_battle_catchup_breaks_marking_livelock() {
        catchup_sweeps_discarded_opening_assets(false);
    }

    fn catchup_sweeps_discarded_opening_assets(in_battle: bool) {
        // Model three objects whose indivisible traces must be redone if their children change.
        struct Hot<'gc>(Gc<'gc, u32>);
        // SAFETY: the only GC pointer is traced; Hot has no Drop or interior mutability.
        unsafe impl<'gc> Collect<'gc> for Hot<'gc> {
            fn trace<T: gc_arena::collect::Trace<'gc>>(&self, cc: &mut T) {
                cc.trace(&self.0);
                let start = Instant::now();
                while start.elapsed() < Duration::from_micros(2300) {
                    std::hint::spin_loop();
                }
            }
        }
        #[derive(Collect)]
        #[collect(no_drop)]
        struct Scene<'gc> {
            active: Vec<gc_arena::GcRefLock<'gc, Hot<'gc>>>,
            opening: Option<Gc<'gc, BitmapData<'gc>>>,
            observer: GcWeak<'gc, BitmapData<'gc>>,
            retained: Gc<'gc, u32>,
        }
        for catchup_enabled in [false, true] {
            let mut arena = Arena::<Rootable![Scene<'_>]>::new(|mc| {
                let opening = Gc::new(mc, BitmapData::new(mc, 32, 32, true, 0));
                Scene {
                    active: (0..3).map(|i| Gc::new(mc, gc_arena::RefLock::new(Hot(Gc::new(mc, i))))).collect(),
                    opening: Some(opening),
                    observer: Gc::downgrade(opening),
                    retained: Gc::new(mc, 42),
                }
            });
            arena.finish_cycle();
            let mut battle = BattleMaintenance { catchup_enabled, ..Default::default() };
            if in_battle { battle.enter(1); }
            arena.mutate_root(|_, scene| scene.opening = None);
            arena.collect_step();
            let mut scheduler = Scheduler::default();
            let start = scheduler.last_completed;
            let frame = Duration::from_millis(17);
            for i in 0..32 {
                arena.mutate(|mc, scene| {
                    for object in &scene.active { object.borrow_mut(mc).0 = Gc::new(mc, i); }
                    assert_eq!(*scene.retained, 42);
                });
                let now = if i == 0 { start } else { start + PERIODIC_INTERVAL + frame * i };
                scheduler.collect_for_frame(&mut arena, Duration::from_millis(4), frame, frame,
                    &mut battle, now, |_| unreachable!("no post-battle maintenance during combat"));
                arena.mutate(|_, scene| assert!(scene.active.iter().all(|object| *object.borrow().0 == i)));
                if scheduler.completed_cycles > 0 { break; }
            }
            let dropped = arena.mutate(|_, scene| scene.observer.is_dropped());
            eprintln!("battle_catchup={catchup_enabled} slices={} boosted={} cycles={} dropped={} freed={} opening_dropped={dropped}",
                scheduler.slices, battle.catchup_slices, battle.combat_cycles, battle.combat_dropped, battle.combat_freed);
            assert_eq!(dropped, catchup_enabled);
            assert_eq!(scheduler.completed_cycles, u64::from(catchup_enabled));
            assert_eq!(battle.combat_freed > 0, in_battle && catchup_enabled);
            assert_eq!(battle.combat_cycles, u64::from(in_battle && catchup_enabled));
            assert_eq!(battle.exits, 0);
            assert_eq!(battle.is_active(), in_battle);
        }
    }

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
    fn battle_dispose_requires_both_returns_and_preserves_nested_calls() {
        let mut battle = BattleMaintenance::default();
        let now = Instant::now();
        battle.enter(1);
        battle.finish_dispose(None, Some(1), true, now, true); // early _isDisposed return
        battle.managers_returned = Some(2);
        battle.finish_dispose(None, Some(1), true, now, true); // another receiver
        battle.managers_returned = Some(1);
        battle.finish_dispose(None, Some(1), false, now, true); // AVM2 error in tail of dispose
        assert_eq!(battle.exits, 0);

        battle.managers_returned = Some(1);
        let outer = battle.managers_returned.take();
        battle.finish_dispose(outer, Some(1), true, now, true); // reentrant early return
        battle.enter(2); // dispose tail started the next fight
        battle.finish_dispose(None, Some(1), true, now, true);
        assert_eq!((battle.entries, battle.exits, battle.pending()), (2, 1, false));
        battle.managers_returned = Some(2);
        battle.finish_dispose(None, Some(2), true, now, true);
        battle.finish_dispose(None, Some(2), true, now, true);
        assert_eq!((battle.exits, battle.pending()), (2, true));
        battle.enter(3);
        assert!(!battle.pending());
    }

    #[test]
    fn battle_due_waits_for_a_claimed_frame_and_nonzero_budget() {
        let mut arena = Arena::<Rootable![()]>::new(|_| ());
        let mut scheduler = Scheduler::default();
        let mut battle = BattleMaintenance { legacy_followup_only: true, ..Default::default() };
        let now = scheduler.last_completed;
        let due = now + BATTLE_FOLLOWUP_DELAY;
        let frame = Duration::from_millis(16);
        let budget = Duration::from_millis(2);
        battle.enter(1);
        battle.managers_returned = Some(1);
        battle.finish_dispose(None, Some(1), true, now, true);
        scheduler.collect_for_frame(&mut arena, budget, frame, Duration::ZERO, &mut battle, due - Duration::from_millis(1), |_| (0, 0));
        scheduler.collect_for_frame(&mut arena, budget, frame, Duration::ZERO, &mut battle, due, |_| (0, 0));
        scheduler.collect_for_frame(&mut arena, Duration::ZERO, frame, Duration::ZERO, &mut battle, due + frame, |_| (0, 0));
        assert_eq!((scheduler.steps, battle.pending()), (0, true));
        scheduler.collect_for_frame(&mut arena, budget, frame, Duration::ZERO, &mut battle, due + frame, |_| (0, 0));
        let steps = scheduler.steps;
        assert!(steps > 0);
        scheduler.collect_for_frame(&mut arena, budget, frame, Duration::ZERO, &mut battle, due + frame, |_| (0, 0));
        assert_eq!((scheduler.steps, scheduler.periodic_cycles), (steps, 0));
        assert_eq!((battle.followup_slices, battle.pending()), (1, false));
    }

    #[test]
    fn battle_phases_collect_only_unreachable_values_and_apply_the_registry_threshold() {
        for (movies, legacy, full_followup) in [(305, false, false), (306, false, true), (306, true, false)] {
            let mut arena = Arena::<Rootable![Gc<'_, u32>]>::new(|mc| Gc::new(mc, 42));
            arena.finish_cycle();
            arena.mutate(|mc, _| { let _ = Gc::new(mc, 13u32); });
            let mut scheduler = Scheduler::default();
            let now = scheduler.last_completed;
            let mut battle = BattleMaintenance { legacy_followup_only: legacy, ..Default::default() };
            battle.enter(1);
            battle.managers_returned = Some(1);
            battle.finish_dispose(None, Some(1), true, now, true);
            let mut frame = |budget, battle: &mut BattleMaintenance, time| {
                scheduler.collect_for_frame(&mut arena, budget, Duration::ZERO, Duration::ZERO, battle, time, |_| (movies, 0))
            };
            assert!(frame(Duration::ZERO, &mut battle, now).is_none());
            if !legacy {
                let report = frame(Duration::from_millis(2), &mut battle, now).unwrap();
                assert!(report.full);
                assert_eq!((report.dropped, report.freed, report.cycles), (1, 1, 1));
            }
            let Some(BattleWork::Followup(due)) = battle.work else { panic!("missing follow-up") };
            assert!(due >= now + BATTLE_FOLLOWUP_DELAY);
            assert!(frame(Duration::from_millis(2), &mut battle, due - Duration::from_millis(1)).is_none());
            let report = frame(Duration::from_millis(2), &mut battle, due).unwrap();
            assert_eq!(report.full, full_followup);
            assert_eq!((battle.main_cycles, battle.full_followup_cycles, battle.followup_slices),
                (u64::from(!legacy), u64::from(full_followup), 1));
            assert!(frame(Duration::from_millis(2), &mut battle, due + BATTLE_FOLLOWUP_DELAY).is_none());
            arena.mutate(|_, key| assert_eq!(**key, 42));
        }
    }

    #[test]
    fn battle_drains_span_frames_and_reentry_cancels_the_pending_report() {
        struct SlowRoot<'gc>(Gc<'gc, u32>);
        // SAFETY: The only GC pointer is traced and the root has no destructor.
        unsafe impl<'gc> Collect<'gc> for SlowRoot<'gc> {
            fn trace<T: gc_arena::collect::Trace<'gc>>(&self, trace: &mut T) {
                trace.trace(&self.0);
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(2) {
                    std::hint::spin_loop();
                }
            }
        }
        let mut arena = Arena::<Rootable![SlowRoot<'_>]>::new(|mc| {
            let _ = Gc::new(mc, 13u32);
            SlowRoot(Gc::new(mc, 42))
        });
        let mut scheduler = Scheduler::default();
        let mut battle = BattleMaintenance::default();
        let now = scheduler.last_completed;
        let interval = Duration::from_millis(1);
        let budget = Duration::from_millis(4);
        battle.enter(1);
        battle.managers_returned = Some(1);
        battle.finish_dispose(None, Some(1), true, now, true);
        let mut frame = |battle: &mut BattleMaintenance, time| {
            scheduler.collect_for_frame(&mut arena, budget, interval, Duration::ZERO, battle, time,
                |_| (FULL_FOLLOWUP_MOVIES + 1, 0))
        };
        assert!(frame(&mut battle, now).is_none());
        assert_eq!(battle.main_cycles, 0);
        assert!(battle.drain.is_some());
        let report = (1..10).find_map(|i| frame(&mut battle, now + interval * i)).unwrap();
        assert_eq!((report.freed, report.cycles, battle.main_cycles), (1, 1, 1));

        let Some(BattleWork::Followup(due)) = battle.work else { panic!("missing follow-up") };
        assert!(frame(&mut battle, due).is_none());
        assert!(battle.drain.is_some());
        battle.enter(2);
        assert!(!battle.pending());
        assert!(battle.drain.is_none());
        assert!(frame(&mut battle, due + interval).is_none());
        assert_eq!((battle.followup_slices, battle.full_followup_cycles), (0, 0));
        arena.mutate(|_, root| assert_eq!(*root.0, 42));
    }

    #[test]
    #[ignore = "Set AOLA_TEST_BATTLE_PROBE to the SWF generated by test_battle_gc.py and AOLA_SWF_OPERATIONS=1"]
    fn battle_avm2_returns_schedule_followup_on_player() {
        use std::sync::{Arc, Mutex};
        struct Trace(Arc<Mutex<Vec<String>>>);
        impl crate::backend::log::LogBackend for Trace {
            fn avm_trace(&self, s: &str) { self.0.lock().unwrap().push(s.to_owned()); }
            fn avm_warning(&self, _: &str) {}
        }
        let path = std::env::var_os("AOLA_TEST_BATTLE_PROBE").unwrap();
        let movie = crate::tag_utils::SwfMovieData::from_data(&std::fs::read(path).unwrap(),
            "file:///battle-probe.swf".into(), None, None).unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let player = crate::PlayerBuilder::new().with_movie(movie).with_log(Trace(lines.clone())).build();
        let mut guard = player.lock().unwrap();
        let player = &mut *guard;
        let enabled = ruffle_render::backend::aola_long_run_maintenance();
        let expected = [
            ("standalone managers", 1, 0, false),
            ("normal then early 1", 1, 1, enabled),
            ("tail threw after managers", 2, 1, false),
            ("replacement entered 3", 4, 2, false),
            ("replacement exited 4", 4, 3, enabled),
        ];
        let mut seen = 0;
        for _ in 0..8 {
            player.run_frame();
            let lines = lines.lock().unwrap();
            if lines.len() > seen {
                assert_eq!(lines.len(), seen + 1);
                let stats = player.gc_memory_stats();
                let actual = (lines[seen].as_str(), stats.battle_entries, stats.battle_exits, stats.battle_followup_pending);
                assert_eq!(actual, expected[seen]);
                eprintln!("battle_avm2 enabled={enabled} stage={actual:?}");
                drop(lines);
                // Exercise an actual goto between AS3 construction, failed or
                // reentrant disposal, and the successful final disposal.
                let in_battle = [true, false, true, true, false][seen];
                player.mutate_with_update_context(|context| {
                    use crate::display_object::{GotoInfo, StopOrPlay};
                    assert_eq!(context.battle_maintenance.is_active(), in_battle);
                    let clip = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
                    let before = context.frame_profile.stats.target_only_gotos;
                    context.frame_profile.begin_frame();
                    clip.goto_frame(context, GotoInfo { frame: clip.current_frame(), stop_or_play: StopOrPlay::Stop });
                    context.frame_profile.finish_frame(true);
                    let mode = crate::goto_profile::goto_target_only_mode();
                    assert_eq!(context.frame_profile.stats.target_only_gotos - before,
                        u64::from(mode == "all" || mode == "battle" && in_battle));
                });
                seen += 1;
            }
        }
        assert_eq!(seen, expected.len());
        let due = Instant::now() + BATTLE_FOLLOWUP_DELAY + Duration::from_millis(10);
        let mut arena = player.gc_arena.borrow_mut();
        player.gc_scheduler.collect_for_frame(&mut arena, Duration::from_millis(2),
            Duration::from_millis(20), Duration::ZERO, &mut player.battle_maintenance, due, |_| (0, 0));
        drop(arena);
        let stats = player.gc_memory_stats();
        assert_eq!((stats.battle_followup_pending, stats.battle_followup_slices), (false, u64::from(enabled)));
        eprintln!("battle_avm2 enabled={enabled} consumed_slices={}", stats.battle_followup_slices);
    }

    #[test]
    fn repeated_battle_followups_release_idle_bitmaps_before_the_next_fight() {
        const ROUNDS: usize = 8;
        let mut released = [0usize; 2];
        for enabled in [false, true] {
            type Bitmaps = Arena<Rootable![Vec<(Option<Gc<'_, BitmapData<'_>>>, GcWeak<'_, BitmapData<'_>>)>]>;
            let mut arena = Bitmaps::new(|mc| (0..ROUNDS).map(|_| {
                let bitmap = Gc::new(mc, BitmapData::new(mc, 256, 256, true, 0));
                (Some(bitmap), Gc::downgrade(bitmap))
            }).collect());
            arena.finish_cycle(); // establish a sleeping arena with no allocation debt
            let mut scheduler = Scheduler::default();
            let mut battle = BattleMaintenance::default();
            let start = scheduler.last_completed;
            for round in 0..ROUNDS {
                let exit = start + Duration::from_secs(3 * round as u64);
                battle.enter(round + 1);
                arena.mutate_root(|_, roots| roots[round].0 = None);
                assert_eq!(arena.metrics().allocation_debt(), 0.0);
                battle.managers_returned = Some(round + 1);
                battle.finish_dispose(None, Some(round + 1), true, exit, enabled);
                for ms in (0..3000).step_by(20) {
                    scheduler.collect_for_frame(&mut arena, Duration::from_millis(2), Duration::from_millis(20), Duration::ZERO,
                        &mut battle, exit + Duration::from_millis(ms), |_| (0, 0));
                }
                released[usize::from(enabled)] += arena.mutate(|_, roots| usize::from(roots[round].1.is_dropped()));
            }
            eprintln!("battle_followup={enabled} entries={} exits={} followups={} periodic={} slices={} steps={} released_before_next={}/{}",
                battle.entries, battle.exits, battle.followup_slices, scheduler.periodic_cycles, scheduler.slices, scheduler.steps,
                released[usize::from(enabled)], ROUNDS);
            assert_eq!((battle.entries, battle.exits), (ROUNDS as u64, ROUNDS as u64));
            assert_eq!(battle.followup_slices, if enabled { ROUNDS as u64 } else { 0 });
        }
        assert_eq!(released[1], ROUNDS);
        assert!(released[0] < released[1]);
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
        let due = scheduler.last_completed + PERIODIC_INTERVAL;
        scheduler.drive(&mut arena, Duration::ZERO, due, false);
        assert_eq!(arena.collection_phase(), CollectionPhase::Sleeping);
        assert_eq!(arena.metrics().allocation_debt(), debt);
        arena.collect_step();
        let phase = arena.collection_phase();
        scheduler.drive(&mut arena, Duration::ZERO, due, false);
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
            scheduler.drive(&mut arena, Duration::from_millis(10), Instant::now(), false);
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
        scheduler.drive(&mut arena, Duration::from_millis(10), Instant::now(), false);
        assert_eq!(scheduler.completed_cycles, 0);
        for _ in 0..100 {
            scheduler.drive(&mut arena, Duration::from_millis(10), scheduler.last_completed + PERIODIC_INTERVAL, false);
            if scheduler.completed_cycles == 1 { break; }
        }
        assert_eq!(scheduler.completed_cycles, 1);
        assert_eq!(scheduler.periodic_cycles, 1);
        arena.mutate(|_, root| assert!(root.1.is_dropped()));
    }
}
