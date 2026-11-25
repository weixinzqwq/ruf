//! Metadata-only snapshot of the AVM2 weak orphan list after a profiled detail response.
use crate::avm2::TObject;
use crate::context::UpdateContext;
use crate::display_object::{DisplayObject, TDisplayObject, TDisplayObjectContainer};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use web_time::Instant;

pub const SAMPLE_LIMIT: usize = 256;
pub const GROUP_LIMIT: usize = 16;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootGroup {
    pub class: &'static str,
    pub movie: &'static str,
    pub symbol: Option<u16>,
    pub playback: &'static str,
    pub leaf: bool,
}

#[derive(Default, Debug)]
pub struct Census {
    pub slots: usize,
    pub sampled: usize,
    pub valid_roots: usize,
    pub expired: usize,
    pub reparented: usize,
    pub unknown_class: usize,
    pub unknown_movie: usize,
    pub omitted_roots: usize,
    pub omitted_groups: usize,
    pub groups: Vec<(RootGroup, usize)>,
    pub scan_us: u64,
}

// Group resource URLs by a fixed label. Never log an address or instance property.
fn movie_label(url: &str) -> &'static str {
    let Some(path) = url.strip_prefix("https://aola.100bt.com/play/")
        .or_else(|| url.strip_prefix("http://aola.100bt.com/play/")) else { return "unknown"; };
    if path.len() > 2048 { return "unknown"; }
    let clean = path.split(['?', '#']).next().unwrap_or_default();
    match clean {
        "pet/newpetpackage~2026111953323155.swf" => "backpack-2026111953323155",
        "library/util~2026090340574444.swf" => "util-2026090340574444",
        _ if clean.starts_with("library/") => "other-library",
        _ if clean.starts_with("peticon/") || clean.starts_with("gqpeticon/") => "pet-icon",
        _ if clean.starts_with("pet/") || clean.starts_with("gqpet/") => "other-pet",
        _ if clean.starts_with("fightassets/") => "fight-assets",
        _ if clean.starts_with("activityres/") => "activity",
        _ => "unknown",
    }
}

fn class_label(namespace: &str, name: &str) -> &'static str {
    match (namespace, name) {
        ("flash.display", "MovieClip") => "MovieClip",
        ("flash.display", "Sprite") => "Sprite",
        ("flash.display", "Bitmap") => "Bitmap",
        ("flash.display", "Loader") => "Loader",
        ("mmo.petpackage.panel", "PetPackagePanel") => "PetPackagePanel",
        ("mmo.petpackage.panel", "SimpleViewController") => "SimpleViewController",
        ("mmo.petpackage.panel.inclusion", "PetCardList") => "PetCardList",
        ("mmo.petpackage.panel.inclusion", "GodCardList") => "GodCardList",
        ("mmo.petpackage.panel.inclusion", "PetAttrIcon") => "PetAttrIcon",
        ("newpetpackage_fla", _) => "backpack-timeline-class",
        (ns, _) if ns.starts_with("mmo.petpackage.") => "other-backpack-class",
        (ns, _) if ns.starts_with("mmo.") => "other-game-class",
        _ => "unknown",
    }
}

fn sampled_indices(slots: usize) -> impl Iterator<Item = usize> {
    let n = slots.min(SAMPLE_LIMIT);
    let divisor = n.saturating_sub(1).max(1);
    let span = slots.saturating_sub(1);
    (0..n).map(move |i| i * (span / divisor) + i * (span % divisor) / divisor)
}

/// Root-local observations at the start of ordinary frame processing. The registry
/// supplies an already upgraded object; sampling must not change its GC lifetime.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrdinaryRootGroup {
    pub class: String,
    pub movie: &'static str,
    pub symbol: Option<u16>,
    pub playback: &'static str,
    pub children: bool,
    pub pending_script: bool,
    pub script_on_current_frame: bool,
}

#[derive(Debug, Default)]
pub struct OrdinarySnapshot {
    pub frame: u64,
    pub in_battle: bool,
    pub slots: usize,
    pub sampled: usize,
    pub invalid: usize,
    pub omitted_roots: usize,
    pub groups: Vec<(OrdinaryRootGroup, usize)>,
    pub script_calls: usize,
    pub script_receivers: BTreeMap<String, usize>,
}

