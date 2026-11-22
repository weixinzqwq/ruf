//! Bounded, metadata-only call accounting during one pet-detail response.
use std::collections::HashMap;
use web_time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailMode {
    #[default]
    Full,
    Summary,
}

impl DetailMode {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("summary") => Self::Summary,
            _ => Self::Full,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Summary => "summary",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailRoot {
    ViewSinglePet,
    DispatchCachedDetail,
}

impl DetailRoot {
    // Resolve the qualified class once while its traits are loaded. Neither
    // method indexes nor ABC method debug names are stable across releases.
    pub(crate) fn for_class(name: &crate::string::WStr, namespace: Option<&crate::string::WStr>) -> Option<Self> {
        use crate::string::WStr;
        if name == WStr::from_units(b"PetDataService") && namespace == Some(WStr::from_units(b"mmo.pet.dataservice")) {
            Some(Self::ViewSinglePet)
        } else if name == WStr::from_units(b"PetDetailDataMap") && namespace == Some(WStr::from_units(b"mmo.petpackage.data")) {
            Some(Self::DispatchCachedDetail)
        } else {
            None
        }
    }

    pub(crate) fn member(self) -> &'static crate::string::WStr {
        use crate::string::WStr;
        match self {
            Self::ViewSinglePet => WStr::from_units(b"onViewSinglePet"),
            Self::DispatchCachedDetail => WStr::from_units(b"tryDispatcherCurrentDetailedCachedEvent"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ViewSinglePet => "PetDataService/onViewSinglePet",
            Self::DispatchCachedDetail => "PetDetailDataMap/tryDispatcherCurrentDetailedCachedEvent",
        }
    }
}

#[derive(Default, Debug)]
pub struct MethodCost {
    pub name: String,
    pub calls: u64,
    pub total_ns: u64,
    pub self_ns: u64,
    pub max_ns: u64,
}

#[derive(Default, Debug)]
pub struct ResponseCost {
    pub sequence: u64,
    pub root: Option<DetailRoot>,
    pub stage: Option<usize>,
    pub total_ns: u64,
    pub bookkeeping_ns: u64,
    pub unlisted_calls: u64,
    pub unlisted_self_ns: u64,
    pub resource_creation_ns: u64,
    pub resource_creations: u64,
    pub mesh_registration_ns: u64,
    pub submit_ns: u64,
    pub methods: Vec<MethodCost>,
    pub goto: Option<Box<crate::goto_profile::GotoStats>>,
    pub orphan_census: Option<Box<crate::orphan_census::Census>>,
}

pub(crate) struct Token { start: Instant, children: u64, index: Option<usize> }

#[derive(Default)]
pub(crate) struct DetailProfiler {
    stack: Vec<Token>,
    summary_started: Option<Instant>,
    names: HashMap<String, usize>,
    current: ResponseCost,
    sequence: u64,
    completed: Vec<ResponseCost>,
    dropped: u64,
}

impl DetailProfiler {
    pub(crate) fn just_completed(&mut self) -> Option<&mut ResponseCost> {
        self.completed.last_mut().filter(|row| row.sequence == self.sequence && self.stack.is_empty())
    }
    pub fn active(&self) -> bool { !self.stack.is_empty() }
    pub fn set_stage(&mut self, stage: Option<usize>) { self.current.stage = stage; }
    pub fn set_goto(&mut self, stats: Option<Box<crate::goto_profile::GotoStats>>) { self.current.goto = stats; }

    pub fn begin_summary(&mut self, root: DetailRoot, stage: Option<usize>) -> bool {
        if self.summary_started.is_some() { return false; }
        self.sequence += 1;
        self.current.sequence = self.sequence;
        self.current.root = Some(root);
        self.current.stage = stage;
        self.summary_started = Some(Instant::now());
        true
    }

    pub fn finish_summary(&mut self) {
        self.current.total_ns = self.summary_started.take().expect("paired summary root").elapsed().as_nanos() as u64;
        self.complete();
    }

