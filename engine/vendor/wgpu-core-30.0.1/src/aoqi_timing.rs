//! Thread-local cumulative CPU timings for the native host's submit path.
//! Slots may enclose driver waits; these are not GPU execution timestamps.
pub const ENCODE: usize = 0;
pub const BUFFER_INIT: usize = 1;
pub const TEXTURE_INIT: usize = 2;
pub const BARRIERS: usize = 3;
pub const DRIVER_SUBMIT: usize = 4;
pub const MAINTAIN: usize = 5;
pub const PENDING_WRITES: usize = 6;
pub const PASS_OPEN: usize = 7;
pub const PASS_START: usize = 8;
pub const PASS_COMMANDS: usize = 9;
pub const PASS_FINISH: usize = 10;
pub const PASS_BARRIERS: usize = 11;
pub const ENCODER_CLOSE: usize = 12;
pub const ENCODER_RESET: usize = 13;
pub const FENCE_QUERY: usize = 14;
pub const MAINTAIN_LOCK: usize = 15;
pub const TRIAGE: usize = 16;
pub const TRACKER_DROP: usize = 17;
pub const TEMP_DROP: usize = 18;
pub const PENDING_DROP: usize = 19;
pub const MAPPING: usize = 20;
pub const SCOPE_DROP: usize = 21;
pub const BUFFER_DROP: usize = 22;
pub const TEXTURE_DROP: usize = 23;
pub const VIEW_DROP: usize = 24;
pub const BIND_GROUP_DROP: usize = 25;
pub const ENCODER_RECYCLE: usize = 26;
pub const PASS_TOTAL: usize = 27;
pub const PASS_PREFLIGHT: usize = 28;
pub const SLOTS: usize = 29;

pub const RENDER_PASSES: usize = 0;
pub const RENDER_COMMANDS: usize = 1;
pub const CLOSED_COMMAND_BUFFERS: usize = 2;
pub const RESET_COMMAND_BUFFERS: usize = 3;
pub const RETIRED_SUBMISSIONS: usize = 4;
pub const RETIRED_ENCODERS: usize = 5;
pub const RETIRED_STATELESS_REFS: usize = 6;
pub const STATELESS_INSERTS: usize = 7;
pub const SUBMISSION_STATELESS_REFS: usize = 8;
pub const BUFFER_DROPS: usize = 9;
pub const TEXTURE_DROPS: usize = 10;
pub const VIEW_DROPS: usize = 11;
pub const BIND_GROUP_DROPS: usize = 12;
pub const SCOPE_METADATA_SLOTS: usize = 13;
pub const SCOPE_METADATA_RELEASED: usize = 14;
pub const INLINE_RENDER_PASSES: usize = 15;
pub const SPLIT_RENDER_PASSES: usize = 16;
pub const INLINE_LIST_REUSES: usize = 17;
pub const INLINE_SCOPE_FALLBACKS: usize = 18;
pub const COUNTERS: usize = 19;

#[cfg(feature = "std")]
std::thread_local! { static FORCE_SPLIT_PASSES: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; }

// A per-thread reference path for pixel, validation and lifecycle regressions.
#[doc(hidden)]
pub fn force_split_passes_for_test(force: bool) -> bool {
    #[cfg(feature = "std")] { FORCE_SPLIT_PASSES.replace(force) }
    #[cfg(not(feature = "std"))] { let _ = force; false }
}

pub(crate) fn split_passes_for_test() -> bool {
    #[cfg(feature = "std")] { FORCE_SPLIT_PASSES.get() }
    #[cfg(not(feature = "std"))] { false }
}

#[cfg(feature = "std")]
std::thread_local! { static TIMINGS: core::cell::Cell<[u64; SLOTS]> = const { core::cell::Cell::new([0; SLOTS]) }; }
#[cfg(feature = "std")]
std::thread_local! { static COUNTS: core::cell::Cell<[u64; COUNTERS]> = const { core::cell::Cell::new([0; COUNTERS]) }; }

pub fn counts() -> [u64; COUNTERS] {
    #[cfg(feature = "std")] { COUNTS.get() }
    #[cfg(not(feature = "std"))] { [0; COUNTERS] }
}

pub(crate) fn count(slot: usize, amount: usize) {
    #[cfg(feature = "std")]
    COUNTS.with(|counts| {
        let mut values = counts.get();
        values[slot] = values[slot].saturating_add(amount as u64);
        counts.set(values);
    });
    #[cfg(not(feature = "std"))] { let _ = (slot, amount); }
}

pub fn snapshot() -> [u64; SLOTS] {
    #[cfg(feature = "std")] { TIMINGS.get() }
    #[cfg(not(feature = "std"))] { [0; SLOTS] }
}

pub(crate) struct Timer {
    #[cfg(feature = "std")]
    started: std::time::Instant,
    #[cfg(feature = "std")]
    slot: usize,
}

impl Timer {
    pub(crate) fn new(slot: usize) -> Self {
        #[cfg(feature = "std")] { Self { started: std::time::Instant::now(), slot } }
        #[cfg(not(feature = "std"))] { let _ = slot; Self {} }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        TIMINGS.with(|timings| {
            let mut values = timings.get();
            values[self.slot] = values[self.slot].saturating_add(self.started.elapsed().as_nanos() as u64);
            timings.set(values);
        });
    }
}
