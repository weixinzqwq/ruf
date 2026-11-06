//! Aola opt-in tick accounting. Top-level stages partition tick wall time;
//! callback durations are inclusive observations, not additional stage time.
use std::cell::RefCell;
use web_time::Instant;

pub const STAGE_NAMES: [&str; 11] = ["other", "frames", "audioSync", "sockets", "netConnections", "timers", "streams", "audioBackend", "gc", "queuedActions", "pointerUpdate"];
pub const CALLBACK_NAMES: [&str; 3] = ["timer", "eventListener", "outerFunction"];
pub const SLOW_CALLBACK_NS: u64 = 5_000_000;
const MAX_SLOW_SOURCES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum TickWork { Other, Frames, AudioSync, Sockets, NetConnections, Timers, Streams, AudioBackend, Gc, QueuedActions, PointerUpdate }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum CallbackKind { Timer, Event, Function }

#[derive(Clone, Copy, Debug)]
pub struct TickStats {
    pub enabled: bool,
    pub stage_ns: [u64; 11],
    pub stage_calls: [u64; 11],
    pub stage_max_ns: [u64; 11],
    pub callback_calls: [u64; 33],
    pub callback_ns: [u64; 33],
    pub callback_slow_calls: [u64; 33],
    pub timer_batches: u64,
    pub timer_limit_hits: u64,
    pub timer_callbacks_max_per_batch: u64,
    pub timer_active_max: u64,
    pub timer_lateness_max_us: u64,
    pub slow_records_dropped: u64,
    pub callback_bookkeeping_ns: u64,
}

