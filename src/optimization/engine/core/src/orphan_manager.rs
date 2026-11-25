//! Special handling for AVM2 orphan objects

use crate::context::UpdateContext;
use crate::display_object::{DisplayObject, DisplayObjectWeak, TDisplayObject, TDisplayObjectContainer};
use crate::frame_lifecycle::FramePhase;
use crate::frame_profile::OrdinaryOrphanStats;
use gc_arena::{Collect, Mutation};
use std::rc::Rc;
use std::sync::OnceLock;

pub(crate) fn ordinary_orphan_mode() -> &'static str {
    static MODE: OnceLock<&str> = OnceLock::new();
    MODE.get_or_init(|| match std::env::var("AOLA_ORDINARY_ORPHANS").as_deref() {
        Ok("battle") => "battle",
        Ok("1") => "all",
        _ => "off",
    })
}

pub(crate) fn ordinary_branch_prune_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_ORDINARY_BRANCH_PRUNE").as_deref() == Ok("1"))
}

/// The list of 'orphan' objects - these objects have no parent,
/// so we need to manually run their frames in `run_all_phases_avm2` to match
/// Flash's behavior. Clips are added to this list with `add_orphan_movie`.
/// and are removed automatically by `cleanup_dead_orphans`.
///
/// We store `DisplayObjectWeak`, since we don't want to keep these objects
/// alive if they would otherwise be garbage-collected. The movie will
/// stop ticking whenever garbage collection runs if there are no more
/// strong references around (this matches Flash's behavior).
#[derive(Collect)]
#[collect(no_drop)]
pub struct OrphanManager<'gc> {
    orphans: Rc<Vec<DisplayObjectWeak<'gc>>>,
}

impl<'gc> OrphanManager<'gc> {
    pub fn slots(&self) -> usize { self.orphans.len() }

    pub(crate) fn weak_slots(&self) -> &[DisplayObjectWeak<'gc>] { &self.orphans }

    fn orphans_mut(&mut self) -> &mut Vec<DisplayObjectWeak<'gc>> {
        Rc::make_mut(&mut self.orphans)
    }

    /// Adds a `MovieClip` to the orphan list. In AVM2, movies advance their
    /// frames even when they are not on a display list. Unfortunately,
    /// multiple SWFS rely on this behavior, so we need to match Flash's
    /// behavior. This should not be called manually - `movie_clip` will
    /// call it when necessary.
    pub fn add_orphan_obj(&mut self, dobj: DisplayObject<'gc>) {
        // Note: comparing pointers is correct because GcWeak keeps its allocation alive,
        // so the pointers can't overlap by accident.
        if self
            .orphans
            .iter()
            .all(|d| !std::ptr::eq(d.as_ptr(), dobj.as_ptr()))
        {
            self.orphans_mut().push(dobj.downgrade());
        }
    }

