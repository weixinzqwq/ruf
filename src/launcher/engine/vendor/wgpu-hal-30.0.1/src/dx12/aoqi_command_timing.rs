//! CPU time inside command-list opening, not GPU execution time.
std::thread_local! {
    static VALUES: [core::cell::Cell<u64>; 6] = const { [const { core::cell::Cell::new(0) }; 6] };
}

/// Reset/create/name/CPU-state-clear nanoseconds, then reset/create call counts.
pub fn command_recording_stats() -> [u64; 6] {
    VALUES.with(|v| core::array::from_fn(|i| v[i].get()))
}

pub(super) fn count(slot: usize) {
    VALUES.with(|v| v[slot].set(v[slot].get().saturating_add(1)));
}

pub(super) struct Timer {
    started: std::time::Instant,
    slot: usize,
}

impl Timer {
    pub(super) fn new(slot: usize) -> Self {
        Self { started: std::time::Instant::now(), slot }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        VALUES.with(|v| v[self.slot].set(v[self.slot].get().saturating_add(self.started.elapsed().as_nanos() as u64)));
    }
}
