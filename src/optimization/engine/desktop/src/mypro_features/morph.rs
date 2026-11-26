use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const LIMIT: usize = 500;
#[derive(Clone, Default)]
struct Rule {
    note: String,
    backpack: Option<u32>,
    fight: Option<u32>,
    portrait: Option<u32>,
}
#[derive(Clone, Default)]
struct Rules {
    revision: u64,
    enabled: bool,
    enemy: bool,
    rows: BTreeMap<u32, Rule>,
}

fn id(v: &Value) -> Result<Option<u32>, String> {
    if v.is_null() || v.as_str().is_some_and(|s| matches!(s.trim(), "" | "/")) {
        return Ok(None);
    }
    let n = v
        .as_u64()
        .or_else(|| v.as_str()?.trim().parse::<u64>().ok())
        .filter(|n| (1..=i32::MAX as u64).contains(n))
        .ok_or("InvalidArguments: invalid asset ID")?;
    Ok(Some(n as u32))
}
impl Rules {
    fn parse(p: &Value, revision: u64) -> Result<Self, String> {
        let rules = p.get("rules").ok_or("InvalidArguments: rules")?;
        let rows = rules
            .as_array()
            .or_else(|| rules.get("skins")?.as_array())
            .filter(|a| a.len() <= LIMIT)
            .ok_or("InvalidArguments: expected up to 500 rules")?;
        let mut result = Self {
            revision,
            enabled: p["enabled"].as_bool().ok_or("InvalidArguments: enabled")?,
            enemy: p["enemy"].as_bool().ok_or("InvalidArguments: enemy")?,
            rows: BTreeMap::new(),
        };
        for (i, row) in rows.iter().enumerate() {
            let source = id(&row["originalId"])?
                .ok_or_else(|| format!("InvalidArguments: missing originalId at row {}", i + 1))?;
            let note = row
                .get("remark")
                .map(|v| v.as_str().ok_or("InvalidArguments: remark"))
                .transpose()?
                .unwrap_or("");
            if note.chars().count() > 128 {
                return Err("InvalidArguments: remark length".into());
            }
            let rule = Rule {
                note: note.into(),
                backpack: id(&row["backpackId"])?,
                fight: id(&row["fightId"])?,
                portrait: id(&row["spskiId"])?,
            };
            if result.rows.insert(source, rule).is_some() {
                return Err(format!("InvalidArguments: duplicate originalId {source}"));
            }
        }
        Ok(result)
    }
    fn file(&self) -> Value {
        let field = |id: Option<u32>| id.map(|n| n.to_string()).unwrap_or_else(|| "/".into());
        let rules = self
            .rows
            .iter()
            .map(|(k, r)| {
                json!({"originalId":k.to_string(),"backpackId":field(r.backpack),
            "fightId":field(r.fight),"spskiId":field(r.portrait),"remark":r.note})
            })
            .collect::<Vec<_>>();
        json!({"schemaVersion":1,"revision":self.revision,"rules":rules,"enabled":self.enabled,"enemy":self.enemy})
    }
}

struct State {
    desired: Arc<Rules>,
    battle_rules: Arc<Rules>,
    in_battle: bool,
    battle_id: Option<u64>,
    binding: u32,
    bound: [bool; 4],
    hits: [u64; 4],
    waiting: u64,
    recent: VecDeque<Lookup>,
    stage: &'static str,
    error: Option<String>,
}
struct Lookup {
    kind: &'static str,
    source: u32,
    target: u32,
    face: bool,
    binding: u32,
    state: &'static str,
    reason: Option<&'static str>,
    preparation: Option<Preparation>,
}
#[derive(Clone, Copy, PartialEq)]
struct Preparation {
    stage: &'static str,
    part: u32,
    total: u32,
}

