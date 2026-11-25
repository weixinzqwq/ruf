//! Local numeric diagnostics only. No account, URL, cookie, or message contents.
use crate::aoqi_storage::Storage;
use ruffle_core::{PerformanceStats, Player};
use ruffle_render::backend::{RenderBackend, RenderResourceCategoryStats, RenderResourceStats};
use crate::gui::PresentationStats;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use winit::window::Window;

pub fn fixed_profile() -> Value {
    json!({"name":"command-batch-test-20261105","backend":"dx12","internalWidth":1200,
        "internalHeight":700,"quality":"high","msaaSamples":4,"bitmapUpscale":"off",
        "frameRate":60,"timeScale":1,"windowScale":1,"muted":true})
}

fn category(value: RenderResourceCategoryStats) -> Value {
    json!({"liveCount":value.live_count,"liveAllocationBytes":value.live_allocation_bytes,
        "peakAllocationBytes":value.peak_allocation_bytes,"createdAllocationBytes":value.created_allocation_bytes,
        "releasedAllocationBytes":value.released_allocation_bytes,"liveWrapperCount":value.live_wrapper_count,
        "bindingReferences":value.binding_references,"bindingOnlyAllocationBytes":value.binding_only_allocation_bytes,"liveEstimatedBytes":value.live_estimated_bytes,
        "peakCount":value.peak_count,"peakEstimatedBytes":value.peak_estimated_bytes,
        "createdCount":value.created_count,"releasedCount":value.released_count,
        "createdEstimatedBytes":value.created_estimated_bytes,"releasedEstimatedBytes":value.released_estimated_bytes,
        "timedCreationCount":value.timed_creation_count,"creationCpuMs":value.creation_cpu_ns as f64/1e6,
        "maxCreationCpuMs":value.max_creation_cpu_ns as f64/1e6})
}

pub fn renderer_memory(renderer: &dyn RenderBackend) -> Value {
    let resources = renderer.resource_stats().map(|s| json!({
        "uploadedBitmap":category(s.uploaded_bitmap),"offscreenBitmap":category(s.offscreen_bitmap),
        "bitmapCache":category(s.bitmap_cache),"temporaryTexture":category(s.temporary_texture),
        "readbackBuffer":category(s.readback_buffer),
        "mesh":category(s.mesh),"meshVertexBuffer":category(s.mesh_vertex_buffer),
        "meshIndexBuffer":category(s.mesh_index_buffer),"meshUniformBuffer":category(s.mesh_uniform_buffer),
        "gradientTexture":category(s.gradient_texture),
        "cacheBudgetRejectedCount":s.cache_budget_rejected_count,
        "cacheBudgetRejectedEstimatedBytes":s.cache_budget_rejected_estimated_bytes,
        "cacheAllocationFailedCount":s.cache_allocation_failed_count,
        "cacheAllocationFailedEstimatedBytes":s.cache_allocation_failed_estimated_bytes,
        "temporaryPoolHits":s.temporary_pool_hits,"temporaryPoolMisses":s.temporary_pool_misses,
        "temporaryPoolPressureEvictions":s.temporary_pool_pressure_evictions,
        "temporaryPoolReturnRejections":s.temporary_pool_return_rejections,
        "temporaryPoolProbationRejections":s.temporary_pool_probation_rejections,
        "temporaryPoolRecurrentMisses":s.temporary_pool_recurrent_misses,
        "temporaryPoolIdleLimitBytes":s.temporary_pool_idle_limit_bytes,
        "cachePressureEvictions":s.cache_pressure_evictions,"cacheIdleEvictions":s.cache_idle_evictions,
        "cacheReservedBudgetBytes":s.cache_reserved_bytes,
        "temporaryPoolIdleAllocationBytes":s.temporary_pool_idle_allocation_bytes,
        "temporaryPoolIdleCount":s.temporary_pool_idle_count,"temporaryPoolBuckets":s.temporary_pool_buckets
    }));
    let driver = renderer.driver_stats().map(|s| json!({
        "buffers":s.buffers,"textures":s.textures,"textureViews":s.texture_views,
        "bindGroups":s.bind_groups,"commandEncoders":s.command_encoders,
        "bufferEstimatedBytes":s.buffer_bytes,"textureEstimatedBytes":s.texture_bytes,
        "nativeBuffers":s.native_buffers.map(native_category),
        "nativeCopyBuffers":s.native_copy_buffers.map(native_category),"nativeSamplerBuffers":s.native_sampler_buffers.map(native_category),"nativeTextures":s.native_textures.map(native_category),
        "localUsage":s.local_usage,"localBudget":s.local_budget,
        "nonLocalUsage":s.non_local_usage,"nonLocalBudget":s.non_local_budget,
        "measurement":"wgpu-hal-all-resource-counters-and-dxgi-process-segment-usage"}));
    json!({"available":resources.is_some(),"renderer":renderer.name(),"diagnostics":renderer.debug_info(),"resources":resources,
        "driver":driver,"measurement":"owner-native-allocation-requirements-with-descriptor-fallback-not-residency", "totalGpuBytes":Value::Null})
}

