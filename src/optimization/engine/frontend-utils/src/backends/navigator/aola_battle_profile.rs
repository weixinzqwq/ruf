// The weekly offline builder can replace this profile after verifying both patch stages.
pub(super) const VERSION: &str = "261112849126071";
pub(super) const PACKED: usize = 5377898;
pub(super) const FWS: usize = 6573705;
pub(super) const BASE_EDITS: &[(usize, &[u8], &[u8])] = super::aola_battle_edits::EDITS;
mod model { include!("aola_mypro_battle.rs"); }
pub(super) const MODEL_EDITS: super::ModelEdits = model::EDITS;
