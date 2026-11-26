//! Exclusive CPU accounting inside Player::run_frame. Nested goto/event/script
//! work switches buckets rather than counting the same duration twice.
use crate::context::UpdateContext;
use crate::frame_lifecycle::FramePhase;
use web_time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum FrameWork {
    Other, Preload, TimelineEnter, TimelineConstruct, ScriptTraversal, ScriptExecution,
    OrphanEnter, OrphanConstruct, OrphanScripts, OrphanCleanup,
    EnterFrameEvent, FrameConstructedEvent, ExitFrameEvent, ExitFrameLoading,
    Avm1Frame, AudioAndCallbacks, QueuedActions, PointerUpdate, ScriptCleanup,
}

pub const FRAME_WORK_NAMES: [&str; 19] = [
    "other", "preload", "timelineEnter", "timelineConstruct", "scriptTraversal", "scriptExecution",
    "orphanEnter", "orphanConstruct", "orphanScripts", "orphanCleanup",
    "enterFrameEvent", "frameConstructedEvent", "exitFrameEvent", "exitFrameLoading",
    "avm1Frame", "audioAndCallbacks", "queuedActions", "pointerUpdate", "scriptCleanup",
];

/// Outcomes of weak root visits in one ordinary AVM2 frame phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct OrdinaryOrphanStats {
    pub invalid_roots: u64,
    pub sleeping_roots: u64,
    pub scriptless_roots: u64,
    pub dispatched_roots: u64,
    /// Skips below a dispatched root, counted separately from root outcomes.
    pub sleeping_descendants: u64,
    pub scriptless_descendants: u64,
}

