# Display-owner dictionary and cooperative GC reconstruction

These are new source implementations, not recovered original private Rust.

## Display owners

Recovered FairyGUI `GObject.as` opts its static map into `aoqiUseDisplayOwners()`, stores `DisplayObject -> GObject`, and deletes it in dispose. The key is a StageObject in Ruffle. The new native method is actually registered by the playerglobal source and returns this dictionary.

An opted-in dictionary holds only weak StageObject keys. Each display StageObject stores the corresponding value strongly, together with a weak dictionary identity. A live display (including a display still rooted by the display list) retains its owner value. When neither key nor value has an external root, a value-to-display cycle can be collected despite the globally rooted dictionary. Deleting/replacing a map entry updates the display-owned value. Enabling the mode migrates already inserted display keys. Normal keys, string properties and the unmodified dictionary mode keep upstream behavior.

Owner-key indexing currently uses a simple vector with tombstone reuse; insertion/deletion can be linear in the dictionary size, and each StageObject gains an owner-value container. No memory or throughput improvement has been measured for this reconstruction. Enumeration skips weak keys which cannot be upgraded. Values are removed after a dictionary is dropped, during the next trace of the display owner, so a destroyed dictionary does not permanently pin its values. This can require one additional GC cycle. The custom Collect implementation removes stale entries before tracing all remaining strong and weak references; it never leaves untraced strong pointers or adopts pointers during tracing. Every normal insertion goes through a Gc write barrier.

**Ordinary `Dictionary(true)` remains upstream's unimplemented weak-key behavior.** This extension is not a generic ephemeron implementation and does not claim to fix all Dictionary retention. Non-display keys intentionally fall back to upstream strong storage. Only opted-in display associations receive the new lifetime behavior.

For example, when a display still has an external root and its owner value strongly references the Dictionary, that value also keeps the Dictionary alive. The `is_dropped()` check cannot break this reachable cycle. Only cycles with no external roots are covered by the collection claim above.

Tests cover a rooted display, unrooted key/value cycles, dictionary destruction, and the actual VM's migration/get/has/replace/delete/enumeration paths. The owner lifetime graph tests exercise the same DisplayOwnerValues type used by StageObject, with gc-arena real collection.

## GC

The collector uses the existing recovered-core numeric rules for AOQI_GC_SLICE_MS (default 4; positive finite values clamped to 0.1..12) and AOQI_GC_SLEEP_FACTOR (default 0.1; nonnegative finite values retained). Environment string parsing is a new standard Rust implementation. Sleep factor is applied to real gc-arena Metrics::set_pacing, keeping other upstream pacing parameters.

The collector now uses a minimal local gc-arena 0.7.0 fork (`core/aoqi-gc-arena`) with `Arena::collect_step`. Each step performs one existing phase transition, root/object trace, or sweep item. Existing barriers, pointer semantics and metrics accounting are unchanged. Once a cycle is active, the collector advances it within its configured time budget even if debt has fallen to zero. A sleeping collector with no object debt also starts a cycle every five seconds. This addresses large bitmap workloads that allocate very few GC objects. A single object's trace cannot be interrupted, so this remains a cooperative budget.

The performance revision moves collection out of `Player::update` to the end of `tick`, after frame/timer/stream work. Multiple updates and catch-up frames in one tick no longer obtain separate budgets. A target-frame interval gate bounds repeated host wakeups. Paused ticks service the same gated collector; standalone `run_frame` callers retain a collection entry point. CPU slice totals, peak duration, step count and cooperative budget overruns are recorded. Tests verify ordinary updates do not collect and repeated wakeups share one frame slot.

The former scheduler temporarily capped artificial debt at 32 units. It could stop indefinitely when debt became zero during an active cycle and no further GC objects were allocated. That debt manipulation has been removed. No byte count is falsely added as object-count debt: gc-arena 0.7 counts allocations, not pixel/GPU bytes, and charging megabytes as thousands of object units could cause repeated excessive collections. The periodic-cycle path covers this blind spot without dropping live resources.

Regressions cover zero budget, active cycles with zero debt and no new allocations, and a real 4 MiB BitmapData whose last root is removed after a prior collection. The small fork also tests step boundedness and preservation of live roots. These tests require the root workspace patch to the local gc-arena dependency.

`Player::gc_memory_stats()` reports phase, object-count debt, GC allocation count, completed and periodic cycles, and the count/raw-data bytes of known movies. Pixel buffer bytes and GPU allocation bytes are not included in GC counts. They must be read from their own resource diagnostics. Metrics contain no account, URL or game-session data.

## Retention findings and optional bitmap-cache fallback

`GcRootData.library -> MovieLibraries -> HashMap<SwfMovie, MovieLibrary>` and `Avm2ClassRegistry.class_map` are strong roots without an eviction path. MovieLibrary also refers back to its movie and retains characters, fonts and application domains. No unsafe blanket weak-key conversion or forced deletion has been added; increased known_movie counts in diagnostics can identify retention which ordinary GC cannot free.

BitmapCache now calls the renderer's budgeted `create_bitmap_cache_texture`, dropping an old-sized cache before asking for a replacement. On rejection it keeps no cache handle, and the existing `render_base` fallback draws the object normally. This can omit cache-dependent filter effects under resource pressure; it does not drop an AS-visible BitmapData or forcibly invalidate a live game resource. No per-frame rejection log is added in core; the renderer counts rejections. GPU upload/readback pool cleanup is implemented separately by the renderer changes.

The cache-lifetime revision makes this optional display texture independently evictable. The renderer keeps weak cache registrations, protects the current and previous frame, evicts cold entries under the unchanged 384 MiB budget, and expires entries after 1800 unused presentation frames. Eviction removes the cache owner's bitmap; already-issued commands retain their own texture handles. Core treats an evicted same-size cache as dirty. DX12 cache budget charges round up to 64 KiB; this is still not a driver residency measure. Diagnostics now expose HAL resource counters and DXGI per-process segment usage/budgets separately. Ordinary BitmapData and movie/class roots retain their previous lifetime semantics.

No game login, target EXE execution or network gameplay was performed.
