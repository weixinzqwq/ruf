//! Existing Flash trace fixtures exercising orphan phases and Loader.loadBytes.
use crate::{PlayerBuilder, backend::{log::LogBackend, navigator::{NullExecutor, NullNavigatorBackend}}};
use std::{path::Path, sync::{Arc, Mutex}, time::Duration};
struct Trace(Arc<Mutex<Vec<String>>>);
impl LogBackend for Trace {
    fn avm_trace(&self, text: &str) { self.0.lock().unwrap().push(text.into()); }
    fn avm_warning(&self, _: &str) {}
}
#[test]
fn stage3_existing_flash_timeline_and_loadbytes_traces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/tests/swfs/avm2");
    for (name, frames) in [("goto_on_orphan",2),("orphan_movie_complex",14),
        ("orphan_movie_reorder",15),("orphan_removeobject",9),
        ("loader_loadbytes_events",4),("loader_loadbytes_invalid_png",2),("loader_loadbytes_url",20)] {
        let directory=root.join(name);
        let movie_url = if name == "loader_loadbytes_url" {
            url::Url::from_file_path(directory.join("test.swf").canonicalize().unwrap()).unwrap().to_string()
        } else { "file:///test.swf".into() };
        let movie=crate::tag_utils::SwfMovieData::from_data(&std::fs::read(directory.join("test.swf")).unwrap(),
            movie_url,None,None).unwrap();
        let trace=Arc::new(Mutex::new(Vec::new()));
        let expected=std::fs::read_to_string(directory.join("output.txt")).unwrap().replace("\r\n","\n");
        let expected_lines=expected.trim_end().lines().count();
        let mut executor=NullExecutor::new();
        let player=PlayerBuilder::new().with_movie(movie).with_log(Trace(trace.clone()))
            .with_navigator(NullNavigatorBackend::with_base_path(&directory,&executor).unwrap()).build();
        player.lock().unwrap().disable_gc_for_flash_trace_fixture();
        // Loading is asynchronous: compare the same completed trace, not a fixed
        // number of frames spent waiting for the worker. Timeline-only fixtures
        // still run their exact recorded frame count.
        let loading = name.starts_with("loader_");
        for _ in 0..(frames + if loading {20} else {0}) {
            player.lock().unwrap().run_frame();
            // Small real-time slots allow background work to finish between simulated frames.
            for _ in 0..5 { executor.run(); std::thread::sleep(Duration::from_millis(2)); }
            if loading && trace.lock().unwrap().len() >= expected_lines { break; }
        }
        let actual=trace.lock().unwrap().join("\n").replace("\r\n","\n");
        assert_eq!(actual.trim_end(),expected.trim_end(),"Flash trace fixture {name}");
        println!("stage3_fixture {name} trace_match=true");
    }
}
