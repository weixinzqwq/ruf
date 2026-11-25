//! Bounded, numeric-only goto costs within one pet-detail response.
use crate::frame_profile::{FRAME_WORK_NAMES, FrameWork};
use std::sync::OnceLock;
use web_time::Instant;

pub const REMOVED_SCRIPTS: usize = FRAME_WORK_NAMES.len();
pub const WORK_COUNT: usize = REMOVED_SCRIPTS + 1;

pub fn subtree_script_sleep_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_SUBTREE_SCRIPT_SLEEP").as_deref() == Ok("1"))
        && crate::aoqi_work::mode().sleep
}

pub fn goto_leaf_fast_path_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_GOTO_LEAF_FAST_PATH").as_deref() == Ok("1"))
}

pub fn goto_orphan_idle_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_GOTO_ORPHAN_IDLE").as_deref() == Ok("1"))
}

pub fn goto_target_only_mode() -> &'static str {
    static MODE: OnceLock<&str> = OnceLock::new();
    MODE.get_or_init(|| match std::env::var("AOLA_GOTO_TARGET_ONLY").as_deref() {
        Ok("battle") => "battle",
        Ok("1") => "all",
        _ => "off",
    })
}

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_GOTO_PROFILE").as_deref() == Ok("1"))
}

pub fn work_name(index: usize) -> &'static str {
    if index == REMOVED_SCRIPTS { "removedScripts" } else { FRAME_WORK_NAMES[index] }
}

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Counter {
    ConstructVisits, ScriptVisits, ScriptCalls, OrphanVisits, OrphanExecuted,
    OrphanInvalid, OrphanSleepSkipped, BroadcastCandidates, BroadcastDispatched,
    CleanupSlots, CleanupRemoved, ConstructLeafHits, ScriptLeafHits, OrphanGotoIdleSkipped,
}

pub const COUNTER_NAMES: [&str; 14] = [
    "movieClipConstructVisits", "movieClipScriptVisits", "scriptCalls", "orphanVisits",
    "orphanExecuted", "orphanInvalid", "orphanSleepSkipped", "broadcastCandidates",
    "broadcastDispatched", "cleanupSlots", "cleanupRemoved", "movieClipConstructLeafHits",
    "movieClipScriptLeafHits", "orphanGotoIdleSkipped",
];

#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseCost {
    pub exclusive_ns: u64,
    pub calls: u64,
    pub counts: [u64; COUNTER_NAMES.len()],
}

#[derive(Debug, Default)]
pub struct GotoStats {
    pub calls: u64,
    pub same_frame_calls: u64,
    pub stop_calls: u64,
    pub avm2_calls: u64,
    /// All inner-goto attempts, including legacy skips.
    pub inner_calls: u64,
    pub legacy_inner_skips: u64,
    pub queued_requests: u64,
    pub max_depth: u64,
    pub outer_total_ns: u64,
    /// Includes nested goto durations again; do not add this to phase costs.
    pub inclusive_ns: u64,
    pub max_call_ns: u64,
    pub orphan_slots_first: Option<u64>,
    pub orphan_slots_last: u64,
    pub orphan_slots_max: u64,
    pub phases: [PhaseCost; WORK_COUNT],
}

pub(crate) struct GotoToken {
    start: Instant,
    previous: Option<usize>,
}

pub(crate) struct GotoProfiler {
    stats: Option<Box<GotoStats>>,
    active: Option<usize>,
    since: Instant,
    depth: u64,
}

impl Default for GotoProfiler {
    fn default() -> Self {
        Self { stats: None, active: None, since: Instant::now(), depth: 0 }
    }
}

impl GotoProfiler {
    pub fn begin_response(&mut self, enabled: bool) {
        debug_assert!(!self.is_active());
        self.stats = enabled.then(|| Box::new(GotoStats::default()));
    }

    pub fn finish_response(&mut self) -> Option<Box<GotoStats>> {
        debug_assert!(!self.is_active());
        self.stats.take()
    }

    pub fn is_active(&self) -> bool { self.active.is_some() }