pub fn readiness_reason(code: &str) -> Option<&'static str> {
    [
        "pm-data-missing",
        "config-missing",
        "frame-missing",
        "invalid-config",
        "class-mismatch",
        "preload-pending",
        "preload-error",
        "preload-timeout",
        "display-check-error",
        "unsupported-id",
    ]
    .into_iter()
    .find(|&known| known == code)
}
fn channel(name: &str) -> Option<(usize, &'static str)> {
    ["body", "icon", "backpack", "portrait"]
        .into_iter()
        .enumerate()
        .find(|(_, s)| *s == name)
}
impl Default for State {
    fn default() -> Self {
        Self {
            desired: Default::default(),
            battle_rules: Default::default(),
            in_battle: false,
            battle_id: None,
            binding: 1,
            bound: [false; 4],
            hits: [0; 4],
            waiting: 0,
            recent: VecDeque::new(),
            stage: "loading",
            error: None,
        }
    }
}
impl State {
    fn publish(&mut self, r: Rules) {
        self.desired = Arc::new(r);
        if !self.in_battle {
            self.battle_rules = self.desired.clone();
        }
        self.stage = "ready";
        self.error = None;
    }
    fn boundary(&mut self, in_battle: bool) {
        self.in_battle = in_battle;
        self.battle_rules = self.desired.clone();
        self.binding = self.binding.wrapping_add(1).max(1);
        if !in_battle {
            self.battle_id = None;
        }
    }
    fn proposed(&self, kind: &str, source: u32, face: bool) -> u32 {
        let fallback = if kind == "portrait" { 0 } else { source };
        let rules = if matches!(kind, "backpack" | "portrait") {
            &self.desired
        } else {
            &self.battle_rules
        };
        if !rules.enabled {
            return fallback;
        }
        let Some(rule) = rules.rows.get(&source) else {
            return fallback;
        };
        match kind {
            "body" if !face || rules.enemy => rule.fight,
            "icon" => rule.fight,
            "backpack" => rule.backpack,
            "portrait" => rule.portrait,
            _ => None,
        }
        .unwrap_or(fallback)
    }
    fn lookup(&mut self, kind: &'static str, source: u32, target: u32, face: bool) -> &mut Lookup {
        let previous = self
            .recent
            .iter()
            .position(|r| r.kind == kind && r.source == source && r.face == face)
            .and_then(|index| self.recent.remove(index));
        let row = previous
            .filter(|r| r.target == target && r.binding == self.binding)
            .unwrap_or(Lookup {
                kind,
                source,
                target,
                face,
                binding: self.binding,
                state: if target == 0 || (target == source && kind != "portrait") {
                    "original"
                } else {
                    "requested"
                },
                reason: None,
                preparation: None,
            });
        if self.recent.len() >= 24 {
            self.recent.pop_back();
        }
        self.recent.push_front(row);
        self.recent.front_mut().unwrap()
    }
    fn candidate(&mut self, kind: &'static str, source: u32, face: bool) -> u32 {
        let target = self.proposed(kind, source, face);
        self.lookup(kind, source, target, face);
        target
    }
    fn use_target(
        &mut self,
        kind: &str,
        source: u32,
        target: u32,
        face: bool,
        ready: bool,
        reason: Option<&'static str>,
    ) -> (u32, bool) {
        let Some((index, kind)) = channel(kind) else {
            return (source, false);
        };
        if target == 0 || (source == target && kind != "portrait") || self.proposed(kind, source, face) != target {
            return (if kind == "portrait" { 0 } else { source }, false);
        }
        let row = self.lookup(kind, source, target, face);
        let status = if ready { "selected" } else { "unavailable" };
        let reason = if ready { None } else { reason };
        let changed = row.state != status || row.reason != reason;
        row.state = status;
        row.reason = reason;
        if ready {
            self.hits[index] += 1;
            (target, changed)
        } else {
            self.waiting += 1;
            (source, changed)
        }
    }
    fn note_progress(
        &mut self,
        kind: &str,
        source: u32,
        target: u32,
        face: bool,
        binding: u32,
        progress: Preparation,
    ) -> bool {
        if binding != self.binding
            || source == target
            || self.proposed(kind, source, face) != target
        {
            return false;
        }
        let Some(row) = self.recent.iter_mut().find(|r| {
            r.kind == kind
                && r.source == source
                && r.target == target
                && r.face == face
                && r.binding == binding
        }) else {
            return false;
        };
        if row.preparation == Some(progress) {
            return false;
        }
        row.preparation = Some(progress);
        true
    }
    fn snapshot(&self) -> Value {
        let mut result = self.desired.file();
        let o = result.as_object_mut().unwrap();
        o.insert("activeRevision".into(), json!(self.battle_rules.revision));
        o.insert("battleBound".into(), json!(self.in_battle));
        o.insert("battleId".into(), json!(self.battle_id));
        o.insert("bindingId".into(), json!(self.binding));
        o.insert("stage".into(), json!(self.stage));
        o.insert("error".into(), json!(self.error));
        o.insert("hits".into(),json!({"body":self.hits[0],"icon":self.hits[1],"backpack":self.hits[2],"portrait":self.hits[3],"waiting":self.waiting}));
        o.insert("recent".into(),Value::Array(self.recent.iter().map(|e|json!({"kind":e.kind,"source":e.source,
            "target":e.target,"face":e.face,"state":e.state,"reason":e.reason,
            "preparation":e.preparation.map(|p|json!({"stage":p.stage,"part":p.part,"total":p.total}))})).collect()));
        let mut channels = json!({});
        for (i, name) in ["body", "icon", "backpack", "portrait"].into_iter().enumerate() {
            channels[name] = json!({"available":self.bound[i],"reason":if self.bound[i]{"当前会话已到达展示入口"}else{"等待当前会话进入已识别的展示入口"}});
        }
        o.insert("capabilities".into(), channels);
        result
    }
}

