use super::*;
use crate::backend::navigator::{NullExecutor, NullNavigatorBackend};
use std::path::Path;

struct Trace(Arc<Mutex<Vec<String>>>);
impl LogBackend for Trace {
    fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
    fn avm_warning(&self, _: &str) {}
}

#[test]
fn aola_library_flash_traces_survive_collections() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/tests/swfs/avm2");
    for (name, frames) in [
        ("delayed_symbolclass", 4), ("doabc_and_symbolclass_script_init_goto", 3),
        ("symbol_class_binary_data", 1), ("loader_duplicate_coerce_new_domain", 2),
        ("loader_applicationDomain", 4), ("loader_loadbytes_events", 4),
    ] {
        let directory = root.join(name);
        let movie = SwfMovieData::from_data(&std::fs::read(directory.join("test.swf")).unwrap(),
            "file:///test.swf".into(), None, None).unwrap();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let expected = std::fs::read_to_string(directory.join("output.txt")).unwrap().replace("\r\n", "\n");
        let mut executor = NullExecutor::new();
        let player = PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone()))
            .with_navigator(NullNavigatorBackend::with_base_path(&directory, &executor).unwrap()).build();
        let loading = name.starts_with("loader_");
        for _ in 0..(frames + if loading {60} else {0}) {
            {
                let mut player = player.lock().unwrap();
                player.run_frame();
                // Deliberately collect between every frame, including loading:
                // classes, pending loaders and live timelines must keep their libraries.
                player.gc_arena.borrow_mut().finish_cycle();
                player.enter_arena_mut(|mc, root, _| root.library.after_collection(mc));
                player.render();
            }
            for _ in 0..5 { executor.run(); std::thread::sleep(Duration::from_millis(2)); }
            if loading && trace.lock().unwrap().len() >= expected.trim_end().lines().count() { break; }
        }
        let actual = trace.lock().unwrap().join("\n").replace("\r\n", "\n");
        assert_eq!(actual.trim_end(), expected.trim_end(), "Flash trace with GC: {name}");
        let stats = player.lock().unwrap().performance_stats();
        assert_eq!(stats.frame_update.completed_frames, stats.frames);
        assert_eq!(stats.frame_update.started_frames, stats.frame_update.completed_frames + stats.frame_update.preload_returns);
        if stats.frame_update.preload_returns == 0 {
            assert!(stats.frame_update.exclusive_ns.iter().sum::<u64>() <= stats.frame_ns);
        }
        println!("aola_library_fixture {name} trace_match=true gc_between_frames=true");
    }
}