    fn switch_at(&mut self, next: Option<usize>, now: Instant) -> Option<usize> {
        let previous = self.active;
        if let Some(work) = previous {
            self.stats.as_mut().unwrap().phases[work].exclusive_ns +=
                now.duration_since(self.since).as_nanos() as u64;
        }
        self.active = next;
        self.since = now;
        previous
    }

    fn slots(&mut self, slots: u64) {
        let stats = self.stats.as_mut().unwrap();
        stats.orphan_slots_first.get_or_insert(slots);
        stats.orphan_slots_last = slots;
        stats.orphan_slots_max = stats.orphan_slots_max.max(slots);
    }

    pub fn begin_goto(&mut self, same_frame: bool, stop: bool, avm2: bool, slots: u64) -> Option<GotoToken> {
        if self.stats.is_none() { return None; }
        self.begin_goto_at(same_frame, stop, avm2, slots, Instant::now())
    }

    fn begin_goto_at(&mut self, same_frame: bool, stop: bool, avm2: bool, slots: u64, now: Instant) -> Option<GotoToken> {
        let stats = self.stats.as_mut()?;
        self.depth += 1;
        stats.calls += 1;
        stats.same_frame_calls += u64::from(same_frame);
        stats.stop_calls += u64::from(stop);
        stats.avm2_calls += u64::from(avm2);
        stats.max_depth = stats.max_depth.max(self.depth);
        self.slots(slots);
        Some(GotoToken { start: now, previous: self.switch_at(Some(FrameWork::Other as usize), now) })
    }

    pub fn finish_goto(&mut self, token: Option<GotoToken>, slots: u64) {
        if token.is_none() { return; }
        self.finish_goto_at(token, slots, Instant::now());
    }

    fn finish_goto_at(&mut self, token: Option<GotoToken>, slots: u64, now: Instant) {
        let Some(token) = token else { return; };
        self.switch_at(token.previous, now);
        self.depth -= 1;
        self.slots(slots);
        let ns = now.duration_since(token.start).as_nanos() as u64;
        let stats = self.stats.as_mut().unwrap();
        stats.inclusive_ns += ns;
        stats.max_call_ns = stats.max_call_ns.max(ns);
        if self.depth == 0 { stats.outer_total_ns += ns; }
    }

    pub fn enter(&mut self, work: usize) -> Option<usize> {
        if !self.is_active() { return None; }
        self.enter_at(work, Instant::now())
    }

    fn enter_at(&mut self, work: usize, now: Instant) -> Option<usize> {
        if !self.is_active() { return None; }
        self.stats.as_mut().unwrap().phases[work].calls += 1;
        self.switch_at(Some(work), now)
    }

    pub fn leave(&mut self, previous: Option<usize>) {
        if previous.is_none() { return; }
        self.leave_at(previous, Instant::now());
    }

    fn leave_at(&mut self, previous: Option<usize>, now: Instant) {
        if previous.is_some() { self.switch_at(previous, now); }
    }

    pub fn count(&mut self, counter: Counter, count: u64) {
        if let Some(work) = self.active {
            self.stats.as_mut().unwrap().phases[work].counts[counter as usize] += count;
        }
    }

    pub fn inner(&mut self, legacy_skip: bool) {
        if self.is_active() {
            let stats = self.stats.as_mut().unwrap();
            stats.inner_calls += 1;
            stats.legacy_inner_skips += u64::from(legacy_skip);
        }
    }

