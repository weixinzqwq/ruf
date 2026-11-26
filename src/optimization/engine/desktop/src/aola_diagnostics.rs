//! Aola-only sampling of existing engine counters; no account or script values.
use crate::aoqi_diagnostics::{PerformanceSnapshot, performance_interval, renderer_memory};
use crate::gui::PresentationStats;
use ruffle_core::Player;
use ruffle_core::pet_detail_profile::{DetailMode, ResponseCost};
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use winit::window::Window;

pub(super) const BUILD: &str = "swf-lifecycle-pilot-v2";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleKind { Interval, MovieEnd, ProcessEnd }

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

    pub fn sample(&mut self, player: &mut Player, window: &Window, presentation: PresentationStats, kind: SampleKind) -> io::Result<()> {
        let final_sample = kind == SampleKind::ProcessEnd;
        let now = Instant::now();
        let gc = player.gc_memory_stats();
        let ordinary_sample = player.take_ordinary_orphan_sample(
            !final_sample && std::env::var_os("AOLA_ORDINARY_ORPHANS").is_some(),
        );
        let snapshot = PerformanceSnapshot {
            player: player.performance_stats(), presentation,
            gc_ns: gc.total_ns, gc_slices: gc.slices,
            resources: player.renderer().resource_stats().unwrap_or_default(),
        };
        let interval = self.previous.map_or(json!({"available":false,"reason":"first-sample-for-movie"}), |(time, previous)| {
            let mut interval = performance_interval(previous, snapshot, now.duration_since(time).as_secs_f64());
            interval["tickUpdate"] = tick_interval(previous.player, snapshot.player);
            interval["hostPresentGaps"] = present_gaps(previous.presentation, presentation);
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
        let detail_mode = player.pet_detail_mode();
        let details = detail_records(detail_mode, details, dropped_details);
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
        let mut row = json!({
            "schema":7,"game":"aola","build":BUILD,"fluxPortBuild":"aola-flux-v1","myProPortBuild":"aola-mypro-morph-http-v1",
            "longRunBuild":"aola-weak-keys-battle-gc-v1",
            "weakDictionary":{"inserted":gc.weak_dictionary_inserted, "pruned":gc.weak_dictionary_pruned},
            "battleMaintenance":{"entries":gc.battle_entries, "exits":gc.battle_exits,
                "policy":gc.battle_cleanup_policy, "mainCycles":gc.battle_main_cycles,
                "fullFollowupCycles":gc.battle_full_followup_cycles,
                "followupSlices":gc.battle_followup_slices, "pending":gc.battle_followup_pending},
            "processMemory":crate::aoqi_host::process_memory().unwrap_or_else(|_| json!({"available":false})),
            "audioDefinitions":player.audio().sound_definition_stats().map(|s| json!({
                "registered":s.registered, "tracked":s.tracked, "activeInstances":s.active_instances,
                "encodedBytes":s.encoded_bytes, "retired":s.retired,
            })),
            "petSwitchBuild":"pet-switch-leaf-v1","detailProfile":detail_mode.label(),
            "gotoLeafFastPath":ruffle_core::goto_profile::goto_leaf_fast_path_enabled(),
            "gotoOrphanIdle":ruffle_core::goto_profile::goto_orphan_idle_enabled(),
            "gotoTargetOnly":ruffle_core::goto_profile::goto_target_only_mode(),
            "gotoProfile":ruffle_core::goto_profile::enabled(),"hostElapsedMs":crate::aola_host::elapsed_ms(),
            "subtreeScriptSleep":ruffle_core::goto_profile::subtree_script_sleep_enabled(),
            "unixMs":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "petPatchMode":std::env::var("AOLA_PET_PATCH").unwrap_or_else(|_|"official".into()),
            "processId":std::process::id(),"movieGeneration":self.movie_generation,"final":final_sample,
            "runtimeFrameRateSetting":player.frame_rate(),"nativeFrameRateForced":player.is_frame_rate_forced(),
            "optimizationMode":{"sleep":mode.sleep,"loading":mode.loading,"dedup":mode.dedup,
                "backgroundZws":mode.background_for_movie(b"ZWS"),
                "backgroundLargeCws":mode.background_for_large_cws(),
                "directBitmapGroupsRequested":std::env::var("AOLA_DIRECT_BITMAP_GROUPS").as_deref() != Ok("0"),
                "fuseGlowRequested":std::env::var("AOLA_FUSE_GLOW").as_deref() == Ok("1"),
                "socketBudgetMs":ruffle_core::socket::dispatch_budget_from_env().map_or(0, |d| d.as_millis() as u64),
                "hideTransformPortraitRequested":std::env::var("AOLA_HIDE_TRANSFORM_PORTRAIT").as_deref() == Ok("1"),
                "skipBattleBackdropRequested":std::env::var("AOLA_SKIP_BATTLE_BACKDROP").as_deref() == Ok("1"),
                "optimizeBackpackRequested":std::env::var("AOLA_OPTIMIZE_BACKPACK").as_deref() == Ok("1"),
                "longRunMaintenance":ruffle_render::backend::aola_long_run_maintenance(),
                "hideNoticeRequested":std::env::var("AOLA_HIDE_NOTICE").as_deref() == Ok("1"),
                "blockSkillRequested":std::env::var("AOLA_BLOCK_SKILL").as_deref() == Ok("1"),
                "dxgiCachePressureRequested":std::env::var("AOLA_DXGI_CACHE_PRESSURE").as_deref() == Ok("1")},
            "processResourceWorker":{
                "completedJobs":ruffle_core::aoqi_work::BACKGROUND_JOBS.load(Ordering::Relaxed),
                "completedLargeCws":ruffle_core::aoqi_work::BACKGROUND_LARGE_CWS_JOBS.load(Ordering::Relaxed),
                "totalWorkMs":ruffle_core::aoqi_work::BACKGROUND_NS.load(Ordering::Relaxed) as f64 / 1e6},
            "internalViewport":{"width":viewport.width,"height":viewport.height,"scaleFactor":viewport.scale_factor},
            "window":{"physicalWidth":size.width,"physicalHeight":size.height,"dpiScale":window.scale_factor()},
            "performanceInterval":interval,"gpu":renderer_memory(player.renderer()),
            "slowTickCallbacks":{"scope":"since-last-sample-inclusive-nested-callbacks-not-additive", "thresholdMs":5,"sourceLimit":32,"sources":slow_callbacks},
            "petDetailResponses":details,
            "gc":gc_record,

            "sampleCpuMs":now.elapsed().as_secs_f64()*1000.0,
        });
        if ruffle_core::pet_switch_frame::enabled() {
            let events: Vec<_> = player.take_pet_switch_timings(kind != SampleKind::Interval)
                .into_iter().map(pet_switch_record).collect();
            row["petSwitchTiming"] = json!({ "enabled":true, "events":events });
        }
        row["ordinaryOrphanMode"] = player.ordinary_orphan_mode().into();
        row["optimizationMode"]["texturePoolOptimizationRequested"] = (std::env::var("AOLA_BATTLE_TEXTURE_POOL").as_deref() == Ok("1")).into();
        row["optimizationMode"]["battleGlowReuseRequested"] = (std::env::var("AOLA_BATTLE_GLOW_REUSE").as_deref() == Ok("1")).into();
        row["ordinaryOrphanBranchPruneRequested"] = (std::env::var("AOLA_ORDINARY_BRANCH_PRUNE").as_deref() == Ok("1")).into();
        row["ordinaryOrphanSample"] = ordinary_sample.as_ref().map(ordinary_orphan_sample_record).into();
        row["battleMaintenance"]["inBattleGc"] = json!({
            "catchupEnabled":gc.battle_gc_catchup_enabled, "catchupSlices":gc.battle_gc_catchup_slices,
            "completedCycles":gc.battle_gc_cycles, "dropped":gc.battle_gc_dropped, "freed":gc.battle_gc_freed,
        });
        let (to_trace, to_retrace, ready_groups, root_pending) = gc.gc_mark_backlog;
        row["gcMarkBacklog"] = json!({
            "toTrace":to_trace, "toRetrace":to_retrace, "readyWeakKeyGroups":ready_groups, "rootPending":root_pending,
        });
        let _write_phase = final_sample.then(|| crate::aola_close_phases::phase("final-log-write"));
        serde_json::to_writer(&mut self.file, &row)?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

fn pet_switch_record(event: ruffle_core::pet_switch_frame::SwitchEvent) -> serde_json::Value {
    let ms = |ns: u64| ns as f64 / 1e6;
    let [dispose, remove, construct, load] = event.phases_ns.map(|phase| phase.map(ms));
    let same_frame_ms = event.render.as_ref()
        .filter(|render| matches!(render.relation, "sameLogicalFrame" | "duringRender"))
        .map(|render| ms(render.cpu_ns));
    let render = event.render.map(|render| json!({
        "id":render.id, "logicalFrame":render.logical_frame, "cpuMs":ms(render.cpu_ns), "relation":render.relation,
    }));
    json!({
        "switchId":event.id, "logicalFrameAtSwitch":event.logical_frame, "trigger":event.trigger,
        "tickStage":event.tick_stage, "switchPetCompleted":event.completed,
        "oldDisposeMs":dispose, "oldRemoveMs":remove, "newConstructMs":construct, "startLoadMs":load,
        "sameFrameRenderCpuMs":same_frame_ms, "firstRender":render,
        "presentId":event.present_id, "presentGapMs":event.present_gap_ns.map(ms), "status":event.status,
    })
}

fn present_gaps(before: PresentationStats, after: PresentationStats) -> serde_json::Value {
    let limits = PresentationStats::GAP_LIMITS_MS;
    let counts: [u64; PresentationStats::GAP_LIMITS_MS.len() + 1] =
        std::array::from_fn(|i| after.gap_counts[i] - before.gap_counts[i]);
    let samples = counts.iter().sum::<u64>();
    let percentile = |p: u64| {
        let rank = (samples * p).div_ceil(100);
        let mut sum = 0;
        counts.iter().position(|n| { sum += n; samples != 0 && sum >= rank })
            .and_then(|index| limits.get(index).copied())
    };
    json!({"upperBoundsMs":limits,"counts":counts,"samples":samples,
        "p95UpperMs":percentile(95), "p99UpperMs":percentile(99)})
}

fn detail_records(mode: DetailMode, responses: Vec<ResponseCost>, dropped: u64) -> serde_json::Value {
    let response_count = responses.len() as u64 + dropped;
    let records = responses.into_iter().map(|r| {
        let mut row = json!({
            "sequence":r.sequence, "root":r.root.map(|root| root.label()),
            "stage":r.stage.map(|i|ruffle_core::tick_profile::STAGE_NAMES[i]).unwrap_or("outsideTick"),
            "totalMs":r.total_ns as f64/1e6,
        });
        if mode == DetailMode::Full {
            let breakdown = json!({
                "resourceCreationMs":r.resource_creation_ns as f64/1e6,"timedResourceCreations":r.resource_creations,
                "meshRegistrationMs":r.mesh_registration_ns as f64/1e6,"queueSubmitMs":r.submit_ns as f64/1e6,
                "bookkeepingMs":r.bookkeeping_ns as f64/1e6,"unlistedCalls":r.unlisted_calls,"unlistedSelfMs":r.unlisted_self_ns as f64/1e6,
                "goto":goto_record(r.goto),
                "orphanCensus":r.orphan_census.as_deref().map(orphan_census_record),
                "methods":r.methods.into_iter().map(|m|json!({"category":ruffle_core::pet_detail_profile::category(&m.name),"source":m.name,"calls":m.calls,
                    "inclusiveMs":m.total_ns as f64/1e6,"selfMs":m.self_ns as f64/1e6,"maxMs":m.max_ns as f64/1e6})).collect::<Vec<_>>(),
            });
            if let serde_json::Value::Object(fields) = breakdown {
                row.as_object_mut().unwrap().extend(fields);
            }
        }
        row
    }).collect::<Vec<_>>();
    let scope = match mode {
        DetailMode::Summary => "outer-detail-root-wall-time",
        DetailMode::Full => "AVM2-exec-boundaries-self-excludes-instrumented-children",
    };
    json!({"mode":mode.label(),"methodBreakdownMeasured":mode == DetailMode::Full,"scope":scope,
        "responseCount":response_count,"responses":records,"droppedResponses":dropped})
}

fn orphan_census_record(c: &ruffle_core::orphan_census::Census) -> serde_json::Value {
    let groups: Vec<_> = c.groups.iter().map(|(g, n)| json!({
        "class":g.class, "movie":g.movie, "symbol":g.symbol, "playback":g.playback,
        "leaf":g.leaf, "sampledRoots":n
    })).collect();
    json!({
        "schema":1, "scope":"sampled-parentless-roots-after-detail",
        "sampling":"evenly-spaced-weak-slots-in-list-order",
        "sampleLimit":ruffle_core::orphan_census::SAMPLE_LIMIT,
        "groupLimit":ruffle_core::orphan_census::GROUP_LIMIT,
        "slots":c.slots,"sampled":c.sampled,"notSampled":c.slots-c.sampled,
        "validRoots":c.valid_roots,"expired":c.expired,"reparented":c.reparented,
        "unknownClass":c.unknown_class,"unknownMovie":c.unknown_movie,
        "omittedRoots":c.omitted_roots,"omittedGroups":c.omitted_groups,
        "groups":groups,"scanUs":c.scan_us
    })
}

fn ordinary_orphan_sample_record(s: &ruffle_core::orphan_census::OrdinarySnapshot) -> serde_json::Value {
    let groups: Vec<_> = s.groups.iter().map(|(g, count)| json!({
        "class":g.class, "movieGroup":g.movie, "symbol":g.symbol, "rootPlayback":g.playback,
        "hasChildren":g.children, "rootPendingScript":g.pending_script,
        "rootCurrentFrameHasScript":g.script_on_current_frame, "roots":count,
    })).collect();
    let receivers: Vec<_> = s.script_receivers.iter()
        .map(|(class, count)| json!({"class": class, "calls": count})).collect();
    json!({
        "ordinaryFrame":s.frame, "inBattle":s.in_battle, "slots":s.slots,
        "sampled":s.sampled, "invalid":s.invalid, "omittedRoots":s.omitted_roots,
        "groups":groups,
        "frameScriptReceivers": {
            "directCalls": s.script_calls,
            "callLimit": ruffle_core::orphan_census::SAMPLE_LIMIT,
            "classLimit": ruffle_core::orphan_census::GROUP_LIMIT,
            "omittedCalls": s.script_calls - s.script_receivers.values().sum::<usize>(),
            "classes": receivers,
        },
    })
}

fn goto_record(stats: Option<Box<ruffle_core::goto_profile::GotoStats>>) -> serde_json::Value {
    let Some(stats) = stats else { return json!({"enabled":false}); };
    let accounted: u64 = stats.phases.iter().map(|phase| phase.exclusive_ns).sum();
    let phases = stats.phases.iter().enumerate().filter(|(_, phase)|
        phase.calls != 0 || phase.exclusive_ns != 0 || phase.counts.iter().any(|count| *count != 0))
        .map(|(index, phase)| {
            let counts = ruffle_core::goto_profile::COUNTER_NAMES.iter().zip(phase.counts)
                .map(|(name, count)| ((*name).to_owned(), json!(count))).collect::<serde_json::Map<_, _>>();
            json!({"name":ruffle_core::goto_profile::work_name(index),"exclusiveMs":phase.exclusive_ns as f64/1e6,
                "calls":phase.calls,"counts":counts})
        }).collect::<Vec<_>>();
    json!({"enabled":true,"scope":"actual-goto-execution-within-this-detail-response",
        "phaseScope":"exclusive-wall-time-nested-gotos-switch-and-restore-parent",
        "calls":stats.calls,"sameFrameCalls":stats.same_frame_calls,"stopCalls":stats.stop_calls,"avm2Calls":stats.avm2_calls,
        "innerCalls":stats.inner_calls,"legacyInnerSkips":stats.legacy_inner_skips,
        "queuedRequests":stats.queued_requests,"maxDepth":stats.max_depth,
        "outerTotalMs":stats.outer_total_ns as f64/1e6,"accountedMs":accounted as f64/1e6,
        "unaccountedMs":(stats.outer_total_ns as i128-accounted as i128) as f64/1e6,
        "inclusiveMs":stats.inclusive_ns as f64/1e6,"inclusiveScope":"nested-gotos-overlap-do-not-add-to-phases",
        "maxCallMs":stats.max_call_ns as f64/1e6,
        "orphanSlots":{"first":stats.orphan_slots_first,"last":stats.orphan_slots_last,"maxAtGotoBoundaries":stats.orphan_slots_max},
        "counterScope":"visits-and-dispatch-attempts-not-unique-objects-or-listeners","phases":phases})
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
    fn aola_present_gap_percentiles_use_only_the_sample_interval() {
        let mut old = PresentationStats::default();
        old.gap_counts[0] = 320;
        let mut new = old;
        new.gap_counts[2] = 95;
        new.gap_counts[7] = 3;
        new.gap_counts[13] = 2;
        let row = present_gaps(old, new);
        assert_eq!(row["samples"], 100);
        assert_eq!(row["p95UpperMs"], 20);
        assert!(row["p99UpperMs"].is_null()); // The 99th interval is longer than 1 s.
        assert!(present_gaps(new, new)["p95UpperMs"].is_null());
    }
    #[test]
    fn aola_summary_serializes_only_measured_detail_fields() {
        use ruffle_core::pet_detail_profile::DetailRoot;
        let response = || ResponseCost { sequence: 9, root: Some(DetailRoot::ViewSinglePet), total_ns: 2_500_000, ..Default::default() };
        let summary = detail_records(DetailMode::Summary, vec![response()], 2);
        assert_eq!(summary["mode"], "summary");
        assert_eq!(summary["methodBreakdownMeasured"], false);
        assert_eq!(summary["responseCount"], 3);
        assert_eq!(summary["responses"][0], json!({"sequence":9,"root":"PetDataService/onViewSinglePet","stage":"outsideTick","totalMs":2.5}));
        let full = detail_records(DetailMode::Full, vec![response()], 0);
        assert_eq!(full["methodBreakdownMeasured"], true);
        assert_eq!(full["responses"][0]["bookkeepingMs"], 0.0);
        assert_eq!(full["responses"][0]["goto"]["enabled"], false);
        assert_eq!(full["responses"][0]["methods"], json!([]));
        assert_eq!(detail_records(DetailMode::Summary, vec![], 0)["mode"], "summary");
    }

    #[test]
    fn aola_goto_log_keeps_exclusive_costs_and_nested_totals_separate() {
        use ruffle_core::{frame_profile::FrameWork, goto_profile::{Counter, GotoStats}};
        let mut stats = GotoStats::default();
        stats.calls = 3;
        stats.outer_total_ns = 10_000_000;
        stats.inclusive_ns = 17_000_000;
        let phase = &mut stats.phases[FrameWork::OrphanConstruct as usize];
        phase.calls = 2;
        phase.exclusive_ns = 10_000_000;
        phase.counts[Counter::OrphanVisits as usize] = 70;
        let row = goto_record(Some(Box::new(stats)));
        assert_eq!(row["enabled"], true);
        assert_eq!(row["calls"], 3);
        assert_eq!(row["outerTotalMs"], 10.0);
        assert_eq!(row["accountedMs"], 10.0);
        assert_eq!(row["unaccountedMs"], 0.0);
        assert_eq!(row["inclusiveMs"], 17.0);
        assert_eq!(row["phases"][0]["name"], "orphanConstruct");
        assert_eq!(row["phases"][0]["counts"]["orphanVisits"], 70);
        assert_eq!(goto_record(None), json!({"enabled":false}));
    }
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
