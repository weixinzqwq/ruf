//! Optional read-only desktop observations. No AVM references or request bodies cross this boundary.
use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};

static SINK: OnceLock<fn(&str, &str, &str)> = OnceLock::new();
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn install(sink: fn(&str, &str, &str)) { let _ = SINK.set(sink); }
pub fn set_recording(active: bool) { RECORDING.store(active, Ordering::Relaxed); }
pub fn recording() -> bool { RECORDING.load(Ordering::Relaxed) }
pub fn installed() -> bool { SINK.get().is_some() }

pub fn observe(kind: &str, address: &str, class: &str) {
    if recording() && let Some(sink) = SINK.get() { sink(kind, address, class); }
}
