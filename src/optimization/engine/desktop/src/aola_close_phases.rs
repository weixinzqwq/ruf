//! Normal-shutdown timings. Nested durations are inclusive; self time excludes
//! only direct children on the same thread. An unfinished phase keeps its begin row.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};

static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

struct Frame {
    name: String,
    id: u64,
    start: Instant,
    children: Duration,
    gpu: [(u64, u64); 4],
}

fn gpu_drops(before: [(u64, u64); 4], after: [(u64, u64); 4]) -> Value {
    let mut drops = serde_json::Map::new();
    for ((kind, (start_count, start_ns)), (count, ns)) in
        ["buffer", "texture", "textureView", "bindGroup"].into_iter().zip(before).zip(after)
    {
        if count > start_count {
            drops.insert(kind.to_owned(), json!({"count":count-start_count,"totalNs":ns.saturating_sub(start_ns)}));
        }
    }
    Value::Object(drops)
}

struct Recorder {
    file: File,
    origin: Instant,
    main_thread: ThreadId,
    next_id: u64,
    stacks: HashMap<ThreadId, Vec<Frame>>,
    write_failed: bool,
}

fn wall_time() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

pub fn initialize(data_dir: Option<&Path>) -> io::Result<()> {
    if std::env::var_os("AOLA_CLOSE_PHASES").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Ok(());
    }
    let Some(data_dir) = data_dir else { return Ok(()); };
    let dir = data_dir.join("diagnostics");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("close-phases-{}-{}.jsonl", wall_time().as_nanos(), std::process::id()));
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let _ = RECORDER.set(Mutex::new(Recorder {
        file, origin: Instant::now(), main_thread: thread::current().id(), next_id: 0,
        stacks: HashMap::new(), write_failed: false,
    }));
    Ok(())
}

pub fn configured() -> bool { RECORDER.get().is_some() }

impl Recorder {
    fn write(&mut self, at: Instant, mut row: Value) {
        row["schema"] = json!(1);
        row["processId"] = json!(std::process::id());
        row["elapsedUs"] = json!(at.duration_since(self.origin).as_micros());
        row["unixMs"] = json!(wall_time().as_millis());
        // One OS write per boundary. No fsync or tracing callbacks on the measured path.
        if self.write_failed { return; }
        let mut line = row.to_string().into_bytes();
        line.push(b'\n');
        if self.file.write_all(&line).is_err() {
            self.write_failed = true;
            eprintln!("Could not write the Aola shutdown phase log");
        }
    }

    fn edge(&mut self, name: &str, edge: &str, nodes: Option<u64>) {
        let at = Instant::now();
        let thread = thread::current().id();
        let stack = self.stacks.entry(thread).or_default();
        let mut row = json!({"event":edge, "phase":name, "thread":format!("{thread:?}")});
        if let Some(nodes) = nodes { row["nodes"] = json!(nodes); }
        match edge {
            "begin" => {
                self.next_id += 1;
                row["id"] = json!(self.next_id);
                row["parent"] = json!(stack.last().map(|frame| frame.id));
                stack.push(Frame { name: name.to_owned(), id: self.next_id, start: at, children: Duration::ZERO,
                    gpu: wgpu::hal::aola_close_resource_totals() });
            }
            "end" if stack.last().is_some_and(|frame| frame.name == name) => {
                let frame = stack.pop().unwrap();
                let duration = at.duration_since(frame.start);
                row["id"] = json!(frame.id);
                row["durationUs"] = json!(duration.as_micros());
                row["selfUs"] = json!(duration.saturating_sub(frame.children).as_micros());
                let drops = gpu_drops(frame.gpu, wgpu::hal::aola_close_resource_totals());
                if drops.as_object().is_some_and(|items| !items.is_empty()) {
                    row["gpuResourceDrops"] = drops;
                }
                row["parent"] = json!(stack.last().map(|frame| frame.id));
                if let Some(parent) = stack.last_mut() { parent.children += duration; }
            }
            "end" if thread != self.main_thread && stack.is_empty() => {
                // A worker may already be inside a GPU release when shutdown starts.
                row["event"] = json!("worker-already-running");
            }
            _ => {
                row["event"] = json!("unmatched-edge");
                row["expected"] = json!(stack.last().map(|frame| &frame.name));
            }
        }
        self.write(at, row);
    }
}

pub fn start() {
    if let Some(recorder) = RECORDER.get() && let Ok(mut r) = recorder.lock() {
        let now = Instant::now();
        r.origin = now;
        r.main_thread = thread::current().id();
        r.write(now, json!({"event":"start"}));
        wgpu::hal::aola_set_close_resource_timing(true);
        r.edge("shutdown", "begin", None);
        ACTIVE.store(true, Ordering::Release);
    }
}

pub fn finish() {
    if ACTIVE.swap(false, Ordering::AcqRel)
        && let Some(recorder) = RECORDER.get() && let Ok(mut r) = recorder.lock() {
        r.edge("shutdown", "end", None);
        wgpu::hal::aola_set_close_resource_timing(false);
        let drops = gpu_drops([(0, 0); 4], wgpu::hal::aola_close_resource_process_totals());
        let open: usize = r.stacks.values().map(Vec::len).sum();
        let mut worker_phases = Vec::new();
        for (thread, stack) in &r.stacks {
            if *thread != r.main_thread {
                worker_phases.extend(stack.iter().map(|frame| frame.id));
            }
        }
        worker_phases.sort_unstable();
        r.write(Instant::now(), json!({"event":"finish","openPhases":open,
            "openWorkerPhaseIds":worker_phases,"gpuResourceDrops":drops}));
    }
}

pub fn active() -> bool { ACTIVE.load(Ordering::Acquire) }

pub fn record(name: &str, edge: &str, nodes: Option<u64>) {
    if active() && let Some(recorder) = RECORDER.get() && let Ok(mut r) = recorder.lock() && active() {
        r.edge(name, edge, nodes);
    }
}

pub struct Phase(Option<&'static str>);

pub fn phase(name: &'static str) -> Phase {
    if active() { record(name, "begin", None); Phase(Some(name)) } else { Phase(None) }
}

impl Drop for Phase {
    fn drop(&mut self) {
        if let Some(name) = self.0 { record(name, "end", None); }
    }
}

#[derive(Default)]
struct Fields { phase: String, edge: String, nodes: Option<u64> }

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "phase" => self.phase = value.to_owned(),
            "edge" => self.edge = value.to_owned(),
            _ => {}
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "nodes" { self.nodes = Some(value); }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

pub fn on_event(event: &tracing::Event<'_>) {
    if active() {
        let mut fields = Fields::default();
        event.record(&mut fields);
        record(&fields.phase, &fields.edge, fields.nodes);
    }
}