impl OrdinaryOrphanStats {
    pub(crate) fn phase_index(phase: FramePhase) -> Option<usize> {
        match phase {
            FramePhase::Enter => Some(0),
            FramePhase::Construct => Some(1),
            FramePhase::FrameScripts => Some(2),
            FramePhase::Exit | FramePhase::Idle => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameUpdateStats {
    pub exclusive_ns: [u64; 19],
    pub calls: [u64; 19],
    pub started_frames: u64,
    pub completed_frames: u64,
    pub preload_returns: u64,
    pub nested_gotos: u64,
    pub target_only_gotos: u64,
    pub script_calls: u64,
    pub orphan_visits: u64,
    pub orphan_executed: u64,
    pub orphan_invalid: u64,
    pub orphan_sleep_skipped: u64,
    pub orphan_goto_idle_skipped: u64,
    pub ordinary_orphans: [OrdinaryOrphanStats; 3],
    pub ordinary_orphan_script_calls: u64,
    pub broadcast_candidates: u64,
    pub broadcast_dispatched: u64,
}

pub(crate) struct FrameProfiler {
    pub stats: FrameUpdateStats,
    pub(crate) pet_switch: crate::pet_switch_frame::SwitchFrames,
    pub(crate) ordinary_sample_requested: std::cell::Cell<bool>,
    pub(crate) ordinary_sample: std::cell::RefCell<Option<crate::orphan_census::OrdinarySnapshot>>,
    active: Option<FrameWork>,
    ordinary_orphan_script_scope: bool,
    since: Instant,
}

impl Default for FrameProfiler {
    fn default() -> Self {
        Self {
            stats: FrameUpdateStats::default(),
            pet_switch: Default::default(),
            ordinary_sample_requested: Default::default(),
            ordinary_sample: Default::default(),
            active: None,
            ordinary_orphan_script_scope: false,
            since: Instant::now(),
        }
    }
}

impl FrameProfiler {
    pub fn is_active(&self) -> bool { self.active.is_some() }

    pub(crate) fn replace_ordinary_orphan_script_scope(&mut self, ordinary: bool) -> bool {
        std::mem::replace(&mut self.ordinary_orphan_script_scope, ordinary)
    }

    fn switch_at(&mut self, next: Option<FrameWork>, now: Instant) -> Option<FrameWork> {
        let previous = self.active;
        if let Some(work) = previous {
            self.stats.exclusive_ns[work as usize] += now.duration_since(self.since).as_nanos() as u64;
        }
        self.active = next;
        self.since = now;
        previous
    }

    pub fn begin_frame(&mut self) {
        debug_assert!(self.active.is_none());
        self.stats.started_frames += 1;
        self.switch_at(Some(FrameWork::Other), Instant::now());
    }

    pub fn finish_frame(&mut self, completed: bool) {
        self.switch_at(None, Instant::now());
        if completed { self.stats.completed_frames += 1; }
        else { self.stats.preload_returns += 1; }
    }

    pub fn enter(&mut self, work: FrameWork) -> Option<FrameWork> {
        if self.active.is_none() { return None; }
        self.stats.calls[work as usize] += 1;
        self.switch_at(Some(work), Instant::now())
    }

    pub(crate) fn enter_frame_script(
        &mut self, sample_receiver: impl FnOnce(&mut crate::orphan_census::OrdinarySnapshot),
    ) -> Option<FrameWork> {
        if self.ordinary_orphan_script_scope && self.active == Some(FrameWork::OrphanScripts) {
            self.stats.ordinary_orphan_script_calls += 1;
            if let Some(snapshot) = self.ordinary_sample.get_mut()
                && snapshot.frame == self.stats.started_frames
            {
                sample_receiver(snapshot);
            }
        }
        self.enter(FrameWork::ScriptExecution)
    }

    pub fn leave(&mut self, previous: Option<FrameWork>) {
        if previous.is_some() { self.switch_at(previous, Instant::now()); }
    }
}

pub(crate) fn measure<'gc, T>(context: &mut UpdateContext<'gc>, work: FrameWork, f: impl FnOnce(&mut UpdateContext<'gc>) -> T) -> T {
    let previous = context.frame_profile.enter(work);
    let goto_previous = context.tick_profile.goto.enter(work as usize);
    let result = f(context);
    context.tick_profile.goto.leave(goto_previous);
    context.frame_profile.leave(previous);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn aola_nested_profile_is_exclusive_and_restores_parent() {
        let mut p = FrameProfiler::default();
        let start = Instant::now();
        p.switch_at(Some(FrameWork::TimelineEnter), start);
        let parent = p.switch_at(Some(FrameWork::EnterFrameEvent), start + Duration::from_millis(3));
        let event = p.switch_at(Some(FrameWork::ScriptExecution), start + Duration::from_millis(5));
        p.switch_at(event, start + Duration::from_millis(12));
        p.switch_at(parent, start + Duration::from_millis(13));
        p.switch_at(None, start + Duration::from_millis(17));
        assert_eq!(p.stats.exclusive_ns[FrameWork::TimelineEnter as usize], 7_000_000);
        assert_eq!(p.stats.exclusive_ns[FrameWork::EnterFrameEvent as usize], 3_000_000);
        assert_eq!(p.stats.exclusive_ns[FrameWork::ScriptExecution as usize], 7_000_000);
        assert_eq!(p.stats.exclusive_ns.iter().sum::<u64>(), 17_000_000);
    }

    #[test]
    fn aola_profile_ignores_work_outside_run_frame_and_closes_early_return() {
        let mut p = FrameProfiler::default();
        let token = p.enter(FrameWork::EnterFrameEvent);
        p.leave(token);
        assert_eq!(p.stats.calls.iter().sum::<u64>(), 0);
        p.begin_frame();
        p.finish_frame(false);
        p.begin_frame();
        p.finish_frame(true);
        assert_eq!(p.stats.started_frames, 2);
        assert_eq!(p.stats.preload_returns, 1);
        assert_eq!(p.stats.completed_frames, 1);
        assert!(!p.is_active());
        assert_eq!(FRAME_WORK_NAMES.len(), FrameWork::ScriptCleanup as usize + 1);
    }

    #[test]
    fn aola_ordinary_orphan_script_calls_exclude_reentrant_goto_and_stage_scripts() {
        let mut profiler = FrameProfiler::default();
        profiler.begin_frame();
        profiler.ordinary_sample.replace(Some(crate::orphan_census::OrdinarySnapshot {
            frame: profiler.stats.started_frames, ..Default::default()
        }));
        let frame = profiler.enter(FrameWork::OrphanScripts);
        let outer_scope = profiler.replace_ordinary_orphan_script_scope(true);
        let outer_script = profiler.enter_frame_script(|s| s.script_calls += 1);

        let nested = profiler.enter(FrameWork::OrphanScripts);
        let previous_scope = profiler.replace_ordinary_orphan_script_scope(false);
        let nested_script = profiler.enter_frame_script(|_| panic!("nested goto receiver sampled"));
        profiler.leave(nested_script);
        profiler.replace_ordinary_orphan_script_scope(previous_scope);
        profiler.leave(nested);

        let stage = profiler.enter(FrameWork::ScriptTraversal);
        let stage_script = profiler.enter_frame_script(|_| panic!("stage receiver sampled"));
        profiler.leave(stage_script);
        profiler.leave(stage);
        profiler.leave(outer_script);
        let sibling_script = profiler.enter_frame_script(|s| s.script_calls += 1);
        profiler.leave(sibling_script);
        profiler.replace_ordinary_orphan_script_scope(outer_scope);
        profiler.leave(frame);
        profiler.finish_frame(true);

        assert_eq!(profiler.stats.calls[FrameWork::ScriptExecution as usize], 4);
        assert_eq!(profiler.stats.ordinary_orphan_script_calls, 2);
        assert_eq!(profiler.ordinary_sample.borrow().as_ref().unwrap().script_calls, 2);
        profiler.begin_frame();
        profiler.enter(FrameWork::OrphanScripts);
        profiler.replace_ordinary_orphan_script_scope(true);
        profiler.enter_frame_script(|_| panic!("receiver from a later frame sampled"));
        assert_eq!(profiler.stats.ordinary_orphan_script_calls, 3);
        profiler.finish_frame(true);
    }
}
