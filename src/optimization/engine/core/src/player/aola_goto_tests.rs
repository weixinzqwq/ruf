use super::*;
use crate::display_object::{GotoInfo, StopOrPlay};
use crate::display_object::TDisplayObjectContainer;
use crate::frame_profile::FrameWork;
use crate::goto_profile::Counter;
use std::path::Path;

struct Trace(Arc<Mutex<Vec<String>>>);
impl LogBackend for Trace {
    fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
    fn avm_warning(&self, _: &str) {}
}

#[test]
fn aola_goto_target_only_keeps_unrelated_cleanup_until_ordinary_frame() {
    use crate::frame_lifecycle::{FramePhase, run_all_phases_avm2, run_frame_script_cleanup};
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///target/Main.swf".into(), None, None).unwrap();
    let player = PlayerBuilder::new().with_movie(movie).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    for _ in 0..8 { player.run_frame(); }
    player.mutate_with_update_context(|context| {
        let root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        let branch = root.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let target = branch.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let child = target.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        context.battle_maintenance.enter(1);
        *context.frame_phase = FramePhase::FrameScripts;
        for clip in [branch, child] {
            let name = AvmString::new_utf8(context.gc(), "stop");
            let stop = crate::avm2::Value::from(clip.object2().unwrap())
                .get_public_property(name, &mut Avm2Activation::from_nothing(context)).unwrap()
                .as_object().unwrap();
            clip.play();
            clip.register_frame_script(clip.current_frame(), Some(stop), context);
        }
        target.goto_frame(context, GotoInfo { frame: target.current_frame(), stop_or_play: StopOrPlay::Stop });
        let scoped = crate::goto_profile::goto_target_only_mode() != "off";
        assert_eq!(*context.frame_phase, FramePhase::FrameScripts);
        assert!(!child.playing(), "a target descendant must run its pending frame script");
        assert_eq!(branch.playing(), scoped, "the target's parent is outside the target subtree");
        assert_eq!(context.frame_script_cleanup_queue.len(), usize::from(scoped));
        // Finish the current ordinary frame's script phase before the next
        // ENTER_FRAME can advance this playing parent and detach it.
        run_frame_script_cleanup(context);
        assert!(!branch.playing(), "the unrelated registration must survive the goto");
        assert!(context.frame_script_cleanup_queue.is_empty());
        run_all_phases_avm2(context);
        assert!(branch.parent().is_none(), "the following ordinary ENTER_FRAME still detaches the branch");
    });
}

#[test]
fn aola_stopped_stage_subtree_skips_scripts_and_play_wakes_it() {
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///subtree/Main.swf".into(), None, None).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let player = PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone())).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    // Tick 7 attaches a stopped, eight-level tree to the stage.
    for _ in 0..8 { player.run_frame(); }
    assert!(trace.lock().unwrap().last().unwrap().contains("tick 7 broadcasts 7 children 0 parent true"));
    trace.lock().unwrap().clear();
    let mut visits = Vec::new();
    for wake in [false, true] {
        let stats = player.mutate_with_update_context(|context| {
            let clip = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
            if wake {
                clip.iter_render_list().next().unwrap().as_movie_clip().unwrap().play();
            }
            context.tick_profile.goto.begin_response(true);
            for _ in 0..100 {
                clip.goto_frame(context, GotoInfo { frame: clip.current_frame(), stop_or_play: StopOrPlay::Stop });
            }
            context.tick_profile.goto.finish_response().unwrap()
        });
        assert_eq!(stats.calls, 100);
        assert!(trace.lock().unwrap().is_empty(), "Same-frame gotos must not replay initialization or ENTER_FRAME");
        visits.push(stats.phases[FrameWork::ScriptTraversal as usize].counts[
            crate::goto_profile::Counter::ScriptVisits as usize]);
    }
    if crate::goto_profile::subtree_script_sleep_enabled() {
        assert!(visits[0] <= 200, "Quiet descendants should not be visited: {visits:?}");
        assert!(visits[1] > visits[0], "Playing a child must invalidate the parent proof: {visits:?}");
    } else {
        assert!(visits[0] >= 900, "Baseline must exercise the attached subtree: {visits:?}");
    }
    println!("subtree_script_visits enabled={} quiet={} awake={}",
        crate::goto_profile::subtree_script_sleep_enabled(), visits[0], visits[1]);
}