    fn complete(&mut self) {
        let record = std::mem::take(&mut self.current);
        if self.completed.len() < 16 { self.completed.push(record); }
        else { self.dropped += 1; }
    }

    pub fn begin(&mut self, name: &str) -> bool {
        let root = if self.active() { None } else {
            let Some(root) = root_from_profile_name(name) else { return false; };
            Some(root)
        };
        let start = Instant::now();
        if let Some(root) = root {
            self.sequence += 1;
            self.current.sequence = self.sequence;
            self.current.root = Some(root);
        }
        let index = if let Some(index) = self.names.get(name) { Some(*index) }
        else if self.names.len() < 1024 {
            let index = self.current.methods.len();
            self.names.insert(name.to_owned(), index);
            self.current.methods.push(MethodCost { name: name.to_owned(), ..Default::default() });
            Some(index)
        } else { self.current.unlisted_calls += 1; None };
        self.current.bookkeeping_ns += start.elapsed().as_nanos() as u64;
        self.stack.push(Token { start: Instant::now(), children: 0, index });
        true
    }

    pub fn finish(&mut self) {
        let end = Instant::now();
        let token = self.stack.pop().expect("paired detail profiling");
        let ns = end.duration_since(token.start).as_nanos() as u64;
        if let Some(index) = token.index {
            let row = &mut self.current.methods[index];
            row.calls += 1;
            row.total_ns += ns;
            row.self_ns += ns.saturating_sub(token.children);
            row.max_ns = row.max_ns.max(ns);
        } else { self.current.unlisted_self_ns += ns.saturating_sub(token.children); }
        if let Some(parent) = self.stack.last_mut() { parent.children += ns; }
        else {
            self.current.total_ns = ns;
            self.current.methods.sort_by_key(|r| std::cmp::Reverse(r.self_ns));
            // Keep all bounded rows: callers need both small/high-count methods
            // and expensive parents, not just a slow-call threshold.
            self.complete();
            self.names.clear();
        }
        if self.active() { self.current.bookkeeping_ns += end.elapsed().as_nanos() as u64; }
    }

    pub fn take(&mut self) -> (Vec<ResponseCost>, u64) {
        (std::mem::take(&mut self.completed), std::mem::take(&mut self.dropped))
    }

    pub fn resources(&mut self, before: ruffle_render::backend::RenderResourceStats, after: ruffle_render::backend::RenderResourceStats) {
        let totals = |s: ruffle_render::backend::RenderResourceStats| {
            // Mesh itself is an owner record; individual buffers are charged below.
            [s.uploaded_bitmap,s.offscreen_bitmap,s.bitmap_cache,s.temporary_texture,s.readback_buffer,
             s.mesh_vertex_buffer,s.mesh_index_buffer,s.mesh_uniform_buffer,s.gradient_texture]
                .iter().fold((0u64,0u64), |(ns,c), r| (ns+r.creation_cpu_ns,c+r.timed_creation_count))
        };
        let (bns,bc)=totals(before); let (ans,ac)=totals(after);
        self.current.resource_creation_ns=ans.saturating_sub(bns);
        self.current.resource_creations=ac.saturating_sub(bc);
        self.current.mesh_registration_ns=after.work.mesh_registration_ns.saturating_sub(before.work.mesh_registration_ns);
        self.current.submit_ns=after.work.queue_submit_ns.saturating_sub(before.work.queue_submit_ns);
    }
}

fn root_from_profile_name(name: &str) -> Option<DetailRoot> {
    if name.contains("PetDataService/onViewSinglePet(") { Some(DetailRoot::ViewSinglePet) }
    else if name.contains("PetDetailDataMap/tryDispatcherCurrentDetailedCachedEvent(") { Some(DetailRoot::DispatchCachedDetail) }
    else { None }
}

/// Labels describe observed method boundaries, not a claim that internal native
/// work (e.g. glyph upload inside text layout) has been separately instrumented.
pub fn category(name: &str) -> &'static str {
    if name.contains("/parseInfo(") || name.contains("/getPetDetailInfo(") || name.contains("::PetDetailedInfo(") { "detailData" }
    else if name.contains("/gotoAndStop(") || name.contains("/gotoAndPlay(") { "timeline" }
    else if name.contains("TextField/") || name.contains("TextFormat/") || name.contains("::TextField(") { "text" }
    else if name.contains("BitmapData/") || name.contains("Loader/") || name.contains("::BitmapData(") { "resource" }
    else if name.contains("/addChild") || name.contains("/removeChild") { "displayList" }
    else if name.contains("CardList/") || name.contains("PetList/") { "listRefresh" }
    else { "scriptOrOtherNative" }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_summary_recognizes_qualified_owners_and_explicit_setting() {
        use crate::string::WStr;
        let class = WStr::from_units(b"PetDetailDataMap");
        let package = Some(WStr::from_units(b"mmo.petpackage.data"));
        assert_eq!(DetailRoot::for_class(class, package), Some(DetailRoot::DispatchCachedDetail));
        assert_eq!(DetailRoot::for_class(class, Some(WStr::from_units(b"other"))), None);
        assert_eq!(DetailRoot::for_class(WStr::from_units(b"Other"), package), None);
        assert_eq!(DetailMode::from_setting(None), DetailMode::Full);
        assert_eq!(DetailMode::from_setting(Some("full")), DetailMode::Full);
        assert_eq!(DetailMode::from_setting(Some("summary")), DetailMode::Summary);
    }