static CURRENT: OnceLock<Mutex<State>> = OnceLock::new();
fn state() -> &'static Mutex<State> {
    CURRENT.get_or_init(Default::default)
}
pub(super) fn session() {
    let mut s = state().lock().unwrap();
    s.boundary(false);
    s.bound = [false; 4];
    s.hits = [0; 4];
    s.waiting = 0;
    s.recent.clear();
}
pub(super) fn battle(enter: bool, id: u64) {
    let mut s = state().lock().unwrap();
    s.boundary(enter);
    s.battle_id = enter.then_some(id);
}
pub fn binding() -> u32 {
    state().lock().unwrap().binding
}

pub fn candidate(kind: &str, source: u32, face: bool) -> u32 {
    let mut s = state().lock().unwrap();
    let Some((i, kind)) = channel(kind) else {
        return source;
    };
    s.bound[i] = true;
    s.candidate(kind, source, face)
}
pub fn apply(
    kind: &str,
    source: u32,
    target: u32,
    face: bool,
    ready: bool,
    reason: Option<&'static str>,
) -> (u32, bool) {
    state()
        .lock()
        .unwrap()
        .use_target(kind, source, target, face, ready, reason)
}
pub fn progress(
    kind: &str,
    source: u32,
    target: u32,
    face: bool,
    binding: u32,
    stage: &str,
    part: u32,
    total: u32,
) -> bool {
    let Some(stage) = ["loading", "loaded", "load-error", "timeout"]
        .into_iter()
        .find(|&s| s == stage)
    else {
        return false;
    };
    if !(1..=16).contains(&total)
        || !(1..=total).contains(&part)
        || (stage == "loaded" && part != total)
    {
        return false;
    }
    state().lock().unwrap().note_progress(
        kind,
        source,
        target,
        face,
        binding,
        Preparation { stage, part, total },
    )
}

