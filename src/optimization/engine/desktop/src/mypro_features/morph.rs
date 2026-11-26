use ruffle_core::mypro_observer::{MorphRuleSnapshot, publish_morph_rules};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
    stage: &'static str,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            desired: Default::default(),
            battle_rules: Default::default(),
            in_battle: false,
            battle_id: None,
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
        if !in_battle {
            self.battle_id = None;
        }
    }
    fn rule_snapshot(&self) -> MorphRuleSnapshot {
        let mappings = |rules: &Rules, target: fn(&Rule) -> Option<u32>| {
            rules
                .rows
                .iter()
                .filter(|_| rules.enabled)
                .filter_map(|(&source, rule)| {
                    target(rule).filter(|&id| id != source).map(|id| (source, id))
                })
                .collect()
        };
        MorphRuleSnapshot {
            enabled: self.desired.enabled,
            revision: self.desired.revision,
            enemy_enabled: self.battle_rules.enabled && self.battle_rules.enemy,
            backpack_map: mappings(&self.desired, |r| r.backpack),
            battle_map: mappings(&self.battle_rules, |r| r.fight),
            head_icon_map: mappings(&self.battle_rules, |r| r.fight.or(r.backpack)),
            poster_map: self.desired.rows.iter()
                .filter(|_| self.desired.enabled)
                .filter_map(|(&source, row)| row.portrait.map(|skin| (source, skin)))
                .collect(),
        }
    }

    fn snapshot(&self) -> Value {
        let mut result = self.desired.file();
        let o = result.as_object_mut().unwrap();
        o.insert("activeRevision".into(), json!(self.battle_rules.revision));
        o.insert("battleBound".into(), json!(self.in_battle));
        o.insert("battleId".into(), json!(self.battle_id));
        o.insert("stage".into(), json!(self.stage));
        o.insert("error".into(), json!(self.error));
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
    publish_morph_rules(s.rule_snapshot());
}
pub(super) fn battle(enter: bool, id: u64) {
    let mut s = state().lock().unwrap();
    s.boundary(enter);
    s.battle_id = enter.then_some(id);
    publish_morph_rules(s.rule_snapshot());
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
        Ok(r) => {
            s.publish(r);
            publish_morph_rules(s.rule_snapshot());
        }
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
    fn aola_mypro_morph_keeps_identity_and_battle_binding() {
        let p = json!({"enabled":true,"enemy":true,"rules":{"skins":[
            {"originalId":"4","fightId":7,"backpackId":12,"spskiId":45},
            {"originalId":7,"fightId":"9"},
            {"originalId":8,"backpackId":10,"spskiId":8}
        ]}});
        let mut s = State::default();
        s.publish(Rules::parse(&p, 1).unwrap());
        s.boundary(true);

        let active = s.rule_snapshot();
        assert!(active.enabled);
        assert!(active.enemy_enabled);
        assert_eq!(active.battle_map.get(&4), Some(&7));
        assert_eq!(active.head_icon_map.get(&4), Some(&7));
        assert_eq!(active.head_icon_map.get(&8), Some(&10));
        assert_eq!(active.backpack_map.get(&4), Some(&12));
        assert_eq!(active.poster_map.get(&4), Some(&45));
        assert_eq!(active.poster_map.get(&8), Some(&8));

        s.publish(Rules {
            revision: 2,
            ..Default::default()
        });
        let restored = s.rule_snapshot();
        assert_eq!(restored.revision, 2);
        assert!(!restored.enabled);
        assert!(restored.enemy_enabled);
        assert_eq!(restored.battle_map.get(&4), Some(&7));
        assert_eq!(restored.head_icon_map.get(&8), Some(&10));
        assert!(restored.backpack_map.is_empty());
        assert!(restored.poster_map.is_empty());

        s.boundary(false);
        assert!(s.rule_snapshot().battle_map.is_empty());
        assert!(s.rule_snapshot().head_icon_map.is_empty());
        assert!(!s.rule_snapshot().enemy_enabled);
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
