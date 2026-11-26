//! Local lineup plans keyed by their owning role. Only the open game backpack can supply a roster.
use crate::player::PlayerController;
use ruffle_core::external::Value as Flash;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    instance_id: u32,
    config_id: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pet {
    pet_id: u32,
    race_id: u32,
    name: String,
    soul_cards: Option<[u32; 5]>,
    soul_artifacts: Option<[Option<Artifact>; 2]>,
    crystal_keys: Option<[u32; 2]>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preset {
    schema_version: u32,
    owner_player_id: u32,
    name: String,
    saved_at: String,
    pets: Vec<Pet>,
}
impl Preset {
    fn validate(&self) -> Result<(), String> {
        validate_name(&self.name)?;
        if self.schema_version != 1
            || self.owner_player_id == 0
            || !(1..=6).contains(&self.pets.len())
        {
            return Err("方案版本、角色或背包数量无效".into());
        }
        let mut pets = HashSet::new();
        let mut items = HashSet::new();
        for pet in &self.pets {
            if pet.pet_id == 0
                || pet.pet_id > i32::MAX as u32
                || pet.race_id == 0
                || !pets.insert(pet.pet_id)
            {
                return Err("方案中的亚比实例无效或重复".into());
            }
            for artifact in pet.soul_artifacts.iter().flatten().flatten() {
                if artifact.instance_id == 0
                    || artifact.config_id == 0
                    || !items.insert(artifact.instance_id)
                {
                    return Err("方案中的魂器实例无效或重复分配".into());
                }
            }
        }
        Ok(())
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.trim() != name
        || name.chars().count() > 64
        || name
            .chars()
            .any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c))
    {
        return Err("请输入 1 至 64 个字符的方案名称，避免文件名特殊字符".into());
    }
    Ok(())
}
type Plans = BTreeMap<(u32, String), Preset>;
fn load(dir: &Path) -> Result<Plans, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut result = Plans::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let mut buf = Vec::new();
        fs::File::open(&path)
            .and_then(|file| file.take(1_048_577).read_to_end(&mut buf))
            .map_err(|e| e.to_string())?;
        if buf.len() > 1_048_576 {
            return Err("阵容文件超过 1 MiB".into());
        }
        let plan: Preset = serde_json::from_slice(&buf)
            .map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
        plan.validate()?;
        if result
            .insert((plan.owner_player_id, plan.name.clone()), plan)
            .is_some()
        {
            return Err("同角色存在同名方案".into());
        }
    }
    Ok(result)
}
fn write(dir: &Path, preset: &Preset) -> Result<(), String> {
    preset.validate()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("lineups.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|_| "另一个播放器正在保存阵容")?;
    // Re-read under the lock, including user-supplied JSON with a different filename.
    if load(dir)?.contains_key(&(preset.owner_player_id, preset.name.clone())) {
        return Err("当前角色已有同名方案，请另起名字".into());
    }
    let path = dir.join(format!("{}-{}.json", preset.owner_player_id, preset.name));
    if path.exists() {
        return Err("同名文件已存在，请另起名字".into());
    }
    let bytes = serde_json::to_vec_pretty(preset).map_err(|e| e.to_string())?;
    crate::aoqi_storage::atomic_write(&path, &bytes)
}

struct Library {
    plans: Plans,
    loading: bool,
    error: Option<String>,
}
struct Capture {
    token: String,
    owner: u32,
    name: String,
    next: Instant,
    deadline: Instant,
}
pub struct Lineups {
    root: PathBuf,
    library: Arc<Mutex<Library>>,
    pending: Option<Capture>,
    next_token: u64,
    writer: Option<thread::JoinHandle<Result<(), String>>>,
    message: String,
    failed: bool,
}

impl Lineups {
    pub fn new(root: PathBuf) -> Self {
        let root = root.join("lineups");
        let library = Arc::new(Mutex::new(Library {
            plans: Plans::new(),
            loading: true,
            error: None,
        }));
        let target = Arc::clone(&library);
        let folder = root.clone();
        thread::spawn(move || {
            let plans = load(&folder);
            let mut lib = target.lock().unwrap();
            match plans {
                Ok(map) => lib.plans = map,
                Err(e) => lib.error = Some(e),
            }
            lib.loading = false;
        });
        Self {
            root,
            library,
            pending: None,
            next_token: 0,
            writer: None,
            message: String::new(),
            failed: false,
        }
    }
    pub fn reset(&mut self) {
        self.pending = None;
        // A completed capture belongs to its original owner and can finish its atomic write.
        self.writer.take();
        self.message.clear();
        self.failed = false;
    }
    fn unavailable() -> Value {
        json!({"current":null,"sections":[],"stage":"unavailable","canSave":false,"message":"请在战外打开自己的亚比背包"})
    }
    fn call(player: &PlayerController, name: &str, args: Vec<Flash>) -> Value {
        if let Some(Flash::String(raw)) = player
            .get()
            .and_then(|mut p| p.try_call_internal_interface(name, args))
        {
            if let Ok(value) = serde_json::from_str(&raw) {
                return value;
            }
        }
        Self::unavailable()
    }
    fn snapshot(&self, mut game: Value) -> Value {
        let owner = game["current"]["ownerPlayerId"].as_u64().unwrap_or(0);
        let lib = self.library.lock().unwrap();
        let plans: Vec<_> = lib
            .plans
            .keys()
            .filter(|(id, _)| u64::from(*id) == owner)
            .map(|(_, name)| name)
            .collect();
        game["plans"] = json!(plans);
        if let Some(e) = &lib.error {
            game["stage"] = json!("failed");
            game["message"] = json!(e);
            game["canSave"] = json!(false);
        } else if lib.loading || self.pending.is_some() || self.writer.is_some() {
            game["stage"] = json!(if lib.loading { "loading" } else { "saving" });
            game["canSave"] = json!(false);
            if self.writer.is_some() {
                game["message"] = json!("正在写入本地方案");
            }
        } else if game["stage"] == "saving"
            && game["captureId"]
                .as_str()
                .is_some_and(|s| s.starts_with("read-"))
        {
            game["stage"] = json!("loading");
        } else if !self.message.is_empty()
            && game["stage"] != "unavailable"
            && game["stage"] != "restoring"
        {
            game["message"] = json!(self.message);
            if self.failed {
                game["stage"] = json!("failed");
            }
        }
        game
    }
    fn fail(&mut self, text: impl Into<String>) {
        self.message = text.into();
        self.failed = true;
    }
    pub fn tick(&mut self, player: &PlayerController, in_battle: fn() -> bool) {
        if self.writer.as_ref().is_some_and(|w| w.is_finished()) {
            match self.writer.take().unwrap().join() {
                Ok(Ok(())) => self.message = "已保存两次回读一致的背包与配置".into(),
                Ok(Err(e)) => self.fail(e),
                Err(_) => self.fail("保存阵容失败"),
            }
        }
        let Some(wait) = &mut self.pending else {
            return;
        };
        let now = Instant::now();
        if now < wait.next {
            return;
        }
        wait.next = now + Duration::from_millis(100);
        let game = if in_battle() {
            Self::unavailable()
        } else {
            Self::call(
                player,
                "aolaLineupRead",
                vec![Flash::String(wait.token.clone())],
            )
        };
        let same = game["captureId"].as_str() == Some(&wait.token);
        if now < wait.deadline && same && game["stage"] == "saving" {
            return;
        }
        let wait = self.pending.take().unwrap();
        if !same || game["stage"] != "ready" || game["current"]["ownerPlayerId"] != wait.owner {
            self.fail(
                game["message"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("保存期间背包已关闭、回读超时或角色变更"),
            );
            return;
        }
        if game["captured"] != game["current"] {
            self.fail("两次回读完成后背包又发生变化，请重新保存");
            return;
        }
        let Ok(pets) = serde_json::from_value(game["captured"]["pets"].clone()) else {
            self.fail("背包资料格式无效");
            return;
        };
        let preset = Preset {
            schema_version: 1,
            owner_player_id: wait.owner,
            name: wait.name,
            saved_at: chrono::Utc::now().to_rfc3339(),
            pets,
        };
        let library = self.library.clone();
        let dir = self.root.clone();
        self.writer = Some(thread::spawn(move || {
            write(&dir, &preset)?;
            library
                .lock()
                .unwrap()
                .plans
                .insert((preset.owner_player_id, preset.name.clone()), preset);
            Ok(())
        }));
    }
    fn capture(
        &mut self,
        player: &PlayerController,
        prefix: &str,
    ) -> Result<(String, Value), String> {
        self.next_token += 1;
        let token = format!("{prefix}-{}", self.next_token);
        let result = Self::call(
            player,
            "aolaLineupCapture",
            vec![Flash::String(token.clone())],
        );
        if result["captureId"] != token
            || !matches!(result["stage"].as_str(), Some("ready" | "saving"))
        {
            return Err("背包未接受读取请求".into());
        }
        Ok((token, result))
    }
    pub fn command(
        &mut self,
        name: &str,
        payload: &Value,
        player: &PlayerController,
        in_battle: bool,
    ) -> Option<Result<Value, String>> {
        if !matches!(
            name,
            "lineups.read" | "lineups.refresh" | "lineups.save" | "lineups.restore"
        ) {
            return None;
        }
        let game = if in_battle {
            Self::unavailable()
        } else {
            Self::call(player, "aolaLineupRead", vec![])
        };
        let state = self.snapshot(game);
        if name == "lineups.read" {
            return Some(Ok(state));
        }
        Some((|| {
            if state["canSave"] != true {
                return Err("当前背包未就绪或有阵容任务正在执行".into());
            }
            self.message.clear();
            self.failed = false;
            if name == "lineups.refresh" {
                let (_, latest) = self.capture(player, "read")?;
                return Ok(self.snapshot(latest));
            }
            let label = payload["name"].as_str().ok_or("请输入方案名称")?;
            validate_name(label)?;
            let owner = state["current"]["ownerPlayerId"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|id| *id > 0)
                .ok_or("无法识别背包所属角色")?;
            let key = (owner, label.to_owned());
            if name == "lineups.save" {
                if self.library.lock().unwrap().plans.contains_key(&key) {
                    return Err("当前角色已有同名方案，请另起名字".into());
                }
                let (token, feedback) = self.capture(player, "save")?;
                let now = Instant::now();
                self.pending = Some(Capture {
                    token,
                    owner,
                    name: label.to_owned(),
                    next: now,
                    deadline: now + Duration::from_secs(20),
                });
                return Ok(self.snapshot(feedback));
            }
            let section = payload["section"]
                .as_str()
                .filter(|s| matches!(*s, "order" | "cards" | "artifacts" | "keys"))
                .ok_or("请选择恢复项目")?;
            if !state["sections"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == section))
            {
                return Err("本次游戏未提供此项操作".into());
            }
            let lib = self.library.lock().unwrap();
            let plan = lib.plans.get(&key).ok_or("本角色没有这个阵容方案")?;
            let dto = serde_json::to_string(plan).map_err(|e| e.to_string())?;
            drop(lib);
            let feedback = Self::call(
                player,
                "aolaLineupRestore",
                vec![Flash::String(dto), Flash::String(section.into())],
            );
            Ok(self.snapshot(feedback))
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lineup_json_preserves_empty_artifact_slots_owner_and_name() {
        let dir = std::env::temp_dir().join(format!(
            "aola-lineup-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&dir).unwrap();
        let mut plan:Preset=serde_json::from_value(json!({"schemaVersion":1,"name":"测试一","savedAt":"now","ownerPlayerId":77,
            "pets":[{"petId":2,"raceId":5336,"name":"甲","soulCards":null,"crystalKeys":[9,0],"soulArtifacts":[null,{"instanceId":7,"configId":11}]}]})).unwrap();
        write(&dir, &plan).unwrap();
        assert!(write(&dir, &plan).is_err());
        plan.owner_player_id = 88;
        write(&dir, &plan).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded[&(77, "测试一".into())].pets[0]
                .soul_artifacts
                .as_ref()
                .unwrap()[0]
                .is_none()
        );
        plan.pets.push(plan.pets[0].clone());
        assert!(plan.validate().is_err());
        assert!(validate_name("../改名").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