    pub fn each_orphan_obj<const INNER_GOTO: bool>(
        context: &mut UpdateContext<'gc>,
        mut f: impl FnMut(DisplayObject<'gc>, &mut UpdateContext<'gc>),
    ) {
        // Clone the Rc before iterating over it. Any modifications must go through
        // `Rc::make_mut` in `orphan_objects_mut`, which will leave this `Rc` unmodified.
        // This ensures that any orphan additions/removals done by `f` will not affect
        // the iteration in this method.
        let orphan_objs: Rc<_> = context.orphan_manager.orphans.clone();
        context.tick_profile.goto.count(crate::goto_profile::Counter::OrphanVisits, orphan_objs.len() as u64);

        let profiling = context.frame_profile.is_active();
        let phase = *context.frame_phase;
        let ordinary = !INNER_GOTO && match ordinary_orphan_mode() {
            "battle" => context.battle_maintenance.is_active(),
            "all" => true,
            _ => false,
        };
        let ordinary_stats = if !INNER_GOTO && profiling {
            OrdinaryOrphanStats::phase_index(phase)
        } else {
            None
        };
        let prune_branches = ordinary && ordinary_branch_prune_enabled();
        let previous_scope = context.frame_profile.replace_ordinary_orphan_script_scope(
            !INNER_GOTO && profiling && phase == FramePhase::FrameScripts,
        );
        let mut sample = (!INNER_GOTO && phase == FramePhase::Enter
            && context.frame_profile.ordinary_sample_requested.replace(false)).then(|| {
            crate::orphan_census::OrdinaryRootSample::new(
                orphan_objs.len(), context.frame_profile.stats.started_frames, context.battle_maintenance.is_active(),
            )
        });
        let goto_idle = INNER_GOTO && crate::goto_profile::goto_orphan_idle_enabled();
        let reset_rejected = !INNER_GOTO && phase == FramePhase::Enter
            && (ordinary || crate::goto_profile::goto_orphan_idle_enabled());
        if profiling { context.frame_profile.stats.orphan_visits += orphan_objs.len() as u64; }
        for (index, orphan) in orphan_objs.iter().enumerate() {
            let object = valid_orphan(*orphan, context.gc());
            if let Some(sample) = &mut sample { sample.observe(index, object); }
            if let Some(dobj) = object {
                if reset_rejected { dobj.base().set_goto_orphan_rejected(false); }
                if goto_idle && goto_idle_root(dobj) {
                    context.tick_profile.goto.count(crate::goto_profile::Counter::OrphanGotoIdleSkipped, 1);
                    if profiling { context.frame_profile.stats.orphan_goto_idle_skipped += 1; }
                } else if crate::aoqi_work::mode().sleep && sleeping_subtree(dobj) {
                    context.tick_profile.goto.count(crate::goto_profile::Counter::OrphanSleepSkipped, 1);
                    if let Some(i) = ordinary_stats { context.frame_profile.stats.ordinary_orphans[i].sleeping_roots += 1; }
                    if profiling { context.frame_profile.stats.orphan_sleep_skipped += 1; }
                    crate::aoqi_work::SLEEP_SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else if ordinary && matches!(phase, FramePhase::Construct | FramePhase::FrameScripts)
                    && goto_idle_root(dobj)
                {
                    // Enter already advanced the timelines and placed children. A fully
                    // constructed, scriptless tree has no work in either of these phases.
                    if let Some(i) = ordinary_stats { context.frame_profile.stats.ordinary_orphans[i].scriptless_roots += 1; }
                } else {
                    context.tick_profile.goto.count(crate::goto_profile::Counter::OrphanExecuted, 1);
                    if profiling { context.frame_profile.stats.orphan_executed += 1; }
                    if let Some(i) = ordinary_stats { context.frame_profile.stats.ordinary_orphans[i].dispatched_roots += 1; }
                    if ordinary && phase == FramePhase::Enter && let DisplayObject::MovieClip(clip) = dobj {
                        clip.enter_subtree::<true>(context);
                    } else if prune_branches && phase == FramePhase::Construct {
                        crate::display_object::MovieClip::walk_ordinary_orphan::<false>(dobj, None, context);
                    } else if prune_branches && phase == FramePhase::FrameScripts {
                        crate::display_object::MovieClip::walk_ordinary_orphan::<true>(dobj, None, context);
                    } else {
                        f(dobj, context);
                    }
                }
            } else {
                if let Some(i) = ordinary_stats { context.frame_profile.stats.ordinary_orphans[i].invalid_roots += 1; }
                context.tick_profile.goto.count(crate::goto_profile::Counter::OrphanInvalid, 1);
                if profiling { context.frame_profile.stats.orphan_invalid += 1; }
            }
        }
        context.frame_profile.replace_ordinary_orphan_script_scope(previous_scope);
        if let Some(sample) = sample { context.frame_profile.ordinary_sample.replace(Some(sample.finish())); }
    }

    /// Called at the end of `run_all_phases_avm2` - removes any movies
    /// that have been garbage collected, or are no longer orphans
    /// (they've since acquired a parent).
    pub fn cleanup_dead_orphans(&mut self, mc: &Mutation<'gc>) {
        self.orphans_mut().retain(|d| {
            if let Some(dobj) = valid_orphan(*d, mc) {
                // All clips that become orphaned (have their parent removed, or start out with no parent)
                // get added to the orphan list. However, there's a distinction between clips
                // that are removed from a RemoveObject tag, and clips that are removed from ActionScript.
                //
                // Clips removed from a RemoveObject tag only stay on the orphan list until the end
                // of the frame - this lets them run a framescript (with 'this.parent == null')
                // before they're removed. After that, they're removed from the orphan list,
                // and will not be run in any way.
                //
                // Clips removed from ActionScript stay on the orphan list, and will be run
                // indefinitely (if there are no remaining strong references, they will eventually
                // be garbage collected).
                //
                // To detect this, we check 'placed_by_avm2_script'. This flag get set to 'true'
                // for objects constructed from ActionScript, and for objects moved around
                // in the timeline (add/remove child, swap depths) by ActionScript. A
                // RemoveObject tag will only affect objects instantiated by the timeline,
                // which have not been moved in the displaylist by ActionScript. Therefore,
                // any orphan we see that has 'placed_by_avm2_script()' should stay on the orphan
                // list, because it was not removed by a RemoveObject tag.
                dobj.placed_by_avm2_script()
            } else {
                false
            }
        });
    }
}

impl<'gc> Default for OrphanManager<'gc> {
    fn default() -> Self {
        Self {
            orphans: Rc::new(Vec::new()),
        }
    }
}

/// If the provided `DisplayObjectWeak` should have frames run, returns
/// Some(clip) with an upgraded `MovieClip`.
/// If this returns `None`, the entry should be removed from the orphan list.
fn valid_orphan<'gc>(
    dobj: DisplayObjectWeak<'gc>,
    mc: &Mutation<'gc>,
) -> Option<DisplayObject<'gc>> {
    dobj.upgrade(mc).filter(|dobj| dobj.parent().is_none())
}