impl Default for TickStats {
    fn default() -> Self {
        Self { enabled: false, stage_ns: [0; 11], stage_calls: [0; 11], stage_max_ns: [0; 11],
            callback_calls: [0; 33], callback_ns: [0; 33], callback_slow_calls: [0; 33],
            timer_batches: 0, timer_limit_hits: 0, timer_callbacks_max_per_batch: 0,
            timer_active_max: 0, timer_lateness_max_us: 0, slow_records_dropped: 0, callback_bookkeeping_ns: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct SlowCallback {
    pub stage: usize,
    pub kind: usize,
    pub source: String,
    pub timer_id: Option<i32>,
    pub slow_calls: u64,
    pub total_ns: u64,
    pub max_ns: u64,
}

pub(crate) struct CallbackToken {
    start: Instant,
    stage: usize,
    kind: usize,
    timer_id: Option<i32>,
}

pub(crate) struct TickProfiler {
    pub stats: TickStats,
    pub detail: RefCell<crate::pet_detail_profile::DetailProfiler>,
    active: Option<TickWork>,
    since: Instant,
    callback_depth: usize,
    slow: RefCell<Vec<SlowCallback>>,
}

impl Default for TickProfiler {
    fn default() -> Self {
        Self { detail: RefCell::default(), stats: TickStats::default(), active: None, since: Instant::now(), callback_depth: 0, slow: RefCell::new(Vec::new()) }
    }
}

impl TickProfiler {
    pub fn begin(&mut self, now: Instant) {
        if !self.stats.enabled { return; }
        debug_assert!(self.active.is_none());
        self.since = now;
        self.active = Some(TickWork::Other);
        self.stats.stage_calls[0] += 1;
    }

    fn switch_at(&mut self, next: Option<TickWork>, now: Instant) {
        if let Some(stage) = self.active {
            let ns = now.duration_since(self.since).as_nanos() as u64;
            self.stats.stage_ns[stage as usize] += ns;
            self.stats.stage_max_ns[stage as usize] = self.stats.stage_max_ns[stage as usize].max(ns);
        }
        self.active = next;
        self.since = now;
        if let Some(stage) = next { self.stats.stage_calls[stage as usize] += 1; }
    }

    pub fn switch(&mut self, next: TickWork) {
        if self.active.is_some() { self.switch_at(Some(next), Instant::now()); }
    }

    pub fn enter_nonframe(&mut self, work: TickWork) -> Option<TickWork> {
        if !self.callbacks_active() { return None; }
        let previous = self.active;
        self.switch(work);
        previous
    }

    pub fn leave(&mut self, previous: Option<TickWork>) {
        if let Some(previous) = previous { self.switch(previous); }
    }

    pub fn finish(&mut self, now: Instant) {
        if self.active.is_some() { self.switch_at(None, now); }
        debug_assert_eq!(self.callback_depth, 0);
    }

    pub fn callbacks_active(&self) -> bool {
        self.active.is_some_and(|work| work != TickWork::Frames)
    }

    // Cached detail refresh can run directly from host input, outside tick().
    pub fn detail_scope_active(&self) -> bool {
        self.stats.enabled && self.active != Some(TickWork::Frames)
    }

    pub fn detail_stage(&self) -> Option<usize> { self.active.map(|s| s as usize) }

    pub fn outer_function_active(&self) -> bool { self.callbacks_active() && self.callback_depth == 0 }

    pub fn begin_callback(&mut self, kind: CallbackKind, timer_id: Option<i32>) -> Option<CallbackToken> {
        if !self.callbacks_active() { return None; }
        self.callback_depth += 1;
        Some(CallbackToken { start: Instant::now(), stage: self.active.unwrap() as usize, kind: kind as usize, timer_id })
    }

    pub fn finish_callback(&mut self, token: Option<CallbackToken>, source: impl FnOnce() -> String) {
        let Some(token) = token else { return; };
        let end = Instant::now();
        self.record_callback(token.stage, token.kind, token.timer_id, end.duration_since(token.start).as_nanos() as u64, source);
        self.callback_depth -= 1;
        self.stats.callback_bookkeeping_ns += end.elapsed().as_nanos() as u64;
    }

    fn record_callback(&mut self, stage: usize, kind: usize, timer_id: Option<i32>, ns: u64, source: impl FnOnce() -> String) {
        let index = stage * 3 + kind;
        self.stats.callback_calls[index] += 1;
        self.stats.callback_ns[index] += ns;
        if ns < SLOW_CALLBACK_NS { return; }
        self.stats.callback_slow_calls[index] += 1;
        let source = source();
        let rows = self.slow.get_mut();
        if let Some(row) = rows.iter_mut().find(|r| r.stage == stage && r.kind == kind && r.timer_id == timer_id && r.source == source) {
            row.slow_calls += 1;
            row.total_ns += ns;
            row.max_ns = row.max_ns.max(ns);
            return;
        }
        let row = SlowCallback { stage, kind, source, timer_id, slow_calls: 1, total_ns: ns, max_ns: ns };
        if rows.len() < MAX_SLOW_SOURCES { rows.push(row); }
        else {
            let (index, minimum) = rows.iter().enumerate().min_by_key(|(_, r)| r.max_ns).unwrap();
            if ns > minimum.max_ns {
                self.stats.slow_records_dropped += minimum.slow_calls;
                rows[index] = row;
            } else { self.stats.slow_records_dropped += 1; }
        }
    }

    pub fn take_slow(&self) -> Vec<SlowCallback> {
        let mut rows = std::mem::take(&mut *self.slow.borrow_mut());
        rows.sort_by_key(|r| std::cmp::Reverse(r.max_ns));
        rows
    }
}

/// Compiled metadata only. No object values, arguments, full URLs or query strings.
pub(crate) fn avm2_source(function: crate::avm2::object::FunctionObject<'_>) -> String {
    let exec = function.executable();
    let method = exec.as_method();
    let movie = method.owner_movie();
    let leaf = resource_leaf(movie.url());
    let name: String = exec.debug_full_name().to_utf8_lossy().chars().take(192).collect();
    format!("{leaf} abc#{} {name}", method.abc_method_index())
}

fn resource_leaf(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("")
        .chars().take(128).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn aola_detail_scope_includes_host_input_without_changing_tick_callbacks() {
        let mut p = TickProfiler::default();
        assert!(!p.detail_scope_active());
        p.stats.enabled = true;
        assert!(p.detail_scope_active());
        assert!(!p.callbacks_active());
        p.begin(Instant::now()); p.switch(TickWork::Frames);
        assert!(!p.detail_scope_active());
        p.finish(Instant::now());
        assert!(p.detail_scope_active()); assert!(!p.callbacks_active());
    }

    #[test]
    fn aola_tick_stages_partition_the_complete_tick() {
        let mut p = TickProfiler::default();
        p.stats.enabled = true;
        let start = Instant::now();
        p.begin(start);
        p.switch_at(Some(TickWork::Frames), start + Duration::from_millis(2));
        p.switch_at(Some(TickWork::Timers), start + Duration::from_millis(7));
        p.switch_at(Some(TickWork::PointerUpdate), start + Duration::from_millis(10));
        p.switch_at(Some(TickWork::Timers), start + Duration::from_millis(14));
        p.finish(start + Duration::from_millis(19));
        assert_eq!(p.stats.stage_ns.iter().sum::<u64>(), 19_000_000);
        assert_eq!(p.stats.stage_ns[TickWork::Timers as usize], 8_000_000);
        assert_eq!(p.stats.stage_ns[TickWork::PointerUpdate as usize], 4_000_000);
        assert!(!p.callbacks_active());
    }

    #[test]
    fn aola_slow_callbacks_are_bounded_lazy_and_drained() {
        let mut p = TickProfiler::default();
        p.record_callback(5, 0, Some(1), 1, || panic!("fast callback should not format metadata"));
        for id in 0..40 { p.record_callback(5, 0, Some(id), SLOW_CALLBACK_NS + id as u64, || "compiled function".into()); }
        let rows = p.take_slow();
        assert_eq!(rows.len(), MAX_SLOW_SOURCES);
        assert_eq!(p.stats.callback_calls[15], 41);
        assert_eq!(p.stats.callback_slow_calls[15], 40);
        assert_eq!(p.stats.slow_records_dropped, 8);
        assert!(p.take_slow().is_empty());
        assert_eq!(resource_leaf("https://host/private/model.swf?sid=secret#token"), "model.swf");
    }

    #[test]
    fn aola_nested_callbacks_restore_outer_call_detection() {
        let mut p = TickProfiler::default();
        assert!(p.begin_callback(CallbackKind::Timer, None).is_none());
        p.stats.enabled = true;
        p.begin(Instant::now());
        p.switch(TickWork::Frames);
        assert!(p.begin_callback(CallbackKind::Event, None).is_none());
        p.switch(TickWork::Timers);
        let parent = p.begin_callback(CallbackKind::Timer, Some(1));
        assert!(!p.outer_function_active());
        let child = p.begin_callback(CallbackKind::Event, None);
        p.finish_callback(child, || "event".into());
        assert!(!p.outer_function_active());
        p.finish_callback(parent, || "timer".into());
        assert!(p.outer_function_active());
        p.finish(Instant::now());
        assert_eq!(p.stats.callback_calls[15], 1);
        assert_eq!(p.stats.callback_calls[16], 1);
    }

    #[test]
    fn aola_tick_profile_preserves_real_timer_events_and_complete_trace() {
        use std::sync::{Arc, Mutex};
        struct Trace(Arc<Mutex<Vec<String>>>);
        impl crate::backend::log::LogBackend for Trace {
            fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
            fn avm_warning(&self, _: &str) {}
        }
        for enabled in [false, true] {
            let movie = crate::tag_utils::SwfMovieData::from_data(
                include_bytes!("../../tests/tests/swfs/avm2/timer_events/test.swf"),
                "file:///timer_events.swf".into(), None, None).unwrap();
            let trace = Arc::new(Mutex::new(Vec::new()));
            let player = crate::PlayerBuilder::new().with_movie(movie).with_autoplay(true).with_log(Trace(trace.clone())).build();
            let mut player = player.lock().unwrap();
            if enabled { player.enable_tick_profiling(); }
            for _ in 0..20 { player.tick(ruffle_common::duration::FloatDuration::from_millis(40.0)); }
            let expected = include_str!("../../tests/tests/swfs/avm2/timer_events/output.txt").replace("\r\n", "\n");
            assert_eq!(trace.lock().unwrap().join("\n"), expected.trim_end());
            let stats = player.performance_stats();
            if enabled {
                assert_eq!(stats.tick_update.stage_ns.iter().sum::<u64>(), stats.tick_ns);
                assert!(stats.tick_update.callback_calls[15] > 0);
                assert_eq!(stats.tick_update.callback_calls[16], 3);
                assert!(stats.tick_update.stage_calls[TickWork::PointerUpdate as usize] > 0);
                assert!(stats.tick_update.stage_calls[TickWork::QueuedActions as usize] > 0);
            } else { assert_eq!(stats.tick_update.callback_calls.iter().sum::<u64>(), 0); }
        }
    }
}
