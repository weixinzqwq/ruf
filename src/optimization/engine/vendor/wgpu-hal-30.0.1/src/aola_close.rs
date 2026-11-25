//! CPU-side release intervals for the native Windows close diagnostic.
#[cfg(windows)]
extern crate std;

#[cfg(windows)]
fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("AOLA_CLOSE_PHASES").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

#[cfg(not(windows))]
fn enabled() -> bool {
    false
}

#[derive(Debug)]
#[must_use]
pub struct Phase(Option<&'static str>);

impl Phase {
    pub fn begin(phase: &'static str) -> Self {
        if enabled() {
            tracing::info!(target: "aola_close_phase", phase, edge = "begin");
            Self(Some(phase))
        } else {
            Self(None)
        }
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        if let Some(phase) = self.0 {
            tracing::info!(target: "aola_close_phase", phase, edge = "end");
        }
    }
}

// The recorder owns the single normal-shutdown session. Per-thread counters
// remain monotonic for taking deltas across nested and overlapping phases.
#[cfg(dx12)]
mod resources {
    use super::std;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
    pub(super) static PROCESS: Mutex<[(u64, u64); 4]> = Mutex::new([(0, 0); 4]);
    std::thread_local! {
        pub(super) static LOCAL: [Cell<(u64, u64)>; 4] = const { [const { Cell::new((0, 0)) }; 4] };
    }

    #[repr(usize)]
    pub(crate) enum Resource { Buffer, Texture, TextureView, BindGroup }

    #[inline]
    pub(crate) fn destroy<T>(kind: Resource, resource: T, release: impl FnOnce(T)) {
        if !ACTIVE.load(Ordering::Relaxed) {
            release(resource);
            return;
        }
        let start = std::time::Instant::now();
        // Ownership crosses the closure boundary, so this includes the ordinary
        // implicit drops of all remaining fields after the destruction body.
        release(resource);
        let ns = start.elapsed().as_nanos() as u64;
        let i = kind as usize;
        LOCAL.with(|cells| {
            let (count, total_ns) = cells[i].get();
            cells[i].set((count + 1, total_ns + ns));
        });
        // Keep each count/time pair and the finish snapshot coherent across
        // threads. The reporting lock is outside the measured driver work.
        let mut totals = PROCESS.lock().unwrap();
        if ACTIVE.load(Ordering::Relaxed) {
            totals[i].0 += 1;
            totals[i].1 += ns;
        }
    }
}

#[cfg(dx12)]
pub(crate) use resources::{destroy, Resource};

pub fn aola_set_close_resource_timing(enabled: bool) {
    #[cfg(dx12)]
    {
        use core::sync::atomic::Ordering;
        let mut totals = resources::PROCESS.lock().unwrap();
        if enabled && !resources::ACTIVE.load(Ordering::Relaxed) {
            *totals = [(0, 0); 4];
        }
        resources::ACTIVE.store(enabled, Ordering::Release);
    }
    #[cfg(not(dx12))]
    let _ = enabled;
}

/// Completed DX12 (destroy calls, CPU nanoseconds) on this thread, in the
/// order buffer, texture, texture view, bind group.
pub fn aola_close_resource_totals() -> [(u64, u64); 4] {
    #[cfg(dx12)]
    return resources::LOCAL.with(|cells| core::array::from_fn(|i| cells[i].get()));
    #[cfg(not(dx12))]
    [(0, 0); 4]
}

/// Completed destruction work on all threads since the recorder enabled timing.
pub fn aola_close_resource_process_totals() -> [(u64, u64); 4] {
    #[cfg(dx12)]
    return *resources::PROCESS.lock().unwrap();
    #[cfg(not(dx12))]
    [(0, 0); 4]
}

#[cfg(all(test, dx12))]
mod tests {
    use super::*;
    use std::{thread, time::Duration};

    #[test]
    fn close_resource_timing_includes_implicit_drop_and_worker_threads() {
        struct DelayedRelease;
        impl Drop for DelayedRelease {
            fn drop(&mut self) {
                thread::sleep(Duration::from_millis(4));
            }
        }

        let baseline = aola_close_resource_totals();
        destroy(Resource::Buffer, (), |_| {});
        assert_eq!(aola_close_resource_totals(), baseline);
        aola_set_close_resource_timing(true);
        // No explicit drop in the callback, just as for remaining view fields.
        destroy(Resource::TextureView, DelayedRelease, |_| {});
        let worker = thread::spawn(|| {
            destroy(Resource::Texture, (), |_| {});
            destroy(Resource::BindGroup, DelayedRelease, |_| {});
            aola_close_resource_totals()
        }).join().unwrap();
        aola_set_close_resource_timing(false);
        let local = aola_close_resource_totals();
        let process = aola_close_resource_process_totals();
        assert_eq!(local[2].0 - baseline[2].0, 1);
        assert!(local[2].1 - baseline[2].1 >= 4_000_000);
        assert_eq!(worker.map(|(count, _)| count), [0, 1, 0, 1]);
        assert!(worker[3].1 >= 4_000_000);
        for i in 0..4 {
            assert_eq!(process[i], (
                local[i].0 - baseline[i].0 + worker[i].0,
                local[i].1 - baseline[i].1 + worker[i].1,
            ));
        }
        destroy(Resource::Buffer, (), |_| {});
        assert_eq!(aola_close_resource_totals(), local);
        assert_eq!(aola_close_resource_process_totals(), process);
    }
}