fn native_category(s: RenderResourceCategoryStats) -> Value {
    json!({"liveCount":s.live_count,"createdCount":s.created_count,"releasedCount":s.released_count,
        "liveAllocationBytes":s.live_allocation_bytes,"peakAllocationBytes":s.peak_allocation_bytes,
        "createdAllocationBytes":s.created_allocation_bytes,"releasedAllocationBytes":s.released_allocation_bytes,
        "measurement":"dx12-backend-allocations-until-hal-free-excludes-imports-swapchain-zero-helper-not-residency"})
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PerformanceSnapshot {
    pub(crate) player: PerformanceStats,
    pub(crate) presentation: PresentationStats,
    pub(crate) gc_ns: u64,
    pub(crate) gc_slices: u64,
    pub(crate) resources: RenderResourceStats,
}

fn creation_totals(stats: RenderResourceStats) -> (u64, u64) {
    // Keep the schema9 timing scope comparable; geometry is reported separately.
    [stats.uploaded_bitmap,stats.offscreen_bitmap,stats.bitmap_cache,stats.temporary_texture,stats.readback_buffer]
        .iter().fold((0,0), |(count,ns),c| (count+c.timed_creation_count,ns+c.creation_cpu_ns))
}

fn geometry_creation_totals(stats: RenderResourceStats) -> (u64, u64) {
    [stats.mesh_vertex_buffer,stats.mesh_index_buffer,stats.mesh_uniform_buffer,stats.gradient_texture]
        .iter().fold((0,0), |(count,ns),c| (count+c.timed_creation_count,ns+c.creation_cpu_ns))
}

pub(crate) fn performance_interval(before: PerformanceSnapshot, after: PerformanceSnapshot, seconds: f64) -> Value {
    if seconds <= 0.0 || !seconds.is_finite() || after.player.frames < before.player.frames || after.gc_ns < before.gc_ns {
        return json!({"available":false,"reason":"counter-reset-or-empty-interval"});
    }
    let delta = |a: u64, b: u64| a.saturating_sub(b);
    let (geometry_created,geometry_ns)=geometry_creation_totals(after.resources);
    let (old_geometry_created,old_geometry_ns)=geometry_creation_totals(before.resources);
    let ms = |a, b| delta(a,b) as f64/1e6;
    let average = |ns: u64, count: u64| if count==0 {Value::Null} else {json!(ns as f64/1e6/count as f64)};
    let p=after.player; let old=before.player;
    let frame_stages: Vec<_> = ruffle_core::frame_profile::FRAME_WORK_NAMES.iter().enumerate().map(|(i, name)| json!({
        "name":name,"exclusiveCpuMs":ms(p.frame_update.exclusive_ns[i],old.frame_update.exclusive_ns[i]),
        "calls":delta(p.frame_update.calls[i],old.frame_update.calls[i]),
    })).collect();
    let ordinary_phase = |i: usize| {
        let now = p.frame_update.ordinary_orphans[i];
        let previous = old.frame_update.ordinary_orphans[i];
        json!({
            "invalidRoots": delta(now.invalid_roots, previous.invalid_roots),
            "sleepingRoots": delta(now.sleeping_roots, previous.sleeping_roots),
            "scriptlessRoots": delta(now.scriptless_roots, previous.scriptless_roots),
            "dispatchedRoots": delta(now.dispatched_roots, previous.dispatched_roots),
            "sleepingDescendants": delta(now.sleeping_descendants, previous.sleeping_descendants),
            "scriptlessDescendants": delta(now.scriptless_descendants, previous.scriptless_descendants),
        })
    };
    let frame_update = json!({
        "scope":"run_frame-exclusive-including-preload-returns", "stages":frame_stages,
        "startedFrames":delta(p.frame_update.started_frames,old.frame_update.started_frames),
        "completedFrames":delta(p.frame_update.completed_frames,old.frame_update.completed_frames),
        "preloadReturns":delta(p.frame_update.preload_returns,old.frame_update.preload_returns),
        "nestedGotos":delta(p.frame_update.nested_gotos,old.frame_update.nested_gotos),
        "targetOnlyGotos":delta(p.frame_update.target_only_gotos,old.frame_update.target_only_gotos),
        "scriptCalls":delta(p.frame_update.script_calls,old.frame_update.script_calls),
        "orphanVisits":delta(p.frame_update.orphan_visits,old.frame_update.orphan_visits),
        "orphanExecuted":delta(p.frame_update.orphan_executed,old.frame_update.orphan_executed),
        "orphanInvalid":delta(p.frame_update.orphan_invalid,old.frame_update.orphan_invalid),
        "orphanSleepSkipped":delta(p.frame_update.orphan_sleep_skipped,old.frame_update.orphan_sleep_skipped),
        "orphanGotoIdleSkipped":delta(p.frame_update.orphan_goto_idle_skipped,old.frame_update.orphan_goto_idle_skipped),
        "ordinaryOrphans": {
            "enter": ordinary_phase(0), "construct": ordinary_phase(1), "scripts": ordinary_phase(2),
            "directFrameScriptCalls": delta(p.frame_update.ordinary_orphan_script_calls, old.frame_update.ordinary_orphan_script_calls),
        },
        "broadcastCandidates":delta(p.frame_update.broadcast_candidates,old.frame_update.broadcast_candidates),
        "broadcastDispatched":delta(p.frame_update.broadcast_dispatched,old.frame_update.broadcast_dispatched),
    });
    let present=after.presentation; let old_present=before.presentation;
    let hits=delta(after.resources.temporary_pool_hits,before.resources.temporary_pool_hits);
    let misses=delta(after.resources.temporary_pool_misses,before.resources.temporary_pool_misses);
    let (created,creation_ns)=creation_totals(after.resources);
    let (old_created,old_creation_ns)=creation_totals(before.resources);
    let w=after.resources.work; let old_w=before.resources.work;
    let filters: Vec<_> = ["colorMatrix","blur","glow","dropShadow","bevel","displacementMap","shader","gradient","other"]
        .iter().enumerate().map(|(i,name)| json!({"name":name,
            "calls":delta(w.filter_calls[i],old_w.filter_calls[i]),
            "cpuMs":ms(w.filter_ns[i],old_w.filter_ns[i])})).collect();
    let kinds = ["msaaColor","singleSampleColor","stencil","other"];
    let areas = ["upTo64Squared","upTo256Squared","upTo512Squared","belowStageArea","stageAreaOrLarger"];
    let temporary_allocations: Vec<_> = kinds
        .iter().enumerate().map(|(i,name)| json!({"name":name,
            "created":delta(w.temporary_created[i],old_w.temporary_created[i]),
            "createdTexels":delta(w.temporary_created_texels[i],old_w.temporary_created_texels[i]),
            "createdEstimatedBytes":delta(w.temporary_created_bytes[i],old_w.temporary_created_bytes[i]),
            "creationCpuMs":ms(w.temporary_creation_ns[i],old_w.temporary_creation_ns[i])})).collect();
    let mut size_requests = serde_json::Map::new();
    let mut pool_misses = serde_json::Map::new();
    for (bin, area) in areas.iter().enumerate() {
        size_requests.insert((*area).to_owned(), json!(delta(w.temporary_size_requests[bin], old_w.temporary_size_requests[bin])));
    }
    for (kind, name) in kinds.iter().enumerate() {
        let mut counts = serde_json::Map::new();
        for (bin, area) in areas.iter().enumerate() {
            counts.insert((*area).to_owned(), json!({
                "missingBucket":delta(w.temporary_pool_missing_bucket[kind][bin],old_w.temporary_pool_missing_bucket[kind][bin]),
                "emptyBucket":delta(w.temporary_pool_empty_bucket[kind][bin],old_w.temporary_pool_empty_bucket[kind][bin]),
            }));
        }
        pool_misses.insert((*name).to_owned(), Value::Object(counts));
    }
    let mut render_stages = json!({
            "eventMs":ms(p.render_event_ns,old.render_event_ns),
            "sceneBuildMs":ms(p.render_build_ns,old.render_build_ns),
            "backendMs":ms(p.render_submit_ns,old.render_submit_ns),
            "targetAcquireMs":ms(w.target_acquire_ns,old_w.target_acquire_ns),
            "cacheDrawMs":ms(w.cache_draw_ns,old_w.cache_draw_ns),
            "cacheFilterMs":ms(w.cache_filter_ns,old_w.cache_filter_ns),
            "mainDrawMs":ms(w.main_draw_ns,old_w.main_draw_ns),
            "queueSubmitMs":ms(w.queue_submit_ns,old_w.queue_submit_ns),
            "stagingFinishMs":ms(w.staging_finish_ns,old_w.staging_finish_ns),
            "commandFinishMs":ms(w.command_finish_ns,old_w.command_finish_ns),
            "encoderCreateMs":ms(w.encoder_create_ns,old_w.encoder_create_ns),
            "queueExecuteMs":ms(w.queue_execute_ns,old_w.queue_execute_ns),
            "stagingRecallMs":ms(w.staging_recall_ns,old_w.staging_recall_ns),
            "cleanupMs":ms(w.cleanup_ns,old_w.cleanup_ns),
            "cacheLookupMs":ms(w.cache_lookup_ns,old_w.cache_lookup_ns),
            "temporaryPoolLookupMs":ms(w.temporary_pool_lookup_ns,old_w.temporary_pool_lookup_ns),
            "meshRegistrationMs":ms(w.mesh_registration_ns,old_w.mesh_registration_ns)});
    for (i,key) in [
        "nativeCommandEncodeMs",
        "nativeBufferInitMs",
        "nativeTextureInitMs",
        "nativeBarrierMs",
        "nativeDriverSubmitMs",
        "nativeMaintainMs",
        "nativePendingWritesMs",
        "nativePassOpenMs",
        "nativePassStartMs",
        "nativePassCommandsMs",
        "nativePassFinishMs",
        "nativePassBarriersMs",
        "nativeEncoderCloseMs",
        "nativeEncoderResetMs",
        "nativeFenceQueryMs",
        "nativeMaintainLockMs",
        "nativeTriageMs",
        "nativeTrackerDropMs",
        "nativeTempDropMs",
        "nativePendingDropMs",
        "nativeMappingMs",
        "nativeScopeDropMs",
        "nativeBufferDropMs",
        "nativeTextureDropMs",
        "nativeViewDropMs",
        "nativeBindGroupDropMs",
        "nativeEncoderRecycleMs",
        "nativeRenderPassTotalMs",
        "nativePassPreflightMs",
    ].into_iter().enumerate() {
        render_stages[key] = json!(ms(w.native_submit_ns[i],old_w.native_submit_ns[i]));
    }
    for (i,key) in ["dx12CommandListResetMs","dx12CommandListCreateMs","dx12CommandListNameMs","dx12CommandStateClearMs"].into_iter().enumerate() {
        render_stages[key] = json!(ms(w.native_command_recording[i],old_w.native_command_recording[i]));
    }
    let mut report = json!({"available":true,"seconds":seconds,"filterWork":filters,"temporaryAllocations":temporary_allocations,
        "filterSetup":{"uniformCacheHits":delta(w.filter_uniform_cache_hits,old_w.filter_uniform_cache_hits),
            "uniformCacheMisses":delta(w.filter_uniform_cache_misses,old_w.filter_uniform_cache_misses),
            "fusedGlowPasses":delta(w.fused_glow_passes,old_w.fused_glow_passes),
            "uniformCacheEvictions":delta(w.filter_uniform_cache_evictions,old_w.filter_uniform_cache_evictions),
            "uniformUploads":delta(w.filter_uniform_uploads,old_w.filter_uniform_uploads),
            "uniformPrepareMs":ms(w.filter_uniform_prepare_ns,old_w.filter_uniform_prepare_ns),
            "bindGroupMs":ms(w.filter_bind_group_ns,old_w.filter_bind_group_ns),
            "bindGroupReuses":delta(w.filter_bind_group_reuses,old_w.filter_bind_group_reuses),
            "passEncodeMs":ms(w.filter_pass_encode_ns,old_w.filter_pass_encode_ns),
            "blurPasses":delta(w.blur_passes,old_w.blur_passes),
            "glowStaticVertexDraws":delta(w.glow_static_vertex_draws,old_w.glow_static_vertex_draws),
            "glowVertexUploads":delta(w.glow_vertex_uploads,old_w.glow_vertex_uploads)},
        "blendArea":{"targets":delta(w.blend_targets,old_w.blend_targets),
            "boundedTargets":delta(w.bounded_blend_targets,old_w.bounded_blend_targets),
            "fullTargetPixels":delta(w.blend_full_pixels,old_w.blend_full_pixels),
            "allocatedTargetPixels":delta(w.blend_target_pixels,old_w.blend_target_pixels),
            "paddedTargets":delta(w.blend_padded_targets,old_w.blend_padded_targets),
            "paddingPixels":delta(w.blend_padding_pixels,old_w.blend_padding_pixels),
            "parentCopyPixels":delta(w.blend_parent_copy_pixels,old_w.blend_parent_copy_pixels)},
        "temporarySizeRequests":size_requests,
        "logicalFps":delta(p.frames,old.frames) as f64/seconds,
        "presentedFps":delta(present.frames,old_present.frames) as f64/seconds,
        "tickCpuMs":ms(p.tick_ns,old.tick_ns),"averageTickMs":average(delta(p.tick_ns,old.tick_ns),delta(p.ticks,old.ticks)),
        "frameCpuMs":ms(p.frame_ns,old.frame_ns),"averageFrameMs":average(delta(p.frame_ns,old.frame_ns),delta(p.frames,old.frames)),
        "frameUpdate":frame_update,
        "renderCpuMs":ms(p.render_ns,old.render_ns),"averageRenderMs":average(delta(p.render_ns,old.render_ns),delta(p.renders,old.renders)),
        "renderStages":render_stages,
        "nativeCommandCounts":{
            "renderPasses":delta(w.native_submit_counts[0],old_w.native_submit_counts[0]),
            "renderCommands":delta(w.native_submit_counts[1],old_w.native_submit_counts[1]),
            "closedCommandBuffers":delta(w.native_submit_counts[2],old_w.native_submit_counts[2]),
            "resetCommandBuffers":delta(w.native_submit_counts[3],old_w.native_submit_counts[3]),
            "retiredSubmissions":delta(w.native_submit_counts[4],old_w.native_submit_counts[4]),
            "retiredEncoders":delta(w.native_submit_counts[5],old_w.native_submit_counts[5]),
            "statelessRefsDropped":delta(w.native_submit_counts[6],old_w.native_submit_counts[6]),
            "statelessInsertions":delta(w.native_submit_counts[7],old_w.native_submit_counts[7]),
            "submissionStatelessRefsReleased":delta(w.native_submit_counts[8],old_w.native_submit_counts[8]),
            "buffersDropped":delta(w.native_submit_counts[9],old_w.native_submit_counts[9]),
            "texturesDropped":delta(w.native_submit_counts[10],old_w.native_submit_counts[10]),
            "viewsDropped":delta(w.native_submit_counts[11],old_w.native_submit_counts[11]),
            "bindGroupsDropped":delta(w.native_submit_counts[12],old_w.native_submit_counts[12]),
            "scopeMetadataSlots":delta(w.native_submit_counts[13],old_w.native_submit_counts[13]),
            "scopeMetadataReferencesReleased":delta(w.native_submit_counts[14],old_w.native_submit_counts[14]),
            "inlineRenderPasses":delta(w.native_submit_counts[15],old_w.native_submit_counts[15]),
            "splitRenderPasses":delta(w.native_submit_counts[16],old_w.native_submit_counts[16]),
            "inlineListReuses":delta(w.native_submit_counts[17],old_w.native_submit_counts[17]),
            "inlineScopeFallbacks":delta(w.native_submit_counts[18],old_w.native_submit_counts[18]),
            "dx12CommandListResets":delta(w.native_command_recording[4],old_w.native_command_recording[4]),
            "dx12CommandListCreates":delta(w.native_command_recording[5],old_w.native_command_recording[5])},
        "renderCounts":{
            "cachedSurfaces":delta(w.cached_surfaces,old_w.cached_surfaces),
            "filterPasses":delta(w.filter_passes,old_w.filter_passes),
            "surfacePasses":delta(w.surface_passes,old_w.surface_passes),
            "commands":delta(w.draw_commands,old_w.draw_commands),
            "queueSubmissions":delta(w.queue_submissions,old_w.queue_submissions),
            "directSubmissions":delta(w.direct_submissions,old_w.direct_submissions),
            "smallUploadSubmissionsSkipped":delta(w.small_upload_submissions_skipped,old_w.small_upload_submissions_skipped),
            "cacheEntriesExamined":delta(w.cache_entries_examined,old_w.cache_entries_examined),
            "directBitmapBlends":delta(w.direct_bitmap_blends,old_w.direct_bitmap_blends),
            "directBitmapGroups":delta(w.direct_bitmap_groups,old_w.direct_bitmap_groups),
            "poolPressureCandidates":delta(w.pool_pressure_candidates,old_w.pool_pressure_candidates),
            "fullTextureFilterQuads":delta(w.full_texture_filter_quads,old_w.full_texture_filter_quads),
            "croppedFilterVertexUploads":delta(w.cropped_filter_vertex_uploads,old_w.cropped_filter_vertex_uploads)},
        "catchupLimitedTicks":delta(p.catchup_limited_ticks,old.catchup_limited_ticks),
        "discardedFrameTimeMs":ms(p.discarded_frame_time_ns,old.discarded_frame_time_ns),
        "presentationCpuMs":ms(present.cpu_ns,old_present.cpu_ns),
        "gcCpuMs":ms(after.gc_ns,before.gc_ns),"gcSlices":delta(after.gc_slices,before.gc_slices),
        "resourceCreationCpuMs":ms(creation_ns,old_creation_ns),"timedResourceCreations":delta(created,old_created),
        "geometryResourceCreationCpuMs":ms(geometry_ns,old_geometry_ns),
        "timedGeometryResourceCreations":delta(geometry_created,old_geometry_created),
        "temporaryPoolHits":hits,"temporaryPoolMisses":misses,
        "temporaryPoolHitRatio":if hits+misses==0 {Value::Null} else {json!(hits as f64/(hits+misses) as f64)},
        "temporaryPoolPressureEvictions":delta(after.resources.temporary_pool_pressure_evictions,before.resources.temporary_pool_pressure_evictions),
        "temporaryPoolReturnRejections":delta(after.resources.temporary_pool_return_rejections,before.resources.temporary_pool_return_rejections),
        "temporaryPoolProbationRejections":delta(after.resources.temporary_pool_probation_rejections,before.resources.temporary_pool_probation_rejections),
        "temporaryPoolRecurrentMisses":delta(after.resources.temporary_pool_recurrent_misses,before.resources.temporary_pool_recurrent_misses)});
    report["temporaryPoolMissesByKindAndArea"] = Value::Object(pool_misses);
    report
}

pub struct MemoryLog {
    file: File,
    started: Instant,
    last_sample: Option<Instant>,
    previous_performance: Option<(Instant, PerformanceSnapshot)>,
}

impl MemoryLog {
    pub fn new(storage: &Storage) -> Result<Self, String> {
        storage.call("storage.makeFolder", &[json!("diagnostics")])?;
        let timestamp=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_nanos();
        let path=storage.resolve(&format!("diagnostics/resources-{timestamp}-{}.jsonl",std::process::id()))?;
        let file=OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|e.to_string())?;
        Ok(Self {file,started:Instant::now(),last_sample:None,previous_performance:None})
    }

    pub fn due(&self) -> bool {
        self.last_sample.is_none_or(|time| time.elapsed().as_secs() >= 5)
    }

    pub fn sample(&mut self, player: &Player, window: &Window, process: Value, presentation: PresentationStats, final_sample: bool) -> std::io::Result<()> {
        let now=Instant::now();
        self.last_sample=Some(now);
        let gc=player.gc_memory_stats();
        let performance=player.performance_stats();
        let snapshot=PerformanceSnapshot {player:performance,presentation,gc_ns:gc.total_ns,gc_slices:gc.slices,
            resources:player.renderer().resource_stats().unwrap_or_default()};
        let interval=self.previous_performance.map_or(json!({"available":false,"reason":"first-sample"}),|(time,previous)|
            performance_interval(previous,snapshot,now.duration_since(time).as_secs_f64()));
        self.previous_performance=Some((now,snapshot));
        let viewport=player.renderer().viewport_dimensions();
        let size=window.inner_size();
        let menu_height=if window.fullscreen().is_some(){0.0}else{f64::from(crate::gui::MENU_HEIGHT)*window.scale_factor()};
        let data=json!({"schema":15,"unixMs":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "elapsedSeconds":self.started.elapsed().as_secs_f64(),"processId":std::process::id(),"final":final_sample,
            "stage3":stage3_counters(),"fixedProfile":fixed_profile(),"runtimeFrameRateSetting":player.frame_rate(),
            "nativeFrameRateForced":player.is_frame_rate_forced(),
            "internalViewport":{"width":viewport.width,"height":viewport.height,"scaleFactor":viewport.scale_factor},
            "window":{"physicalWidth":size.width,"physicalHeight":size.height,"dpiScale":window.scale_factor(),"fullscreen":window.fullscreen().is_some()},
            "presentationRect":crate::gui::GuiController::aoqi_presentation_rect(size,menu_height),
            "process":process,"gpu":renderer_memory(player.renderer()),
            "performanceInterval":interval,
            "performanceLifetime":{"ticks":performance.ticks,"frames":performance.frames,"renders":performance.renders,
                "presentations":presentation.frames,"maxTickMs":performance.max_tick_ns as f64/1e6,
                "maxFrameMs":performance.max_frame_ns as f64/1e6,"maxRenderMs":performance.max_render_ns as f64/1e6,
                "maxPresentationCpuMs":presentation.max_cpu_ns as f64/1e6},
            "gc":{"phase":gc.phase,"allocationDebt":gc.allocation_debt,"allocations":gc.gc_allocations,
                "completedCycles":gc.completed_cycles,"periodicCycles":gc.periodic_cycles,
                "slices":gc.slices,"steps":gc.steps,"totalCpuMs":gc.total_ns as f64/1e6,
                "maxSliceMs":gc.max_slice_ns as f64/1e6,"overBudgetSlices":gc.over_budget_slices,"frameBudgetMs":gc.frame_budget_ns as f64/1e6,
                "libraryCharacters":gc.library_characters,"libraryUploadedBitmaps":gc.library_uploaded_bitmaps,
                "libraryGraphicMeshEntries":gc.library_graphic_mesh_entries,
                "libraryGpuCacheSweepChecks":gc.library_gpu_cache_sweep.checks,
                "libraryGpuMeshEntriesReleased":gc.library_gpu_cache_sweep.mesh_entries_released,
                "libraryGpuBitmapsReleased":gc.library_gpu_cache_sweep.bitmaps_released,
                "libraryGpuCacheSweepMs":gc.library_gpu_cache_sweep.total_ns as f64 / 1_000_000.0,"knownMovies":gc.known_movies,"knownMovieBytes":gc.known_movie_bytes}});
        serde_json::to_writer(&mut self.file, &data)?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn aola_frame_phase_intervals_are_deltas_not_cumulative() {
        let mut before = PerformanceSnapshot::default();
        let mut after = PerformanceSnapshot::default();
        before.player.frame_update.exclusive_ns[0] = 5_000_000;
        after.player.frame_update.exclusive_ns[0] = 8_000_000;
        before.player.frame_update.calls[0] = 2;
        after.player.frame_update.calls[0] = 5;
        before.player.frame_update.orphan_visits = 50;
        after.player.frame_update.orphan_visits = 95;
        before.player.frame_update.target_only_gotos = 3;
        after.player.frame_update.target_only_gotos = 10;
        before.player.frame_update.ordinary_orphans[0].dispatched_roots = 20;
        after.player.frame_update.ordinary_orphans[0].dispatched_roots = 22;
        before.player.frame_update.ordinary_orphans[1].scriptless_roots = 4;
        after.player.frame_update.ordinary_orphans[1].scriptless_roots = 11;
        before.player.frame_update.ordinary_orphans[2].sleeping_descendants = 10;
        after.player.frame_update.ordinary_orphans[2].sleeping_descendants = 23;
        before.player.frame_update.ordinary_orphans[2].scriptless_descendants = 5;
        after.player.frame_update.ordinary_orphans[2].scriptless_descendants = 11;
        before.player.frame_update.ordinary_orphan_script_calls = 10;
        after.player.frame_update.ordinary_orphan_script_calls = 12;
        before.player.frame_update.script_calls = 20;
        after.player.frame_update.script_calls = 23;
        let report = performance_interval(before, after, 5.0);
        let update = &report["frameUpdate"];
        assert_eq!(update["stages"][0]["name"], "other");
        assert_eq!(update["stages"][0]["exclusiveCpuMs"], 3.0);
        assert_eq!(update["stages"][0]["calls"], 3);
        assert_eq!(update["orphanVisits"], 45);
        assert_eq!(update["targetOnlyGotos"], 7);
        assert_eq!(update["scriptCalls"], 3);
        let ordinary = &update["ordinaryOrphans"];
        assert_eq!(ordinary["enter"]["dispatchedRoots"], 2);
        assert_eq!(ordinary["construct"]["scriptlessRoots"], 7);
        assert_eq!(ordinary["scripts"]["scriptlessRoots"], 0);
        assert_eq!(ordinary["scripts"]["sleepingDescendants"], 13);
        assert_eq!(ordinary["scripts"]["scriptlessDescendants"], 6);
        assert_eq!(ordinary["directFrameScriptCalls"], 2);
    }

    use super::*;
    #[test]
    fn geometry_creation_is_separate_from_legacy_timing_scope() {
        let before=PerformanceSnapshot::default();
        let mut after=before;
        after.resources.temporary_texture.creation_cpu_ns=2_000_000;
        after.resources.temporary_texture.timed_creation_count=3;
        after.resources.mesh_vertex_buffer.creation_cpu_ns=5_000_000;
        after.resources.mesh_vertex_buffer.timed_creation_count=1;
        after.resources.gradient_texture.creation_cpu_ns=7_000_000;
        after.resources.gradient_texture.timed_creation_count=2;
        after.resources.temporary_pool_probation_rejections=4;
        after.resources.temporary_pool_recurrent_misses=8;
        let report=performance_interval(before,after,5.0);
        assert_eq!(report["resourceCreationCpuMs"],2.0);
        assert_eq!(report["geometryResourceCreationCpuMs"],12.0);
        assert_eq!(report["timedResourceCreations"],3);
        assert_eq!(report["timedGeometryResourceCreations"],3);
        assert_eq!(report["temporaryPoolProbationRejections"],4);
        assert_eq!(report["temporaryPoolRecurrentMisses"],8);
    }
    #[test]
    fn performance_rates_use_observed_counts_and_elapsed_wall_time() {
        let mut before=PerformanceSnapshot::default();
        before.player.frames=20;
        before.presentation.frames=10;
        before.gc_ns=1_000_000;
        let mut after=before;
        after.player.frames+=100;
        after.presentation.frames+=150;
        after.gc_ns+=8_000_000;
        after.resources.temporary_pool_hits=75;
        after.resources.temporary_pool_misses=25;
        let report=performance_interval(before,after,5.0);
        assert_eq!(report["logicalFps"],20.0);
        assert_eq!(report["presentedFps"],30.0);
        assert_eq!(report["gcCpuMs"],8.0);
        assert_eq!(report["temporaryPoolHitRatio"],0.75);
        assert!(report["averageTickMs"].is_null());
    }

    #[test]
    fn reset_or_zero_length_sampling_never_claims_a_valid_fps() {
        let before=PerformanceSnapshot {player:PerformanceStats {frames:30,..Default::default()},..Default::default()};
        assert_eq!(performance_interval(before,PerformanceSnapshot::default(),5.0)["available"],false);
        assert_eq!(performance_interval(before,before,0.0)["available"],false);
        let idle=performance_interval(before,before,5.0);
        assert_eq!(idle["logicalFps"],0.0);
        assert!(idle["temporaryPoolHitRatio"].is_null());
    }
    #[test]
    fn render_stage_deltas_and_discarded_time_do_not_include_earlier_samples() {
        let mut before=PerformanceSnapshot::default();
        before.player.render_build_ns=90_000_000;
        before.player.discarded_frame_time_ns=200_000_000;
        before.resources.work.main_draw_ns=150_000_000;
        before.resources.work.direct_bitmap_blends=30;
        before.resources.work.direct_bitmap_groups=10;
        before.resources.work.command_finish_ns=11_000_000;
        before.resources.work.queue_execute_ns=6_000_000;
        before.resources.work.filter_calls[1]=20;
        before.resources.work.filter_ns[1]=70_000_000;
        before.resources.work.temporary_created[0]=100;
        before.resources.work.filter_uniform_cache_hits=100;
        before.resources.work.filter_uniform_cache_misses=50;
        before.resources.work.filter_uniform_cache_evictions=20;
        before.resources.work.filter_uniform_uploads=25;
        before.resources.work.filter_uniform_prepare_ns=1_000_000;
        before.resources.work.filter_bind_group_ns=2_000_000;
        before.resources.work.filter_bind_group_reuses=100;
        before.resources.work.filter_pass_encode_ns=3_000_000;
        before.resources.work.blur_passes=500;
        before.resources.work.fused_glow_passes=20;
        before.resources.work.glow_static_vertex_draws=120;
        before.resources.work.glow_vertex_uploads=10;
        before.resources.work.temporary_created_texels[0]=1000;
        before.resources.work.temporary_created_bytes[0]=16000;
        before.resources.work.temporary_size_requests=[10,20,30,40,50];
        before.resources.work.temporary_pool_missing_bucket[1][3]=90;
        before.resources.work.temporary_pool_empty_bucket[1][3]=40;
        before.resources.work.blend_targets=100;
        before.resources.work.bounded_blend_targets=20;
        before.resources.work.blend_full_pixels=84000000;
        before.resources.work.blend_target_pixels=20000000;
        before.resources.work.blend_padded_targets=15;
        before.resources.work.blend_padding_pixels=5000;
        before.resources.work.blend_parent_copy_pixels=10000000;
        before.resources.work.temporary_creation_ns[0]=50_000_000;
        let mut after=before;
        after.player.render_build_ns+=4_000_000;
        after.player.discarded_frame_time_ns+=50_000_000;
        after.resources.work.main_draw_ns+=9_000_000;
        after.resources.work.direct_bitmap_blends+=7;
        after.resources.work.direct_bitmap_groups+=2;
        after.resources.work.command_finish_ns+=3_000_000;
        before.resources.work.native_submit_ns=[1_000_000;29];
        after.resources.work.native_submit_ns=std::array::from_fn(|i| (i as u64+2)*1_000_000);
        after.resources.work.native_submit_counts=std::array::from_fn(|i| i as u64+1);
        after.resources.work.native_command_recording=[1_000_000,2_000_000,3_000_000,4_000_000,17,19];
        after.resources.work.queue_execute_ns+=2_000_000;
        after.resources.work.staging_finish_ns+=500_000;
        after.resources.work.staging_recall_ns+=100_000;
        after.resources.work.filter_calls[1]+=4;
        after.resources.work.filter_ns[1]+=5_000_000;
        after.resources.work.temporary_created[0]+=3;
        after.resources.work.filter_uniform_cache_hits+=10;
        after.resources.work.filter_uniform_cache_misses+=2;
        after.resources.work.filter_uniform_cache_evictions+=1;
        after.resources.work.filter_uniform_uploads+=3;
        after.resources.work.filter_uniform_prepare_ns+=100_000;
        after.resources.work.filter_bind_group_ns+=200_000;
        after.resources.work.filter_bind_group_reuses+=7;
        after.resources.work.filter_pass_encode_ns+=300_000;
        after.resources.work.blur_passes+=15;
        after.resources.work.fused_glow_passes+=8;
        after.resources.work.glow_static_vertex_draws+=4;
        after.resources.work.glow_vertex_uploads+=1;
        after.resources.work.temporary_created_texels[0]+=500;
        after.resources.work.temporary_created_bytes[0]+=8000;
        after.resources.work.temporary_size_requests=[11,22,33,44,55];
        after.resources.work.temporary_pool_missing_bucket[1][3]+=2;
        after.resources.work.temporary_pool_empty_bucket[1][3]+=7;
        after.resources.work.temporary_pool_empty_bucket[2][0]+=1;
        after.resources.work.blend_targets+=4;
        after.resources.work.bounded_blend_targets+=3;
        after.resources.work.blend_full_pixels+=3360000;
        after.resources.work.blend_target_pixels+=850000;
        after.resources.work.blend_padded_targets+=2;
        after.resources.work.blend_padding_pixels+=700;
        after.resources.work.blend_parent_copy_pixels+=10000;
        after.resources.work.temporary_creation_ns[0]+=2_000_000;
        let report=performance_interval(before,after,5.0);
        assert_eq!(report["renderStages"]["sceneBuildMs"],4.0);
        assert_eq!(report["filterSetup"],json!({"uniformCacheHits":10,"uniformCacheMisses":2,"uniformCacheEvictions":1,
            "uniformUploads":3,"uniformPrepareMs":0.1,"bindGroupMs":0.2,"bindGroupReuses":7,"passEncodeMs":0.3,"blurPasses":15,
            "fusedGlowPasses":8,"glowStaticVertexDraws":4,"glowVertexUploads":1}));
        assert_eq!(report["renderStages"]["mainDrawMs"],9.0);
        assert_eq!(report["renderCounts"]["directBitmapBlends"],7);
        assert_eq!(report["renderCounts"]["directBitmapGroups"],2);
        assert_eq!(report["discardedFrameTimeMs"],50.0);
        assert_eq!(report["renderStages"]["commandFinishMs"],3.0);
        for (i,key) in ["nativeCommandEncodeMs","nativeBufferInitMs","nativeTextureInitMs","nativeBarrierMs","nativeDriverSubmitMs","nativeMaintainMs","nativePendingWritesMs"].into_iter().enumerate() {
            assert_eq!(report["renderStages"][key],(i+1) as f64);
        }
        assert_eq!(report["renderStages"]["nativePassOpenMs"],8.0);
        assert_eq!(report["renderStages"]["nativePassStartMs"],9.0);
        assert_eq!(report["renderStages"]["nativePassCommandsMs"],10.0);
        assert_eq!(report["renderStages"]["nativePassFinishMs"],11.0);
        assert_eq!(report["renderStages"]["nativePassBarriersMs"],12.0);
        assert_eq!(report["renderStages"]["nativeEncoderCloseMs"],13.0);
        assert_eq!(report["renderStages"]["nativeEncoderResetMs"],14.0);
        assert_eq!(report["renderStages"]["nativeFenceQueryMs"],15.0);
        assert_eq!(report["renderStages"]["nativeMaintainLockMs"],16.0);
        assert_eq!(report["renderStages"]["nativeTriageMs"],17.0);
        assert_eq!(report["renderStages"]["nativeTrackerDropMs"],18.0);
        assert_eq!(report["renderStages"]["nativeTempDropMs"],19.0);
        assert_eq!(report["renderStages"]["nativePendingDropMs"],20.0);
        assert_eq!(report["renderStages"]["nativeMappingMs"],21.0);
        assert_eq!(report["renderStages"]["nativeScopeDropMs"],22.0);
        assert_eq!(report["renderStages"]["nativeBufferDropMs"],23.0);
        assert_eq!(report["renderStages"]["nativeTextureDropMs"],24.0);
        assert_eq!(report["renderStages"]["nativeViewDropMs"],25.0);
        assert_eq!(report["renderStages"]["nativeBindGroupDropMs"],26.0);
        assert_eq!(report["renderStages"]["nativeEncoderRecycleMs"],27.0);
        assert_eq!(report["renderStages"]["nativeRenderPassTotalMs"],28.0);
        assert_eq!(report["nativeCommandCounts"]["renderPasses"],1);
        assert_eq!(report["nativeCommandCounts"]["renderCommands"],2);
        assert_eq!(report["nativeCommandCounts"]["closedCommandBuffers"],3);
        assert_eq!(report["nativeCommandCounts"]["resetCommandBuffers"],4);
        assert_eq!(report["nativeCommandCounts"]["retiredSubmissions"],5);
        assert_eq!(report["nativeCommandCounts"]["retiredEncoders"],6);
        assert_eq!(report["nativeCommandCounts"]["statelessRefsDropped"],7);
        assert_eq!(report["nativeCommandCounts"]["statelessInsertions"],8);
        assert_eq!(report["nativeCommandCounts"]["submissionStatelessRefsReleased"],9);
        assert_eq!(report["nativeCommandCounts"]["buffersDropped"],10);
        assert_eq!(report["nativeCommandCounts"]["texturesDropped"],11);
        assert_eq!(report["nativeCommandCounts"]["viewsDropped"],12);
        assert_eq!(report["nativeCommandCounts"]["bindGroupsDropped"],13);
        for (i,key) in ["dx12CommandListResetMs","dx12CommandListCreateMs","dx12CommandListNameMs","dx12CommandStateClearMs"].into_iter().enumerate() {
            assert_eq!(report["renderStages"][key],(i+1) as f64);
        }
        assert_eq!(report["nativeCommandCounts"]["scopeMetadataSlots"],14);
        assert_eq!(report["nativeCommandCounts"]["scopeMetadataReferencesReleased"],15);
        assert_eq!(report["renderStages"]["nativePassPreflightMs"],29.0);
        for (i,key) in ["inlineRenderPasses","splitRenderPasses","inlineListReuses","inlineScopeFallbacks"].into_iter().enumerate() {
            assert_eq!(report["nativeCommandCounts"][key],i as u64+16);
        }
        assert_eq!(report["nativeCommandCounts"]["dx12CommandListResets"],17);
        assert_eq!(report["nativeCommandCounts"]["dx12CommandListCreates"],19);
        assert_eq!(report["renderStages"]["queueExecuteMs"],2.0);
        assert_eq!(report["renderStages"]["stagingFinishMs"],0.5);
        assert_eq!(report["renderStages"]["stagingRecallMs"],0.1);
        assert_eq!(report["filterWork"][1],json!({"name":"blur","calls":4,"cpuMs":5.0}));
        assert_eq!(report["temporaryAllocations"][0],json!({"name":"msaaColor","created":3,"creationCpuMs":2.0,"createdTexels":500,"createdEstimatedBytes":8000}));
        assert_eq!(report["blendArea"],json!({"targets":4,"boundedTargets":3,"fullTargetPixels":3360000,"allocatedTargetPixels":850000,"paddedTargets":2,"paddingPixels":700,"parentCopyPixels":10000}));
        assert_eq!(report["temporarySizeRequests"],json!({"upTo64Squared":1,"upTo256Squared":2,"upTo512Squared":3,"belowStageArea":4,"stageAreaOrLarger":5}));
        let misses = &report["temporaryPoolMissesByKindAndArea"];
        assert_eq!(misses["singleSampleColor"]["belowStageArea"],json!({"missingBucket":2,"emptyBucket":7}));
        assert_eq!(misses["stencil"]["upTo64Squared"],json!({"missingBucket":0,"emptyBucket":1}));
        assert_eq!(misses["msaaColor"]["stageAreaOrLarger"],json!({"missingBucket":0,"emptyBucket":0}));
    }
    #[test]
    fn resource_report_distinguishes_owner_estimates_from_gpu_residency() {
        let backend=ruffle_render::backend::null::NullRenderer::new(ruffle_render::backend::ViewportDimensions {width:1200,height:700,scale_factor:1.0});
        let report=renderer_memory(&backend);
        assert_eq!(report["available"],false);
        assert!(report["totalGpuBytes"].is_null());
        let values=category(RenderResourceCategoryStats {live_count:2,live_estimated_bytes:4096,released_count:3,..Default::default()});
        assert_eq!(values["liveCount"],2);
        assert_eq!(values["liveEstimatedBytes"],4096);
        assert_eq!(values["releasedCount"],3);
    }
}

fn stage3_counters() -> Value {
    use ruffle_core::aoqi_work as work;
    use std::sync::atomic::Ordering::Relaxed;
    let mode = work::mode();
    json!({"enabled":{"sleep":mode.sleep,"loading":mode.loading,"dedup":mode.dedup},
        "backgroundJobs":work::BACKGROUND_JOBS.load(Relaxed),"backgroundCpuMs":work::BACKGROUND_NS.load(Relaxed) as f64/1e6,
        "swfDecodes":work::SWF_DECODES.load(Relaxed),"swfDedupHits":work::SWF_DEDUP_HITS.load(Relaxed),
        "swfSharedBytesCumulative":work::SWF_SHARED_BYTES.load(Relaxed),
        "sleepNodesScanned":work::SLEEP_SCANNED.load(Relaxed),"sleepSubtreePhaseSkips":work::SLEEP_SKIPPED.load(Relaxed),
        "sleepWakeRoots":work::SLEEP_WAKES.load(Relaxed)})
}
