//! Opt-in diagnostics for the personal-info activity. Never inspect call arguments.
use std::sync::{OnceLock, atomic::{AtomicBool, AtomicUsize, Ordering}};

static ENABLED: OnceLock<bool> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);
static EVENTS: AtomicUsize = AtomicUsize::new(0);
const LIMIT: usize = 8192;

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("AOLA_PANEL_DIAGNOSTICS").as_deref() == Ok("1"))
}

pub fn armed() -> bool { enabled() && ARMED.load(Ordering::Relaxed) }

fn selected(name: &str) -> bool {
    name.contains("petfightspace3") || [
        "mmo.common.panel::PanelBase/show", "mmo.common.panel::PanelBase/onGetRes(",
        "mmo.common.panel::PanelBase/tryGetFromCurDomain(", "mmo.common.panel::PanelBase/beforeInitPanel(",
        "mmo.common.client::CommonClientHandler/", "mmo.loader::SimpleCustomLoader/",
        "mmo.loader::CustomLoader/", "mmo.loader.scheduler::DomainLoader/",
    ].iter().any(|prefix| name.contains(prefix))
}

pub(crate) fn begin_method(name: &str) -> bool {
    if name.contains("petfightspace3") { ARMED.store(true, Ordering::Relaxed); }
    let selected = armed() && selected(name);
    if selected { emit(name, "method-enter", 0, ""); }
    selected
}

pub fn emit(source: &str, phase: &str, value: u64, stack: &str) {
    if !armed() { return; }
    let count = EVENTS.fetch_add(1, Ordering::Relaxed);
    if count < LIMIT {
        tracing::info!(target: "aola_panel_diagnostic", source, phase, value, stack);
    } else if count == LIMIT {
        tracing::info!(target: "aola_panel_diagnostic", source="diagnostic", phase="limit", value=LIMIT as u64, stack="");
    }
}

pub fn resource_path(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    let path = url.path();
    (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("aola.100bt.com")
        && url.port().is_none() && url.username().is_empty() && url.password().is_none()
        && path.starts_with("/play/activityres/activity231229/petfightspace3/")
        && path.ends_with(".swf") && path.len() <= 512
        && path.bytes().all(|c| c.is_ascii_alphanumeric() || b"/-_.~".contains(&c)))
        .then(|| path.to_owned())
}

pub fn resource(address: &str, phase: &str, value: u64) {
    if !enabled() { return; }
    if let Some(path) = resource_path(address) {
        ARMED.store(true, Ordering::Relaxed);
        emit(&path, phase, value, "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_panel_resource_scope_strips_query_and_rejects_other_origins() {
        let path = "/play/activityres/activity231229/petfightspace3/petfightspace3_code~2026110521924920.swf";
        assert_eq!(resource_path(&format!("https://aola.100bt.com{path}?sid=synthetic-secret")), Some(path.into()));
        for address in [format!("https://aola.100bt.com.invalid{path}"), format!("https://user:secret@aola.100bt.com{path}"),
            format!("https://aola.100bt.com:444{path}"), "https://aola.100bt.com/play/pet/newpetpackage.swf".into()] {
            assert_eq!(resource_path(&address), None);
        }
    }
    #[test]
    fn aola_panel_method_scope_includes_loader_and_excludes_pet_detail() {
        assert!(selected("abc#1 mmo.activity231229.petfightspace3.panel.personalinfo::PFSpace3_PersonalInfoPl/showPanel()"));
        assert!(selected("abc#2 mmo.loader.scheduler::DomainLoader/onLoadByteComplete()"));
        assert!(!selected("abc#3 mmo.pet.dataservice::PetDataService/onViewSinglePet()"));
    }
}