    pub fn queued(&mut self) {
        if let Some(stats) = self.stats.as_mut() { stats.queued_requests += 1; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn aola_goto_nested_phases_partition_outer_time() {
        let mut p = GotoProfiler::default();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        p.begin_response(true);
        let outer = p.begin_goto_at(false, true, true, 5, at(0));
        p.enter_at(FrameWork::TimelineConstruct as usize, at(1));
        p.count(Counter::ConstructVisits, 2);
        p.inner(false);
        let nested = p.begin_goto_at(true, false, true, 7, at(4));
        p.inner(true);
        let orphan = p.enter_at(FrameWork::OrphanScripts as usize, at(5));
        p.count(Counter::OrphanVisits, 7);
        let script = p.enter_at(FrameWork::ScriptExecution as usize, at(6));
        p.count(Counter::ScriptCalls, 1);
        p.leave_at(script, at(9));
        p.leave_at(orphan, at(10));
        p.finish_goto_at(nested, 3, at(11));
        let event = p.enter_at(FrameWork::FrameConstructedEvent as usize, at(13));
        p.leave_at(event, at(15));
        p.finish_goto_at(outer, 9, at(17));
        let stats = p.finish_response().unwrap();
        assert_eq!(stats.outer_total_ns, 17_000_000);
        assert_eq!(stats.phases.iter().map(|p| p.exclusive_ns).sum::<u64>(), stats.outer_total_ns);
        assert_eq!(stats.phases[FrameWork::TimelineConstruct as usize].exclusive_ns, 7_000_000);
        assert_eq!(stats.phases[FrameWork::OrphanScripts as usize].exclusive_ns, 2_000_000);
        assert_eq!(stats.phases[FrameWork::ScriptExecution as usize].exclusive_ns, 3_000_000);
        assert_eq!((stats.inclusive_ns, stats.max_call_ns, stats.max_depth), (24_000_000, 17_000_000, 2));
        assert_eq!((stats.calls, stats.same_frame_calls, stats.stop_calls, stats.avm2_calls), (2, 1, 1, 2));
        assert_eq!((stats.inner_calls, stats.legacy_inner_skips), (2, 1));
        assert_eq!((stats.orphan_slots_first, stats.orphan_slots_last, stats.orphan_slots_max), (Some(5), 9, 9));
        assert_eq!(stats.phases[FrameWork::TimelineConstruct as usize].counts[Counter::ConstructVisits as usize], 2);
        assert_eq!(stats.phases[FrameWork::ScriptExecution as usize].counts[Counter::ScriptCalls as usize], 1);
        assert_eq!(stats.phases.iter().map(|p| p.calls).sum::<u64>(), 4);
        assert!(!p.is_active());
    }

    #[test]
    fn aola_goto_disabled_and_outside_response_do_not_record() {
        let mut p = GotoProfiler::default();
        for begin_disabled in [false, true] {
            if begin_disabled { p.begin_response(false); }
            p.queued();
            p.inner(false);
            p.count(Counter::ConstructVisits, 1);
            let token = p.begin_goto(false, true, true, 5);
            assert!(token.is_none());
            let previous = p.enter(REMOVED_SCRIPTS);
            assert!(previous.is_none());
            p.leave(previous);
            p.finish_goto(token, 3);
            assert!(p.finish_response().is_none());
            assert!(!p.is_active());
        }
    }

    #[test]
    fn aola_goto_queued_requests_and_drain_are_per_response() {
        let mut p = GotoProfiler::default();
        p.begin_response(true);
        p.queued();
        p.queued();
        p.count(Counter::ScriptVisits, 9);
        let first = p.finish_response().unwrap();
        assert_eq!((first.queued_requests, first.calls), (2, 0));
        assert_eq!(first.orphan_slots_first, None);
        assert!(first.phases.iter().all(|phase| phase.counts.iter().all(|count| *count == 0)));
        assert!(p.finish_response().is_none());
        p.begin_response(true);
        let start = Instant::now();
        let token = p.begin_goto_at(true, true, false, 1, start);
        p.enter_at(REMOVED_SCRIPTS, start + Duration::from_millis(1));
        p.count(Counter::ScriptVisits, 3);
        p.finish_goto_at(token, 0, start + Duration::from_millis(2));
        let next = p.finish_response().unwrap();
        assert_eq!((next.queued_requests, next.calls, next.avm2_calls), (0, 1, 0));
        assert_eq!(next.phases[REMOVED_SCRIPTS].counts[Counter::ScriptVisits as usize], 3);
        assert_eq!(next.phases.iter().map(|p| p.exclusive_ns).sum::<u64>(), next.outer_total_ns);
        assert_eq!(work_name(REMOVED_SCRIPTS), "removedScripts");
        assert_eq!(COUNTER_NAMES.len(), Counter::OrphanGotoIdleSkipped as usize + 1);
    }
}