// Read the declared ABC class metadata without invoking AS3 property access.
fn declared_class(receiver: crate::avm2::StageObject<'_>) -> Option<String> {
    let name = receiver.instance_class().name();
    let ns = name.namespace().as_uri_opt()?;
    let local = name.local_name();
    if ns.len() > 96 || local.len() > 96 { return None; }
    Some(if ns.is_empty() { local.to_string() } else { format!("{ns}::{local}") })
}

impl OrdinarySnapshot {
    pub(crate) fn record_frame_script(&mut self, receiver: crate::avm2::StageObject<'_>) {
        self.script_calls += 1;
        if self.script_calls > SAMPLE_LIMIT { return; }
        let class = declared_class(receiver).unwrap_or_else(|| "unknown".to_owned());
        if let Some(count) = self.script_receivers.get_mut(&class) {
            *count += 1;
        } else if self.script_receivers.len() < GROUP_LIMIT {
            self.script_receivers.insert(class, 1);
        }
    }
}

pub(crate) struct OrdinaryRootSample {
    snapshot: OrdinarySnapshot,
    indices: std::iter::Peekable<std::vec::IntoIter<usize>>,
    groups: BTreeMap<OrdinaryRootGroup, usize>,
}

impl OrdinaryRootSample {
    pub(crate) fn new(slots: usize, frame: u64, in_battle: bool) -> Self {
        Self {
            snapshot: OrdinarySnapshot { slots, frame, in_battle, ..Default::default() },
            indices: sampled_indices(slots).collect::<Vec<_>>().into_iter().peekable(),
            groups: BTreeMap::new(),
        }
    }

    pub(crate) fn observe(&mut self, index: usize, valid: Option<DisplayObject<'_>>) {
        if self.indices.next_if_eq(&index).is_none() { return; }
        self.snapshot.sampled += 1;
        let Some(object) = valid else {
            self.snapshot.invalid += 1;
            return;
        };
        let class = object.object2().and_then(declared_class).unwrap_or_else(|| "unknown".to_owned());
        let clip = object.as_movie_clip();
        let playback = match clip {
            Some(c) if !c.initialized() => "uninitialized",
            Some(c) if !c.playing() => "stopped",
            Some(c) if c.header_frames() > 1 => "playing-multiple-frames",
            Some(_) => "playing-single-or-empty",
            None => "non-timeline",
        };
        let id = object.id();
        let group = OrdinaryRootGroup {
            class, movie: movie_label(object.movie().url()),
            symbol: (id != 0 && id != u16::MAX).then_some(id),
            playback, children: object.as_container().is_some_and(|c| c.num_children() > 0),
            pending_script: clip.is_some_and(|c| c.has_pending_script()),
            script_on_current_frame: clip.is_some_and(|c| c.has_frame_script(c.current_frame())),
        };
        *self.groups.entry(group).or_default() += 1;
    }

    pub(crate) fn finish(mut self) -> OrdinarySnapshot {
        let groups = &mut self.snapshot.groups;
        groups.extend(self.groups);
        groups.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        self.snapshot.omitted_roots = groups.iter().skip(GROUP_LIMIT).map(|(_, count)| count).sum();
        groups.truncate(GROUP_LIMIT);
        self.snapshot
    }
}