#[test]
fn aola_goto_leaf_path_follows_a_playing_leaf_through_reparenting() {
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///leaf/Main.swf".into(), None, None).unwrap();
    let player = PlayerBuilder::new().with_movie(movie).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    for _ in 0..8 { player.run_frame(); }
    let (attached, detached, reattached) = player.mutate_with_update_context(|context| {
        let root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        let mut parent = root.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let leaf = loop {
            let child = parent.iter_render_list().next().unwrap().as_movie_clip().unwrap();
            if child.num_children() == 0 { break child; }
            parent = child;
        };
        leaf.play();
        assert!(leaf.playing() && leaf.initialized() && leaf.object2().is_some());
        let measure = |context: &mut crate::context::UpdateContext<'_>| {
            let stage_root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
            context.tick_profile.goto.begin_response(true);
            stage_root.goto_frame(context, GotoInfo { frame: stage_root.current_frame(), stop_or_play: StopOrPlay::Stop });
            let stats = context.tick_profile.goto.finish_response().unwrap();
            assert_eq!(stats.inner_calls, 1);
            let construct = FrameWork::OrphanConstruct as usize;
            let scripts = FrameWork::OrphanScripts as usize;
            for (index, phase) in stats.phases.iter().enumerate() {
                if index != construct { assert_eq!(phase.counts[Counter::ConstructLeafHits as usize], 0); }
                if index != scripts { assert_eq!(phase.counts[Counter::ScriptLeafHits as usize], 0); }
            }
            [
                stats.phases[construct].counts[Counter::ConstructLeafHits as usize]
                    + stats.phases[construct].counts[Counter::OrphanGotoIdleSkipped as usize],
                stats.phases[scripts].counts[Counter::ScriptLeafHits as usize]
                    + stats.phases[scripts].counts[Counter::OrphanGotoIdleSkipped as usize],
            ]
        };
        let attached = measure(context);
        parent.remove_child(context, leaf.into());
        assert!(leaf.parent().is_none());
        let detached = measure(context);
        parent.insert_at_index(context, leaf.into(), 0);
        assert!(DisplayObject::ptr_eq(leaf.parent().unwrap(), parent.into()));
        let reattached = measure(context);
        (attached, detached, reattached)
    });
    let enabled = crate::goto_profile::goto_leaf_fast_path_enabled() || crate::goto_profile::goto_orphan_idle_enabled();
    assert_eq!(reattached, attached);
    if enabled {
        assert_eq!(detached, [attached[0] + 1, attached[1] + 1]);
    } else {
        assert_eq!((attached, detached), ([0, 0], [0, 0]));
    }
    println!("goto_leaf_path enabled={enabled} attached={attached:?} detached={detached:?} reattached={reattached:?}");
}

