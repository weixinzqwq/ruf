//! Aola-only sampling of existing engine counters; no account or script values.
use crate::aoqi_diagnostics::{PerformanceSnapshot, performance_interval, renderer_memory};
use crate::gui::PresentationStats;
use ruffle_core::Player;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use winit::window::Window;

pub struct PerformanceLog {
    file: File,
    previous: Option<(Instant, PerformanceSnapshot)>,
    movie_generation: u64,
}

impl PerformanceLog {
    pub fn new(root: &Path) -> io::Result<Self> {
        let directory = root.join("diagnostics");
        std::fs::create_dir_all(&directory)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let file = OpenOptions::new().write(true).create_new(true)
            .open(directory.join(format!("performance-{stamp}-{}.jsonl", std::process::id())))?;
        Ok(Self { file, previous: None, movie_generation: 0 })
    }

    pub fn begin_movie(&mut self) {
        // The start movie is recreated after web login. Do not compare counters
        // across players, even if the new player's frame count is already larger.
        self.previous = None;
        self.movie_generation += 1;
    }

    pub fn due(&self) -> bool {
        self.previous.is_none_or(|(time, _)| time.elapsed().as_secs() >= 5)
    }

    pub fn sample(&mut self, player: &Player, window: &Window, presentation: PresentationStats, final_sample: bool) -> io::Result<()> {
        let now = Instant::now();
        let gc = player.gc_memory_stats();
        let snapshot = PerformanceSnapshot {
            player: player.performance_stats(), presentation,
            gc_ns: gc.total_ns, gc_slices: gc.slices,
            resources: player.renderer().resource_stats().unwrap_or_default(),
        };
        let interval = self.previous.map_or(json!({"available":false,"reason":"first-sample-for-movie"}), |(time, previous)| {
            let mut interval = performance_interval(previous, snapshot, now.duration_since(time).as_secs_f64());
            interval["tickUpdate"] = tick_interval(previous.player, snapshot.player);
            interval
        });
        self.previous = Some((now, snapshot));
        let slow_callbacks = player.take_slow_tick_callbacks().into_iter().map(|row| json!({
            "stage":ruffle_core::tick_profile::STAGE_NAMES[row.stage],
            "kind":ruffle_core::tick_profile::CALLBACK_NAMES[row.kind],
            "source":row.source,"timerId":row.timer_id,"slowCalls":row.slow_calls,
            "inclusiveCpuMs":row.total_ns as f64/1e6,"maxCallMs":row.max_ns as f64/1e6,
        })).collect::<Vec<_>>();
        let (details, dropped_details) = player.take_pet_detail_profile();
        let details = details.into_iter().map(|r| json!({
            "sequence":r.sequence,"stage":r.stage.map(|i|ruffle_core::tick_profile::STAGE_NAMES[i]).unwrap_or("outsideTick"),"totalMs":r.total_ns as f64/1e6,
            "resourceCreationMs":r.resource_creation_ns as f64/1e6,"timedResourceCreations":r.resource_creations,
            "meshRegistrationMs":r.mesh_registration_ns as f64/1e6,"queueSubmitMs":r.submit_ns as f64/1e6,
            "bookkeepingMs":r.bookkeeping_ns as f64/1e6,"unlistedCalls":r.unlisted_calls,"unlistedSelfMs":r.unlisted_self_ns as f64/1e6,
            "methods":r.methods.into_iter().map(|m|json!({"category":ruffle_core::pet_detail_profile::category(&m.name),"source":m.name,"calls":m.calls,
                "inclusiveMs":m.total_ns as f64/1e6,"selfMs":m.self_ns as f64/1e6,"maxMs":m.max_ns as f64/1e6})).collect::<Vec<_>>()
        })).collect::<Vec<_>>();
        let viewport = player.renderer().viewport_dimensions();
        let size = window.inner_size();
        let mode = ruffle_core::aoqi_work::mode();
        let gc_record = json!({"phase":gc.phase,"allocationDebt":gc.allocation_debt,"allocations":gc.gc_allocations,
                "lifetimeAllocated":gc.gc_lifetime_allocated,"lifetimeDropped":gc.gc_lifetime_dropped,
                "lifetimeFreed":gc.gc_lifetime_freed,"steps":gc.steps,"slices":gc.slices,
                "totalCpuMs":gc.total_ns as f64/1e6,"maxSliceMs":gc.max_slice_ns as f64/1e6,
                "overBudgetSlices":gc.over_budget_slices,"sliceBudgetMs":gc.frame_budget_ns as f64/1e6,
                "completedCycles":gc.completed_cycles,"libraryCharacters":gc.library_characters,
                "libraryUploadedBitmaps":gc.library_uploaded_bitmaps,"libraryGraphicMeshEntries":gc.library_graphic_mesh_entries,
                "knownMovies":gc.known_movies,"knownMovieBytes":gc.known_movie_bytes,
                "libraryInventoryCompletedCycle":gc.completed_cycles,
                "libraryInventoryScope":"post-collection",
                "libraryCreated":gc.library_inventory.created,"libraryRemoved":gc.library_inventory.removed,
                "libraryMaintenanceCpuMs":gc.library_inventory.maintenance_ns as f64/1e6,
                "libraryGpuCacheChecks":gc.library_gpu_cache_sweep.checks,
                "libraryGpuMeshesReleased":gc.library_gpu_cache_sweep.mesh_entries_released,
                "libraryGpuBitmapsReleased":gc.library_gpu_cache_sweep.bitmaps_released,
                "libraryGpuSweepCpuMs":gc.library_gpu_cache_sweep.total_ns as f64/1e6,
                "orphanSlots":gc.orphan_slots});
        let row = json!({
            "schema":5,"game":"aola","build":"aola-date-compat-test-20261107",
            "unixMs":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "petPatchMode":std::env::var("AOLA_PET_PATCH").unwrap_or_else(|_|"official".into()),
            "processId":std::process::id(),"movieGeneration":self.movie_generation,"final":final_sample,
            "runtimeFrameRateSetting":player.frame_rate(),"nativeFrameRateForced":player.is_frame_rate_forced(),
            "optimizationMode":{"sleep":mode.sleep,"loading":mode.loading,"dedup":mode.dedup},
            "internalViewport":{"width":viewport.width,"height":viewport.height,"scaleFactor":viewport.scale_factor},
            "window":{"physicalWidth":size.width,"physicalHeight":size.height,"dpiScale":window.scale_factor()},
            "performanceInterval":interval,"gpu":renderer_memory(player.renderer()),
            "slowTickCallbacks":{"scope":"since-last-sample-inclusive-nested-callbacks-not-additive", "thresholdMs":5,"sourceLimit":32,"sources":slow_callbacks},
            "petDetailResponses":{"scope":"AVM2-exec-boundaries-self-excludes-instrumented-children", "responses":details,"droppedResponses":dropped_details},
            "gc":gc_record,

            "sampleCpuMs":now.elapsed().as_secs_f64()*1000.0,
        });
        serde_json::to_writer(&mut self.file, &row)?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

fn tick_interval(old: ruffle_core::PerformanceStats, now: ruffle_core::PerformanceStats) -> serde_json::Value {
    let previous = old.tick_update;
    let current = now.tick_update;
    let delta = |a: u64, b: u64| a.saturating_sub(b);
    let ms = |a: u64, b: u64| delta(a, b) as f64 / 1e6;
    let mut accounted = 0u64;
    let stages = ruffle_core::tick_profile::STAGE_NAMES.iter().enumerate().map(|(index, name)| {
        accounted += delta(current.stage_ns[index], previous.stage_ns[index]);
        json!({"name":name,"exclusiveCpuMs":ms(current.stage_ns[index],previous.stage_ns[index]),
            "calls":delta(current.stage_calls[index],previous.stage_calls[index]),
            "maxSpanMsLifetime":current.stage_max_ns[index] as f64/1e6})
    }).collect::<Vec<_>>();
    let callbacks = (0..current.callback_calls.len()).filter(|&index| current.callback_calls[index] != previous.callback_calls[index]).map(|index| json!({
        "stage":ruffle_core::tick_profile::STAGE_NAMES[index/3],
        "kind":ruffle_core::tick_profile::CALLBACK_NAMES[index%3],
        "calls":delta(current.callback_calls[index],previous.callback_calls[index]),
        "inclusiveCpuMs":ms(current.callback_ns[index],previous.callback_ns[index]),
        "slowCalls":delta(current.callback_slow_calls[index],previous.callback_slow_calls[index]),
    })).collect::<Vec<_>>();
    json!({"enabled":current.enabled,"scope":"playing-tick-exclusive-wall-time",
        "stages":stages,"accountedCpuMs":accounted as f64/1e6,
        "unaccountedCpuMs":(delta(now.tick_ns,old.tick_ns) as i128-accounted as i128) as f64/1e6,
        "callbacks":callbacks,"callbackScope":"outside-frames-inclusive-nested-not-additive",
        "timerBatches":delta(current.timer_batches,previous.timer_batches),
        "timerCatchupLimitHits":delta(current.timer_limit_hits,previous.timer_limit_hits),
        "timerMaxCallbacksPerBatchLifetime":current.timer_callbacks_max_per_batch,
        "timerMaxActiveLifetime":current.timer_active_max,
        "timerMaxLatenessMsLifetime":current.timer_lateness_max_us as f64/1000.0,
        "slowSourceRecordsDropped":delta(current.slow_records_dropped,previous.slow_records_dropped),
        "callbackBookkeepingCpuMs":ms(current.callback_bookkeeping_ns,previous.callback_bookkeeping_ns)})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_tick_intervals_reconcile_and_keep_inclusive_callbacks_separate() {
        let mut old = ruffle_core::PerformanceStats::default();
        old.tick_ns = 100_000_000;
        old.tick_update.stage_ns[1] = 100_000_000;
        old.tick_update.callback_calls[15] = 2;
        let mut now = old;
        now.tick_update.enabled = true;
        now.tick_ns += 70_000_000;
        now.tick_update.stage_ns[1] += 10_000_000;
        now.tick_update.stage_ns[5] += 60_000_000;
        now.tick_update.callback_calls[15] += 3;
        now.tick_update.callback_ns[15] += 58_000_000;
        let record = tick_interval(old, now);
        assert_eq!(record["accountedCpuMs"], 70.0);
        assert_eq!(record["unaccountedCpuMs"], 0.0);
        assert_eq!(record["callbacks"][0]["calls"], 3);
        assert_eq!(record["callbacks"][0]["inclusiveCpuMs"], 58.0);
        assert_eq!(tick_interval(now,now)["accountedCpuMs"], 0.0);
    }
}
