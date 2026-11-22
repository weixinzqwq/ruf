use crate::aoqi::TexturePoolLimits;
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Dxgi::DXGI_MEMORY_SEGMENT_GROUP;

pub(crate) fn memory_usage(
    adapter: &wgpu::Adapter, segment: DXGI_MEMORY_SEGMENT_GROUP,
) -> Option<(u64, u64)> {
    // The HAL guard keeps the native adapter alive during this read-only query.
    let adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }?;
    let info = adapter.raw_adapter().query_video_memory_info(segment).ok()?;
    Some((info.CurrentUsage, info.Budget))
}

pub(crate) struct CachePressure {
    pub configured_pool: TexturePoolLimits,
    pub reduced: bool,
    recovery_samples: u8,
    next_sample: Instant,
}

impl CachePressure {
    pub fn new(configured_pool: TexturePoolLimits) -> Self {
        Self { configured_pool, reduced: false, recovery_samples: 0, next_sample: Instant::now() }
    }

    pub fn due(&mut self, now: Instant) -> bool {
        if now < self.next_sample {
            return false;
        }
        self.next_sample = now + Duration::from_secs(2);
        true
    }

    pub fn pool_limits(&self) -> TexturePoolLimits {
        TexturePoolLimits {
            idle_bytes: if self.reduced { self.configured_pool.idle_bytes / 2 } else { self.configured_pool.idle_bytes },
            ..self.configured_pool
        }
    }

    /// Returns whether the limits changed. An unreadable/zero budget breaks
    /// the low-pressure streak and leaves the last confirmed limits in place.
    pub fn observe(&mut self, sample: Option<(u64, u64)>) -> bool {
        let previous = self.reduced;
        if let Some((usage, budget)) = sample.filter(|&(_, budget)| budget > 0) {
            let used = u128::from(usage) * 100;
            let budget = u128::from(budget);
            if used >= budget * 90 {
                self.reduced = true;
            }
            if self.reduced && used <= budget * 70 {
                self.recovery_samples += 1;
                if self.recovery_samples < 3 {
                    return false;
                }
                self.reduced = false;
            }
        }
        self.recovery_samples = 0;
        self.reduced != previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_frames_never_catch_up_by_polling_twice() {
        let mut state = CachePressure::new(TexturePoolLimits::default());
        let start = state.next_sample;
        assert!(state.due(start));
        assert!(!state.due(start + Duration::from_millis(1999)));
        let later = start + Duration::from_secs(20);
        assert!(state.due(later));
        assert!(!state.due(later));
    }

    #[test]
    fn hysteresis_survives_partial_recovery_missing_readings_and_large_counters() {
        let mut state = CachePressure::new(TexturePoolLimits { idle_bytes: 96, idle_frames: 17 });
        assert!(!state.observe(Some((89, 100))));
        assert!(state.observe(Some((90, 100))));
        assert_eq!(state.pool_limits().idle_bytes, 48);
        for sample in [Some((70, 100)), Some((70, 100)), None, Some((0, 0)),
                       Some((70, 100)), Some((71, 100)), Some((u64::MAX, u64::MAX))] {
            assert!(!state.observe(sample));
            assert!(state.reduced);
        }
        state.configured_pool.idle_bytes = 0;
        for _ in 0..2 { assert!(!state.observe(Some((0, u64::MAX)))); }
        assert!(state.observe(Some((70, 100))));
        assert_eq!(state.pool_limits().idle_bytes, 0);
        assert_eq!(state.pool_limits().idle_frames, 17);
    }
}
