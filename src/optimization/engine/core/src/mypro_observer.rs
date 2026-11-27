//! Shared MyPro rules and desktop observations, without AVM references or request bodies.
use std::cmp::Ordering as VersionOrdering;
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, LazyLock, OnceLock, RwLock,
    atomic::{AtomicBool, Ordering},
};

static SINK: OnceLock<fn(&str, &str, &str)> = OnceLock::new();
static RECORDING: AtomicBool = AtomicBool::new(false);
static BATTLE_INPUT_NORMAL_FPS: AtomicBool = AtomicBool::new(true);

pub fn install(sink: fn(&str, &str, &str)) { let _ = SINK.set(sink); }
pub fn set_recording(active: bool) { RECORDING.store(active, Ordering::Relaxed); }
pub fn recording() -> bool { RECORDING.load(Ordering::Relaxed) }
pub fn installed() -> bool { SINK.get().is_some() }

pub fn set_battle_input_normal_fps(enabled: bool) {
    BATTLE_INPUT_NORMAL_FPS.store(enabled, Ordering::Relaxed);
}

pub fn battle_input_normal_fps() -> bool {
    BATTLE_INPUT_NORMAL_FPS.load(Ordering::Relaxed)
}

pub fn observe(kind: &str, address: &str, class: &str) {
    if recording() && let Some(sink) = SINK.get() { sink(kind, address, class); }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchVersionMismatch {
    pub resource: String,
    pub official_version: String,
    pub local_version: Option<String>,
}

static PATCH_MISMATCHES: LazyLock<RwLock<BTreeMap<String, PatchVersionMismatch>>> =
    LazyLock::new(Default::default);

fn decimal_version_cmp(left: &str, right: &str) -> VersionOrdering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn dated_patch_version(version: &str) -> Option<(u32, &str)> {
    for year_width in [4, 2] {
        if year_width == 4 && !version.starts_with("20") {
            continue;
        }
        let Some(date) = version.get(..year_width + 4) else { continue };
        let Some(year) = date.get(..year_width).and_then(|part| part.parse::<u32>().ok()) else { continue };
        let Some(month) = date.get(year_width..year_width + 2).and_then(|part| part.parse::<u32>().ok()) else { continue };
        let Some(day) = date.get(year_width + 2..).and_then(|part| part.parse::<u32>().ok()) else { continue };
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            let year = if year_width == 2 { 2000 + year } else { year };
            return Some((year * 10_000 + month * 100 + day, &version[date.len()..]));
        }
    }
    None
}

pub fn compare_patch_versions(left: &str, right: &str) -> VersionOrdering {
    if let (Some((left_day, left_revision)), Some((right_day, right_revision))) =
        (dated_patch_version(left), dated_patch_version(right))
    {
        left_day.cmp(&right_day).then_with(|| decimal_version_cmp(left_revision, right_revision))
    } else {
        decimal_version_cmp(left, right)
    }
}

pub fn observe_patch_version_mismatch(resource: &str, official_version: &str, local_version: Option<&str>) {
    if !matches!(resource, "battle" | "backpack" | "notice" | "control" | "start")
        || official_version.is_empty() || !official_version.bytes().all(|byte| byte.is_ascii_digit())
    {
        return;
    }
    let mut mismatches = PATCH_MISMATCHES.write().unwrap();
    if let Some(previous) = mismatches.get(resource)
        && (compare_patch_versions(&previous.official_version, official_version).is_gt()
            || (previous.official_version == official_version
                && previous.local_version.as_deref() == local_version))
    {
        return;
    }
    mismatches.insert(resource.to_owned(), PatchVersionMismatch {
        resource: resource.to_owned(),
        official_version: official_version.to_owned(),
        local_version: local_version.map(str::to_owned),
    });
    drop(mismatches);
    tracing::warn!(target: "aola_weekly_patch", resource, official_version, local_version = ?local_version,
        "Official SWF returned after patch version mismatch");
}

pub fn clear_patch_version_mismatch(resource: &str, covered_version: &str) {
    let mut mismatches = PATCH_MISMATCHES.write().unwrap();
    if mismatches.get(resource).is_some_and(|previous| {
        compare_patch_versions(&previous.official_version, covered_version).is_le()
    }) {
        mismatches.remove(resource);
    }
}

pub fn patch_version_mismatches() -> Vec<PatchVersionMismatch> {
    PATCH_MISMATCHES.read().unwrap().values().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::compare_patch_versions;

    #[test]
    fn weekly_version_comparison_handles_both_official_date_spellings() {
        assert!(compare_patch_versions("261127000000000", "2026111953323155").is_gt());
        assert!(compare_patch_versions("20261127999", "2611271000").is_lt());
        assert!(compare_patch_versions("2107300001", "2021072975243068").is_gt());
        assert!(compare_patch_versions("8", "12").is_lt());
    }
}

#[derive(Default)]
pub struct MorphRuleSnapshot {
    pub enabled: bool,
    pub revision: u64,
    pub enemy_enabled: bool,
    pub backpack_map: HashMap<u32, u32>,
    // An empty map means battle replacement is disabled at the last boundary.
    pub battle_map: HashMap<u32, u32>,
    pub head_icon_map: HashMap<u32, u32>,
    pub poster_map: HashMap<u32, u32>,
}

static MORPH_RULES: LazyLock<RwLock<Arc<MorphRuleSnapshot>>> = LazyLock::new(Default::default);

pub fn publish_morph_rules(rules: MorphRuleSnapshot) {
    *MORPH_RULES.write().unwrap() = Arc::new(rules);
}

pub fn morph_rule_snapshot() -> Arc<MorphRuleSnapshot> {
    MORPH_RULES.read().unwrap().clone()
}