/// Cache the proof for a whole stopped tree. Mutation wakes it through parent links.
/// Unsupported display types stay active; visibility and screen coordinates do not decide sleep.
pub(crate) fn sleeping_subtree(object: DisplayObject<'_>) -> bool {
    if object.base().timeline_quiet() { return true; }
    crate::aoqi_work::SLEEP_SCANNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let quiet = match object {
        DisplayObject::MovieClip(clip) => clip.timeline_can_sleep()
            && clip.iter_render_list().all(|child| is_child_of(child, object) && sleeping_subtree(child)),
        DisplayObject::Bitmap(_) | DisplayObject::Graphic(_) | DisplayObject::MorphShape(_) | DisplayObject::Text(_) => object.object2().is_some() && !object.base().should_skip_next_enter_frame(),
        _ => false,
    };
    if quiet { object.base().mark_timeline_quiet(); }
    quiet
}

pub(crate) fn is_child_of<'gc>(child: DisplayObject<'gc>, parent: DisplayObject<'gc>) -> bool {
    child.parent().is_some_and(|actual| DisplayObject::ptr_eq(actual, parent))
}

fn goto_idle_root(object: DisplayObject<'_>) -> bool {
    let base = object.base();
    if base.goto_orphan_rejected() || base.timeline_quiet() { return false; }
    if goto_idle_subtree::<true>(object) { return true; }
    base.set_goto_orphan_rejected(true);
    false
}

/// Construction and frame scripts can be idle even while a timeline plays.
/// Require real parent links so mutations invalidate every cached ancestor.
// Mixed ordinary roots are already being traversed. In that path only consult
// immediate children's cached proofs to avoid a recursive second walk.
pub(crate) fn goto_idle_subtree<const RECURSE: bool>(object: DisplayObject<'_>) -> bool {
    let base = object.base();
    if base.goto_orphan_idle() { return true; }
    let idle = match object {
        DisplayObject::MovieClip(clip) => clip.idle_during_inner_goto()
            && clip.iter_render_list().all(|child| is_child_of(child, object) && if RECURSE {
                goto_idle_subtree::<true>(child)
            } else {
                child.base().goto_orphan_idle()
            }),
        DisplayObject::Bitmap(_) | DisplayObject::Graphic(_) | DisplayObject::MorphShape(_)
        | DisplayObject::Text(_) => object.object2().is_some(),
        _ => false,
    };
    if idle { base.mark_goto_orphan_idle(); }
    idle
}

#[cfg(test)]
mod stage3_tests {
    use std::sync::{Arc, Mutex, atomic::Ordering::Relaxed};
    struct Trace(Arc<Mutex<Vec<String>>>);
    impl crate::backend::log::LogBackend for Trace {
        fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
        fn avm_warning(&self, _: &str) {}
    }
    #[test]
    fn stage3_stopped_tree_preserves_broadcasts_and_wakes_after_script_mutations() {
        let movie = crate::tag_utils::SwfMovieData::from_data(
            include_bytes!("../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
            "file:///stage3.swf".into(), None, None).unwrap();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let player = crate::PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone())).build();
        let before = crate::aoqi_work::SLEEP_SKIPPED.load(Relaxed);
        let start = std::time::Instant::now();
        for _ in 0..11 { player.lock().unwrap().run_frame(); }
        let lines = trace.lock().unwrap();
        println!("{}", lines.join("\n"));
        let expected = std::iter::once("ready".to_string()).chain((1..=10).map(|i|
            format!("tick {i} broadcasts {i} children {} parent {}", if i==5 {1} else {0}, i==7))).collect::<Vec<_>>();
        assert_eq!(*lines, expected);
        let skipped = crate::aoqi_work::SLEEP_SKIPPED.load(Relaxed)-before;
        println!("stage3_sleep skips={skipped} scanned={} wakes={} elapsed_ms={}",
            crate::aoqi_work::SLEEP_SCANNED.load(Relaxed), crate::aoqi_work::SLEEP_WAKES.load(Relaxed), start.elapsed().as_secs_f64()*1000.0);
        if crate::aoqi_work::mode().sleep {
            assert!(skipped >= 1000);
            assert!(crate::aoqi_work::SLEEP_WAKES.load(Relaxed)>0);
        } else { assert_eq!(skipped,0); }
        drop(lines);
        let start=std::time::Instant::now();
        for _ in 0..300 { player.lock().unwrap().run_frame(); }
        println!("stage3_sleep_warm frames=300 elapsed_ms={}",start.elapsed().as_secs_f64()*1000.0);
    }
}
