use super::*;
use crate::display_object::{GotoInfo, StopOrPlay};
use crate::frame_profile::FrameWork;
use std::path::Path;

struct Trace(Arc<Mutex<Vec<String>>>);
impl LogBackend for Trace {
    fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
    fn avm_warning(&self, _: &str) {}
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
fn aola_goto_profile_preserves_nested_goto_and_orphan_flash_traces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/tests/swfs/avm2");
    let mut total_gotos = 0;
    for enabled in [false, true] {
        for (name, frames) in [
            ("goto_on_orphan", 2), ("goto_in_constructframe", 2),
            ("goto_nested_framescript", 1), ("movieclip_queued_noop_goto_swf10", 2),
            ("movieclip_queued_noop_goto_swf9", 2),
            ("orphan_removeobject", 9),
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