#[test]
fn aola_goto_orphan_idle_tree_wakes_for_pending_script_and_inserted_scripted_child() {
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///idle/Main.swf".into(), None, None).unwrap();
    let player = PlayerBuilder::new().with_movie(movie).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    for _ in 0..8 { player.run_frame(); }
    player.mutate_with_update_context(|context| {
        let mut root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        let branch = root.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let mut parent = branch;
        let leaf = loop {
            let child = parent.iter_render_list().next().unwrap().as_movie_clip().unwrap();
            if child.num_children() == 0 { break child; }
            parent = child;
        };
        root.remove_child(context, branch.into());
        branch.play();
        leaf.play();
        let jump = GotoInfo { frame: root.current_frame(), stop_or_play: StopOrPlay::Stop };
        context.tick_profile.goto.begin_response(true);
        for _ in 0..100 { root.goto_frame(context, jump); }
        let stats = context.tick_profile.goto.finish_response().unwrap();
        let construct = &stats.phases[FrameWork::OrphanConstruct as usize];
        let scripts = &stats.phases[FrameWork::OrphanScripts as usize];
        let enabled = crate::goto_profile::goto_orphan_idle_enabled();
        assert_eq!(branch.base().goto_orphan_idle(), enabled);
        for phase in [construct, scripts] {
            assert_eq!(phase.counts[Counter::OrphanGotoIdleSkipped as usize] >= 100, enabled);
        }
        println!("goto_orphan_idle={enabled} construct_visits={} script_visits={} construct_idle={} script_idle={}",
            construct.counts[Counter::ConstructVisits as usize], scripts.counts[Counter::ScriptVisits as usize],
            construct.counts[Counter::OrphanGotoIdleSkipped as usize], scripts.counts[Counter::OrphanGotoIdleSkipped as usize]);
        leaf.set_has_pending_script(true);
        assert!(!branch.base().goto_orphan_idle());
        root.goto_frame(context, jump);
        assert!(!leaf.has_pending_script(), "construction must update the pending flag");
        assert_eq!(branch.base().goto_orphan_rejected(), enabled);
        leaf.set_has_pending_script(true);
        root.goto_frame(context, jump);
        assert!(!leaf.has_pending_script(), "a rejected root still runs the normal phases");

        parent.remove_child(context, leaf.into());
        root.goto_frame(context, jump);
        assert_eq!(branch.base().goto_orphan_rejected(), enabled);
        assert!(!branch.base().goto_orphan_idle());
        // The fixture detaches this branch on tick 8. Its first ordinary
        // orphan Enter on tick 9 allows the now-idle tree to be checked again.
        root.insert_at_index(context, branch.into(), 0);
        crate::frame_lifecycle::run_all_phases_avm2(context);
        assert!(branch.parent().is_none());
        crate::frame_lifecycle::run_all_phases_avm2(context);
        assert!(!branch.base().goto_orphan_rejected());
        root.goto_frame(context, jump);
        assert_eq!(branch.base().goto_orphan_idle(), enabled);
        let stop_name = AvmString::new_utf8(context.gc(), "stop");
        let stop = crate::avm2::Value::from(leaf.object2().unwrap())
            .get_public_property(stop_name, &mut Avm2Activation::from_nothing(context)).unwrap()
            .as_object().unwrap();
        leaf.register_frame_script(leaf.current_frame(), Some(stop), context);
        leaf.play();
        parent.insert_at_index(context, leaf.into(), 0);
        assert!(!branch.base().goto_orphan_idle());
        assert!(leaf.playing());
        root.goto_frame(context, jump);
        assert!(!leaf.playing(), "the newly attached descendant must receive its frame script");
        root.insert_at_index(context, branch.into(), 0);
        assert!(DisplayObject::ptr_eq(branch.parent().unwrap(), root.into()));
    });
}

#[test]
fn aola_ordinary_orphan_keeps_playback_and_wakes_scriptless_tree() {
    use crate::frame_lifecycle::FramePhase;
    use crate::orphan_manager::{OrphanManager, ordinary_orphan_mode};
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///ordinary/Main.swf".into(), None, None).unwrap();
    let player = PlayerBuilder::new().with_movie(movie).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    for _ in 0..8 { player.run_frame(); }
    player.mutate_with_update_context(|context| {
        let mut root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        let branch = root.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let child = branch.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        root.remove_child(context, branch.into());
        context.battle_maintenance.enter(1);
        branch.play();
        context.frame_profile.begin_frame();
        *context.frame_phase = FramePhase::Enter;
        OrphanManager::each_orphan_obj::<false>(context, |d, ctx| d.enter_frame(ctx));
        let stats = context.frame_profile.stats.ordinary_orphans[0];
        assert!(stats.dispatched_roots > 0, "a playing tree must receive ordinary Enter");
        // Keep the root active even if this single-frame fixture stopped itself.
        branch.play();
        *context.frame_phase = FramePhase::Construct;
        OrphanManager::each_orphan_obj::<false>(context, |d, ctx| d.construct_frame(ctx));
        let enabled = ordinary_orphan_mode() != "off";
        assert_eq!(context.frame_profile.stats.ordinary_orphans[1].scriptless_roots > 0, enabled);
        assert_eq!(branch.base().goto_orphan_idle(), enabled);

        // A listener could install a script between construction and dispatch.
        // Register the descendant's own stop method and make it observable.
        let stop_name = AvmString::new_utf8(context.gc(), "stop");
        let stop = crate::avm2::Value::from(child.object2().unwrap())
            .get_public_property(stop_name, &mut Avm2Activation::from_nothing(context)).unwrap()
            .as_object().unwrap();
        child.play();
        child.register_frame_script(child.current_frame(), Some(stop), context);
        assert!(!branch.base().goto_orphan_idle());
        *context.frame_phase = FramePhase::FrameScripts;
        OrphanManager::each_orphan_obj::<false>(context, |d, ctx| d.run_frame_scripts(ctx));
        assert!(!child.playing(), "a newly registered descendant script must run on an active orphan");
        root.insert_at_index(context, branch.into(), 0);
        OrphanManager::each_orphan_obj::<false>(context, |d, _| {
            assert!(!DisplayObject::ptr_eq(d, branch.into()), "a reattached root must leave the ordinary walk");
        });
        context.frame_profile.finish_frame(true);
    });
}