impl Census {
    fn take(context: &UpdateContext<'_>) -> Self {
        let start = Instant::now();
        let weak = context.orphan_manager.weak_slots();
        let mut result = Self { slots: weak.len(), ..Default::default() };
        let mut groups = BTreeMap::new();
        for index in sampled_indices(weak.len()) {
            result.sampled += 1;
            let Some(object) = weak[index].upgrade(context.gc()) else {
                result.expired += 1;
                continue;
            };
            if object.parent().is_some() {
                result.reparented += 1;
                continue;
            }
            result.valid_roots += 1;
            let class = object.object2().map_or("unknown", |stage| {
                let qname = stage.instance_class().name();
                let Some(ns) = qname.namespace().as_uri_opt() else { return "unknown"; };
                if ns.len() > 96 || qname.local_name().len() > 96 { return "unknown"; }
                class_label(&ns.to_string(), &qname.local_name().to_string())
            });
            let movie = movie_label(object.movie().url());
            result.unknown_class += usize::from(class == "unknown");
            result.unknown_movie += usize::from(movie == "unknown");
            let id = object.id();
            let symbol = (matches!(movie, "backpack-2026111953323155" | "util-2026090340574444")
                && id != 0 && id != u16::MAX).then_some(id);
            let playback = match object.as_movie_clip() {
                Some(clip) if clip.playing() => "playing",
                Some(_) => "stopped",
                None => "non-timeline",
            };
            let leaf = object.as_container().is_none_or(|c| c.num_children() == 0);
            *groups.entry(RootGroup { class, movie, symbol, playback, leaf }).or_insert(0) += 1;
        }
        result.groups = groups.into_iter().collect();
        result.groups.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        result.omitted_groups = result.groups.len().saturating_sub(GROUP_LIMIT);
        result.omitted_roots = result.groups.iter().skip(GROUP_LIMIT).map(|(_, n)| n).sum();
        result.groups.truncate(GROUP_LIMIT);
        result.scan_us = start.elapsed().as_micros() as u64;
        result
    }
}

// The containing tick includes the scan; the completed response timer does not.
pub(crate) fn after_response(context: &UpdateContext<'_>) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("AOLA_ORPHAN_CENSUS").as_deref() == Ok("1")) {
        return;
    }
    let mut detail = context.tick_profile.detail.borrow_mut();
    if let Some(row) = detail.just_completed() {
        row.orphan_census = Some(Box::new(Census::take(context)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_indices_cover_both_ends_without_duplicates() {
        for slots in [0, 1, 2, 255, 256, 257, 2048, usize::MAX] {
            let sample: Vec<_> = sampled_indices(slots).collect();
            assert_eq!(sample.len(), slots.min(SAMPLE_LIMIT));
            assert!(sample.windows(2).all(|pair| pair[0] < pair[1]));
            if slots > 0 {
                assert_eq!(sample.first(), Some(&0));
                assert_eq!(sample.last(), Some(&(slots - 1)));
            }
        }
    }

    #[test]
    fn classifications_return_only_fixed_public_labels() {
        for url in ["file:///private.swf", "https://aola.100bt.com.invalid/play/pet/test.swf",
                    "https://user@aola.100bt.com/play/pet/test.swf"] {
            assert_eq!(movie_label(url), "unknown");
        }
        assert_eq!(movie_label("https://aola.100bt.com/play/pet/newpetpackage~2026111953323155.swf?test=value"),
                   "backpack-2026111953323155");
        assert_eq!(class_label("mmo.petpackage.private", "arbitrary-text"), "other-backpack-class");
        assert_eq!(class_label("unlisted", "PetPackagePanel"), "unknown");
    }

    #[test]
    fn recognizes_detached_stopped_movie_clips_without_mutating_the_list() {
        let movie = crate::tag_utils::SwfMovieData::from_data(
            include_bytes!("../../tests/tests/swfs/avm2/aoqi_stage3_sleep_wake/Main.swf"),
            "file:///census-test.swf".into(), None, None).unwrap();
        let player = crate::PlayerBuilder::new().with_movie(movie).build();
        let mut player = player.lock().unwrap();
        for _ in 0..3 { player.run_frame(); }
        player.mutate_with_update_context(|context| {
            let before = context.orphan_manager.slots();
            let snapshot = Census::take(context);
            assert_eq!(before, context.orphan_manager.slots());
            assert_eq!(snapshot.sampled, before.min(SAMPLE_LIMIT));
            assert_eq!(snapshot.sampled, snapshot.valid_roots + snapshot.expired + snapshot.reparented);
            assert_eq!(snapshot.valid_roots, snapshot.groups.iter().map(|(_, n)| n).sum::<usize>() + snapshot.omitted_roots);
            assert!(snapshot.groups.iter().any(|(g, n)|
                g.class == "MovieClip" && g.playback == "stopped" && !g.leaf && *n > 0));
        });
    }
}