    #[test]
    fn aola_detail_nested_counts_and_drain() {
        let mut p = DetailProfiler::default();
        assert!(!p.begin("unrelated"));
        assert!(p.begin("mmo.pet.dataservice::PetDataService/onViewSinglePet()"));
        for _ in 0..3 { assert!(p.begin("parseInfo")); p.finish(); }
        p.finish();
        let (rows, dropped) = p.take();
        assert_eq!(dropped,0);
        assert_eq!(rows.len(),1);
        assert_eq!(rows[0].methods.iter().find(|r|r.name=="parseInfo").unwrap().calls,3);
        assert_eq!(rows[0].methods.iter().map(|r|r.self_ns).sum::<u64>(),rows[0].total_ns);
        assert!(!p.active()); assert!(p.take().0.is_empty());
    }
    #[test]
    fn aola_detail_bounded_without_retaining_gc_objects() {
        let mut p = DetailProfiler::default();
        for _ in 0..18 {
            p.begin("PetDetailDataMap/tryDispatcherCurrentDetailedCachedEvent()");
            for i in 0..1030 { p.begin(&format!("child{i}")); p.finish(); }
            p.finish();
        }
        let (rows,dropped)=p.take();
        assert_eq!(rows.len(),16); assert_eq!(dropped,2);
        assert_eq!(rows[0].methods.len(),1024); assert_eq!(rows[0].unlisted_calls,7);
    }
    #[test]
    fn aola_detail_resource_deltas_do_not_double_count_mesh_owners() {
        let mut p = DetailProfiler::default();
        p.begin("PetDataService/onViewSinglePet()");
        let before = ruffle_render::backend::RenderResourceStats::default();
        let mut after = before;
        after.mesh.creation_cpu_ns = 99;
        after.mesh_vertex_buffer.creation_cpu_ns = 12;
        after.mesh_vertex_buffer.timed_creation_count = 1;
        after.gradient_texture.creation_cpu_ns = 7;
        after.gradient_texture.timed_creation_count = 1;
        after.work.mesh_registration_ns = 23;
        after.work.queue_submit_ns = 31;
        p.resources(before,after); p.finish();
        let r = p.take().0.remove(0);
        assert_eq!((r.resource_creation_ns,r.resource_creations),(19,2));
        assert_eq!((r.mesh_registration_ns,r.submit_ns),(23,31));
    }
}