#[test]
fn aola_ordinary_orphan_mixed_child_branches_prune_and_wake() {
    use crate::frame_lifecycle::FramePhase;
    use crate::orphan_manager::{OrphanManager, ordinary_branch_prune_enabled, ordinary_orphan_mode, sleeping_subtree};
    let movie = SwfMovieData::from_data(
        include_bytes!("../../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
        "file:///ordinary-branches/Main.swf".into(), None, None).unwrap();
    let player = PlayerBuilder::new().with_movie(movie).build();
    let mut player = player.lock().unwrap();
    player.disable_gc_for_flash_trace_fixture();
    for _ in 0..8 { player.run_frame(); }
    player.mutate_with_update_context(|context| {
        let mut stage_root = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        let mut branch = stage_root.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let mut playing = branch.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        let stopped = playing.iter_render_list().next().unwrap().as_movie_clip().unwrap();
        stage_root.remove_child(context, branch.into());
        playing.remove_child(context, stopped.into());
        branch.insert_at_index(context, stopped.into(), 1);
        assert!(sleeping_subtree(stopped.into()));
        playing.play();
        // A script on the parent prevents either whole-root proof.
        let stop_name = AvmString::new_utf8(context.gc(), "stop");
        let [parent_stop, child_stop] = [branch, playing].map(|clip| {
            crate::avm2::Value::from(clip.object2().unwrap())
                .get_public_property(stop_name, &mut Avm2Activation::from_nothing(context)).unwrap()
                .as_object().unwrap()
        });
        branch.play();
        branch.register_frame_script(branch.current_frame(), Some(parent_stop), context);
        context.battle_maintenance.enter(1);
        context.frame_profile.begin_frame();
        *context.frame_phase = FramePhase::Construct;
        OrphanManager::each_orphan_obj::<false>(context, |d, c| d.construct_frame(c));
        *context.frame_phase = FramePhase::FrameScripts;
        OrphanManager::each_orphan_obj::<false>(context, |d, c| d.run_frame_scripts(c));
        let enabled = ordinary_orphan_mode() != "off" && ordinary_branch_prune_enabled();
        for phase in &context.frame_profile.stats.ordinary_orphans[1..] {
            assert_eq!(phase.sleeping_descendants > 0, enabled);
            assert_eq!(phase.scriptless_descendants > 0, enabled);
        }
        assert!(!branch.playing(), "the active parent's script still runs");
        assert!(playing.playing(), "the scriptless child keeps playing");

        // A newly registered script invalidates a cached child proof between phases.
        *context.frame_phase = FramePhase::Construct;
        playing.register_frame_script(playing.current_frame(), Some(child_stop), context);
        assert!(!playing.base().goto_orphan_idle());
        *context.frame_phase = FramePhase::FrameScripts;
        OrphanManager::each_orphan_obj::<false>(context, |d, c| d.run_frame_scripts(c));
        assert!(!playing.playing(), "the child's newly installed script must run");
        context.frame_profile.finish_frame(true);
    });
}

#[test]
fn aola_goto_profile_preserves_nested_goto_and_orphan_flash_traces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/tests/swfs/avm2");
    let mut total_gotos = 0;
    for enabled in [false, true] {
        for (name, frames) in [
            ("goto_on_orphan", 2), ("goto_in_constructframe", 2),
            ("goto_nested_framescript", 1), ("movieclip_queued_noop_goto_swf10", 2),
            ("movieclip_queued_noop_goto_swf9", 2),
            ("orphan_removeobject", 9), ("orphan_movie_complex", 14), ("orphan_movie_reorder", 15),
            ("goto_button_nested_framescript", 2), ("goto_nested_construct_sibling", 3),
            ("goto_framescript_queued_same_frame", 10), ("swf_10_queued_goto_scripts_construct", 2),
        ] {
            let directory = root.join(name);
            let movie = SwfMovieData::from_data(&std::fs::read(directory.join("test.swf")).unwrap(),
                "file:///goto_fixture/test.swf".into(), None, None).unwrap();
            let trace = Arc::new(Mutex::new(Vec::new()));
            let expected = std::fs::read_to_string(directory.join("output.txt")).unwrap().replace("\r\n", "\n");
            let player = PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone())).build();
            let mut player = player.lock().unwrap();
            // Keep the same short-lived orphan lifetime as the recorded Flash fixtures.
            player.disable_gc_for_flash_trace_fixture();
            player.mutate_with_update_context(|context| context.tick_profile.goto.begin_response(enabled));
            for _ in 0..frames { player.run_frame(); }
            let stats = player.mutate_with_update_context(|context| context.tick_profile.goto.finish_response());
            let actual = trace.lock().unwrap().join("\n").replace("\r\n", "\n");
            assert_eq!(actual.trim_end(), expected.trim_end(), "Flash trace: {name}, profiling={enabled}");
            if enabled {
                let stats = stats.expect("Enabled response must export goto statistics");
                assert_eq!(stats.phases.iter().map(|p| p.exclusive_ns).sum::<u64>(), stats.outer_total_ns,
                    "Exclusive accounting: {name}");
                total_gotos += stats.calls;
            } else {
                assert!(stats.is_none(), "Disabled response should not export: {name}");
            }
            println!("aola_goto_fixture {name} profiling={enabled} trace_match=true");
        }
    }
    assert!(total_gotos > 0, "The fixtures must exercise actual goto execution");
}

