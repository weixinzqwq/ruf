//! Shared MyPro rules and desktop observations, without AVM references or request bodies.
use std::collections::HashMap;
use std::sync::{
    Arc, LazyLock, OnceLock, RwLock,
    atomic::{AtomicBool, Ordering},
};

static SINK: OnceLock<fn(&str, &str, &str)> = OnceLock::new();
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn install(sink: fn(&str, &str, &str)) { let _ = SINK.set(sink); }
pub fn set_recording(active: bool) { RECORDING.store(active, Ordering::Relaxed); }
pub fn recording() -> bool { RECORDING.load(Ordering::Relaxed) }
pub fn installed() -> bool { SINK.get().is_some() }

pub fn observe(kind: &str, address: &str, class: &str) {
    if recording() && let Some(sink) = SINK.get() { sink(kind, address, class); }
}

#[derive(Default)]
pub struct MorphRuleSnapshot {
    pub enabled: bool,
    pub revision: u64,
    pub enemy_enabled: bool,
    pub backpack_map: HashMap<u32, u32>,
    // An empty map means battle replacement is disabled at the last boundary.
    pub battle_map: HashMap<u32, u32>,
    pub poster_map: HashMap<u32, u32>,
}

static MORPH_RULES: LazyLock<RwLock<Arc<MorphRuleSnapshot>>> = LazyLock::new(Default::default);

pub fn publish_morph_rules(rules: MorphRuleSnapshot) {
    *MORPH_RULES.write().unwrap() = Arc::new(rules);
}

pub fn morph_rule_snapshot() -> Arc<MorphRuleSnapshot> {
    MORPH_RULES.read().unwrap().clone()
}