fn load(root: &Path) -> Result<Rules, String> {
    let file = match fs::File::open(root.join("morph.json")) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Rules::default()),
        other => other.map_err(|e| e.to_string())?,
    };
    let mut data = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 1_048_576 {
        return Err("InvalidArguments: morph file size".into());
    }
    let v: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    if v["schemaVersion"] != 1 {
        return Err("InvalidArguments: morph file version".into());
    }
    Rules::parse(
        &v,
        v["revision"].as_u64().ok_or("InvalidArguments: revision")?,
    )
}
fn persist(root: &Path, r: &Rules) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("morph.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|_| "Busy: morph rules")?;
    if load(root)?.revision.checked_add(1) != Some(r.revision) {
        return Err("StaleContext: morph revision".into());
    }
    let tmp = root.join(format!("morph-{}.tmp", std::process::id()));
    let path = root.join("morph.json");
    let write = || -> io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&mut f, &r.file())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        drop(f);
        if path.exists() {
            fs::copy(&path, root.join("morph.previous.json"))?;
        }
        fs::rename(&tmp, &path)
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result.map_err(|e| e.to_string())
}
fn finish(result: Result<Rules, String>) {
    let mut s = state().lock().unwrap();
    match result {
        Ok(r) => s.publish(r),
        Err(e) => {
            s.stage = "failed";
            s.error = Some(e);
        }
    }
}
pub(super) fn init(root: PathBuf) {
    std::thread::spawn(move || finish(load(&root)));
}
pub(super) fn command(root: &Path, name: &str, p: &Value) -> Option<Result<Value, String>> {
    match name {
        "morph.rules.read" => Some(Ok(state().lock().unwrap().snapshot())),
        "morph.rules.replace" | "morph.restore" => Some(change(root, name, p)),
        _ => None,
    }
}
fn change(root: &Path, name: &str, p: &Value) -> Result<Value, String> {
    let mut s = state().lock().unwrap();
    if matches!(s.stage, "loading" | "saving") {
        return Err("Busy: morph rules".into());
    }
    if p["expectedRevision"].as_u64() != Some(s.desired.revision) {
        return Err("StaleContext: morph revision".into());
    }
    let rev = s
        .desired
        .revision
        .checked_add(1)
        .ok_or("InvalidArguments: revision")?;
    let next = if name == "morph.restore" {
        let mut r = (*s.desired).clone();
        r.enabled = false;
        r.revision = rev;
        r
    } else {
        Rules::parse(p, rev)?
    };
    s.stage = "saving";
    let pending = s.snapshot();
    drop(s);
    let root = root.to_owned();
    std::thread::spawn(move || {
        let result = persist(&root, &next).map(|()| next);
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.starts_with("StaleContext"))
            && let Ok(latest) = load(&root)
        {
            finish(Ok(latest));
        }
        finish(result)
    });
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_poster_uses_configured_skin_even_when_it_has_the_same_number_as_the_pet() {
        let mut s = State::default();
        assert_eq!(s.candidate("portrait",17,false),0);
        s.boundary(true);
        let p=json!({"rules":[{"originalId":17,"spskiId":17}],"enabled":true,"enemy":false});
        s.publish(Rules::parse(&p,1).unwrap());
        assert_eq!(s.candidate("portrait",17,false),17);
        assert_eq!(s.use_target("portrait",17,17,false,true,None),(17,true));
        assert_eq!(s.snapshot()["hits"]["portrait"],1);
        s.publish(Rules::default());
        assert_eq!(s.use_target("portrait",17,17,false,true,None),(0,false));
    }
    #[test]
    fn aola_mypro_morph_keeps_identity_and_battle_binding() {
        let p = json!({"enabled":true,"enemy":false,"rules":{"skins":[{"originalId":"4","fightId":7,"backpackId":"/"},{"originalId":7,"fightId":"9"}]}});
        let mut s = State::default();
        s.publish(Rules::parse(&p, 1).unwrap());
        s.boundary(true);
        assert_eq!(s.proposed("body", 4, true), 4);
        assert_eq!(s.proposed("icon", 4, false), 7);
        assert_eq!(
            s.use_target("body", 4, 7, false, false, Some("frame-missing")),
            (4, true)
        );
        assert_eq!(s.candidate("body", 4, false), 7);
        assert_eq!(s.recent[0].state, "unavailable");
        let part = Preparation {
            stage: "loading",
            part: 2,
            total: 3,
        };
        assert_eq!(s.recent[0].reason, Some("frame-missing"));
        assert!(s.note_progress("body", 4, 7, false, s.binding, part));
        assert!(!s.note_progress("body", 4, 7, false, s.binding, part));
        assert_eq!(s.use_target("body", 4, 7, false, true, None), (7, true));
        s.publish(Rules {
            revision: 2,
            ..Default::default()
        });
        assert_eq!(s.proposed("body", 4, false), 7);
        s.boundary(false);
        assert_eq!(s.proposed("body", 4, false), 4);
        assert!(!s.note_progress("body", 4, 7, false, s.binding - 1, part));
        let mut invalid = p;
        invalid["rules"]["skins"][1]["originalId"] = json!("4");
        assert!(Rules::parse(&invalid, 3).is_err());
    }
    #[test]
    fn aola_mypro_legacy_rules_roundtrip_and_restore_keeps_user_entries() {
        let root = std::env::temp_dir().join(format!(
            "aola-mypro-rule-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let v = json!({"rules":[{"originalId":"17","backpackId":22,"fightId":"/","spskiId":null,"remark":"展示"}],"enabled":true,"enemy":false});
        let r = Rules::parse(&v, 1).unwrap();
        persist(&root, &r).unwrap();
        assert_eq!(load(&root).unwrap().file(), r.file());
        let disabled = Rules {
            enabled: false,
            revision: 2,
            ..r.clone()
        };
        persist(&root, &disabled).unwrap();
        assert!(persist(&root, &r).is_err());
        let stored = load(&root).unwrap().file();
        assert_eq!(stored["rules"][0], r.file()["rules"][0]);
        assert_eq!(stored["enabled"], false);
        assert_eq!(
            Rules::parse(
                &serde_json::from_slice(&fs::read(root.join("morph.previous.json")).unwrap())
                    .unwrap(),
                1
            )
            .unwrap()
            .file(),
            r.file()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