#[test]
fn aola_goto_profile_accounts_for_same_frame_goto_outside_run_frame() {
    // This existing SWF has no frame-event listeners or frame scripts after initialization.
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/tests/swfs/avm2/date_parse");
    let bytes = std::fs::read(directory.join("test.swf")).unwrap();
    assert!(bytes[3] > 9, "The fixture must execute the modern inner goto lifecycle");
    let movie = SwfMovieData::from_data(&bytes, "file:///date_parse/test.swf".into(), None, None).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let player = PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone())).build();
    let mut player = player.lock().unwrap();
    player.run_frame();
    let expected = std::fs::read_to_string(directory.join("output.txt")).unwrap().replace("\r\n", "\n");
    assert_eq!(trace.lock().unwrap().join("\n").trim_end(), expected.trim_end());
    trace.lock().unwrap().clear();
    let stats = player.mutate_with_update_context(|context| {
        assert!(!context.frame_profile.is_active());
        context.tick_profile.goto.begin_response(true);
        let clip = context.stage.root_clip().unwrap().as_movie_clip().unwrap();
        assert!(clip.swf_version() > 9);
        clip.goto_frame(context, GotoInfo { frame: clip.current_frame(), stop_or_play: StopOrPlay::Stop });
        assert!(!context.frame_profile.is_active());
        context.tick_profile.goto.finish_response().expect("External goto must be measured")
    });
    assert!(trace.lock().unwrap().is_empty(), "A same-frame goto must not rerun the fixture initialization");
    assert!(stats.calls > 0);
    assert!(stats.same_frame_calls > 0);
    assert!(stats.inner_calls > 0);
    for work in [
        FrameWork::OrphanConstruct, FrameWork::TimelineConstruct, FrameWork::FrameConstructedEvent,
        FrameWork::ScriptTraversal, FrameWork::OrphanScripts, FrameWork::ExitFrameEvent,
        FrameWork::ExitFrameLoading, FrameWork::OrphanCleanup,
    ] {
        assert!(stats.phases[work as usize].calls > 0, "Missing outside-frame phase: {work:?}");
    }
    assert_eq!(stats.phases.iter().map(|p| p.exclusive_ns).sum::<u64>(), stats.outer_total_ns);
}
