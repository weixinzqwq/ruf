//! Bounded, read-only observations for the native MyPro tools window.
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT: usize = 512;
const FILE_LIMIT: u64 = 4 * 1024 * 1024;
const KINDS: [&str; 4] = ["resource", "panel", "battle", "backpack"];
static LIVE: OnceLock<Mutex<Live>> = OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default, Clone)]
struct Records {
    rows: VecDeque<Arc<Value>>,
    dropped: u64,
}

#[derive(Default)]
struct Live {
    records: Records,
    next_seq: u64,
    enabled: bool,
    session: String,
    battle_id: Option<u64>,
    battle_ended: bool,
    switches: u64,
    updated: Option<u64>,
}

impl Live {
    fn push(&mut self, kind: &str, source: &str, summary: &str, payload: Value) {
        if !self.enabled {
            return;
        }
        self.next_seq += 1;
        if self.records.rows.len() == LIMIT {
            self.records.rows.pop_front();
            self.records.dropped += 1;
        }
        self.records.rows.push_back(Arc::new(json!({"seq":self.next_seq, "timeMs":now_ms(),
            "sessionId":self.session, "kind":kind, "source":source, "summary":summary, "payload":payload})));
    }

    fn battle(&self) -> Value {
        json!({"state":if self.battle_id.is_none() {"unknown"} else if self.battle_ended {"ended"} else {"active"},
            "battleId":self.battle_id, "switches":self.battle_id.map(|_|self.switches), "updatedMs":self.updated,
            "source":"petfightnew/FightPanel + SwitchPm"})
    }
}

// Do not retain query strings, file directory paths, credentials, fragments, login pages or opaque URLs.
fn asset_address(raw: &str, relative: bool) -> Option<String> {
    if raw.len() > 2048 {
        return None;
    }
    let address = raw.split(['?', '#']).next()?;
    let parsed = url::Url::parse(address).ok();
    let (prefix, path) = match parsed.as_ref() {
        Some(url)
            if matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            (url.origin().ascii_serialization(), url.path())
        }
        Some(url) if url.scheme() == "file" => {
            ("local:/".to_owned(), url.path_segments()?.next_back()?)
        }
        None if relative && !address.starts_with("//") && !address.contains([':', '\\']) => {
            (String::new(), address)
        }
        _ => return None,
    };
    if path.len() > 768
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/%.-_~".contains(&b))
    {
        return None;
    }
    if path.split('/').any(|segment| {
        let name = segment.split('.').next().unwrap_or_default();
        [
            "login", "auth", "oauth", "account", "session", "ticket", "token", "password",
            "captcha",
        ]
        .iter()
        .any(|s| name.eq_ignore_ascii_case(s))
    }) {
        return None;
    }
    let (_, ext) = path.rsplit_once('.')?;
    if ![
        "swf", "xml", "json", "png", "jpg", "jpeg", "gif", "webp", "atlas", "mp3", "bin", "dat",
    ]
    .iter()
    .any(|e| ext.eq_ignore_ascii_case(e))
    {
        return None;
    }
    Some(format!("{prefix}{path}"))
}

fn asset_event(kind: &str, raw: &str, class: &str) {
    let Some(live) = LIVE.get() else { return };
    let is_panel = matches!(kind, "panel" | "panel-error");
    let Some(url) = asset_address(raw, is_panel) else {
        return;
    };
    let (source, payload) = match kind {
        "resource" => ("navigator.fetch", json!({"url":url,"stage":"requested"})),
        "panel" | "panel-error"
            if !class.is_empty()
                && class.len() <= 192
                && class
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:$".contains(&b)) =>
        {
            (
                "Panel_Helper.loadResAndShow",
                json!({"url":url,"cls":class,"stage":if kind=="panel" {"returned"} else {"threw"}}),
            )
        }
        _ => return,
    };
    live.lock()
        .unwrap()
        .push(if is_panel { "panel" } else { kind }, source, &url, payload);
}

pub fn in_battle() -> bool {
    LIVE.get().is_some_and(|live| {
        let live = live.lock().unwrap();
        live.battle_id.is_some() && !live.battle_ended
    })
}

pub fn game_event(row: &Value) {
    let Some(state) = LIVE.get() else { return };
    let Some(op) = row["operation"].as_str() else {
        return;
    };
    let mut live = state.lock().unwrap();
    match op {
        "battle-enter" | "battle-exit" => {
            let Some(id) = row["battleId"].as_u64() else {
                return;
            };
            if op == "battle-enter" {
                live.battle_id = Some(id);
                live.switches = 0;
            } else if live.battle_id != Some(id) {
                return;
            }
            live.battle_ended = op == "battle-exit";
            live.updated = Some(now_ms());
            live.push(
                "battle",
                "FightPanel",
                op,
                json!({"operation":op,"battleId":id}),
            );
            drop(live);
            crate::mypro_features::battle_event(op, id);
        }
        "pet-switch-completed" if live.battle_id.is_some() && !live.battle_ended => {
            live.switches += 1;
            live.updated = Some(now_ms());
            let payload = json!({"battleId":live.battle_id,"switch":live.switches});
            live.push("battle", "SwitchPm.switchPet", op, payload);
        }
        "panel-created" | "view-switch" | "panel-cleanup" | "classify-return" => {
            live.push("backpack", "newpetpackage", op, json!({"operation":op}))
        }
        _ => {}
    }
}

fn valid_kind(p: &Value) -> Result<Option<&str>, String> {
    if p.get("kind").is_none_or(Value::is_null) {
        return Ok(None);
    }
    p["kind"]
        .as_str()
        .filter(|k| KINDS.contains(k))
        .map(Some)
        .ok_or_else(|| "InvalidArguments".into())
}
fn uint(p: &Value, key: &str) -> Result<Option<u64>, String> {
    p.get(key)
        .map(|v| v.as_u64().ok_or_else(|| "InvalidArguments".into()))
        .transpose()
}

fn page(records: &Records, p: &Value, source: &str, enabled: bool) -> Result<Value, String> {
    let after = uint(p, "after")?;
    let limit = uint(p, "limit")?.unwrap_or(40);
    if !(1..=100).contains(&limit) {
        return Err("InvalidArguments".into());
    }
    let kind = valid_kind(p)?;
    let first = records.rows.front().and_then(|r| r["seq"].as_u64());
    let last = records.rows.back().and_then(|r| r["seq"].as_u64());
    let mut matches: Vec<&Value> = records
        .rows
        .iter()
        .map(AsRef::as_ref)
        .filter(|r| {
            kind.is_none_or(|k| r["kind"] == k)
                && after.is_none_or(|n| r["seq"].as_u64().is_some_and(|s| s > n))
        })
        .collect();
    let more = after.is_some() && matches.len() > limit as usize;
    if after.is_none() {
        matches = matches.into_iter().rev().take(limit as usize).collect();
        matches.reverse();
    } else {
        matches.truncate(limit as usize);
    }
    let cursor = more
        .then(|| matches.last().and_then(|r| r["seq"].as_u64()))
        .flatten();
    let gap = after
        .zip(first)
        .is_some_and(|(a, f)| records.dropped > 0 && a.saturating_add(1) < f);
    Ok(
        json!({"source":source,"enabled":enabled,"rows":matches,"oldestSeq":first,"newestSeq":last,"nextAfter":cursor,"gap":gap,"dropped":records.dropped}),
    )
}

struct ArchiveResult {
    file: String,
    entries: usize,
    imported: Option<Records>,
}
type ArchiveRx = mpsc::Receiver<Result<ArchiveResult, String>>;
pub struct ToolsService {
    packets: crate::mypro_features::network::Packets,
    dir: PathBuf,
    imported: Records,
    job: u64,
    rx: Option<ArchiveRx>,
    status: Value,
}

impl ToolsService {
    pub fn new(root: PathBuf) -> Self {
        LIVE.get_or_init(Default::default);
        ruffle_core::mypro_observer::install(asset_event);
        Self {
            packets: crate::mypro_features::network::Packets::new(&root),
            dir: root.join("mypro").join("recordings"),
            imported: Records::default(),
            job: 0,
            rx: None,
            status: Value::Null,
        }
    }

    pub fn set_session(&mut self, id: &str) {
        let mut live = LIVE.get().unwrap().lock().unwrap();
        if live.session != id {
            self.packets.reset();
            let enabled = live.enabled;
            *live = Live {
                session: id.to_owned(),
                enabled,
                ..Default::default()
            };
            self.rx = None;
            self.status = Value::Null;
            self.imported = Records::default();
        }
    }

    pub fn capabilities(&self) -> Value {
        let mut caps = serde_json::Map::new();
        for key in [
            "capture.configure",
            "capture.page",
            "capture.export",
            "capture.import",
            "capture.job",
            "battle.snapshot",
        ] {
            caps.insert(key.into(), json!({"available":true,"reason":""}));
        }
        for key in [
            "battle.details",
            "capture.business",
            "capture.transport",
            "packets.page",
            "packets.retain.set",
            "packets.export",
        ] {
            caps.insert(key.into(), json!({"available":self.packets.available(),"reason":if self.packets.available(){""}else{"抓包后台不可用"}}));
        }
        Value::Object(caps)
    }


    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("IoError".into()),
        };
        self.rx = None;
        self.status = match result {
            Ok(out) => {
                if let Some(imported) = out.imported {
                    self.imported = imported;
                }
                json!({"jobId":self.job,"state":"completed","file":out.file,"rows":out.entries})
            }
            Err(e) => json!({"jobId":self.job,"state":"failed","error":e}),
        };
    }

    fn archive(&mut self, p: &Value, import: bool) -> Result<Value, String> {
        self.poll();
        if self.rx.is_some() {
            return Err("Busy".into());
        }
        let name = p
            .get("file")
            .map(|v| {
                v.as_str()
                    .filter(|v| valid_filename(v))
                    .map(str::to_owned)
                    .ok_or("InvalidArguments")
            })
            .transpose()?;
        let snapshot = if import {
            None
        } else {
            let kind = valid_kind(p)?;
            let lo = uint(p, "fromSeq")?.unwrap_or(0);
            let hi = uint(p, "toSeq")?.unwrap_or(u64::MAX);
            if lo > hi {
                return Err("InvalidArguments".into());
            }
            let live = LIVE.get().unwrap().lock().unwrap();
            let mut selection = live.records.clone();
            selection.rows.retain(|r| {
                r["seq"].as_u64().is_some_and(|n| n >= lo && n <= hi)
                    && kind.is_none_or(|k| r["kind"] == k)
            });
            Some(selection)
        };
        let dir = self.dir.clone();
        self.job += 1;
        let id = self.job;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("mypro-recordings".into())
            .spawn(move || {
                let result = match snapshot {
                    Some(r) => export(&dir, &r, id),
                    None => import_file(&dir, name),
                };
                let _ = tx.send(result);
            })
            .map_err(|_| "IoError")?;
        self.rx = Some(rx);
        self.status = json!({"jobId":id,"state":"running"});
        Ok(self.status.clone())
    }

    pub fn command(&mut self, name: &str, p: &Value) -> Option<Result<Value, String>> {
        Some(match name {
            "packets.page" | "packets.retain.set" | "packets.export" => {
                self.packets.command(name, p)
            }
            "capture.configure" => p["enabled"]
                .as_bool()
                .ok_or_else(|| "InvalidArguments".to_string())
                .map(|enabled| {
                    LIVE.get().unwrap().lock().unwrap().enabled = enabled;
                    ruffle_core::mypro_observer::set_recording(enabled);
                    json!({"enabled":enabled})
                }),
            "capture.page" => {
                self.poll();
                let live = LIVE.get().unwrap().lock().unwrap();
                match p["source"].as_str().unwrap_or("live") {
                    "live" => page(&live.records, p, "live", live.enabled),
                    "import" => page(&self.imported, p, "import", live.enabled),
                    _ => Err("InvalidArguments".into()),
                }
            }
            "capture.export" => self.archive(p, false),
            "capture.import" => self.archive(p, true),
            "capture.job" => {
                self.poll();
                if self.job != 0 && p["jobId"].as_u64() == Some(self.job) && !self.status.is_null()
                {
                    Ok(self.status.clone())
                } else {
                    Err("UnknownJob".into())
                }
            }
            "battle.snapshot" => {
                let mut snapshot = LIVE.get().unwrap().lock().unwrap().battle();
                snapshot["details"] = self.packets.battle_details();
                Ok(snapshot)
            }
            _ => return None,
        })
    }
}

fn valid_filename(name: &str) -> bool {
    name.len() <= 72
        && name
            .strip_prefix("capture-")
            .and_then(|v| v.strip_suffix(".jsonl"))
            .is_some_and(|stem| {
                !stem.is_empty()
                    && stem
                        .split('-')
                        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            })
}

fn export(dir: &Path, records: &Records, id: u64) -> Result<ArchiveResult, String> {
    fs::create_dir_all(dir).map_err(|_| "IoError")?;
    let name = format!("capture-{}-{}-{id}.jsonl", now_ms(), std::process::id());
    let dest = dir.join(&name);
    let tmp = dest.with_extension("part");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|_| "IoError")?;
    let written = (|| -> std::io::Result<()> {
        writeln!(
            file,
            "{}",
            json!({"schemaVersion":1,"streamId":"mypro-observations","dropped":records.dropped,"count":records.rows.len()})
        )?;
        for r in &records.rows {
            writeln!(file, "{}", r.as_ref())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &dest)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err("IoError".into());
    }
    Ok(ArchiveResult {
        file: name,
        entries: records.rows.len(),
        imported: None,
    })
}

fn import_file(dir: &Path, name: Option<String>) -> Result<ArchiveResult, String> {
    let name = match name {
        Some(name) => name,
        None => fs::read_dir(dir)
            .map_err(|_| "MissingFile")?
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| valid_filename(name))
            .max()
            .ok_or("MissingFile")?,
    };
    let path = dir.join(&name);
    if !fs::symlink_metadata(&path)
        .map_err(|_| "MissingFile")?
        .file_type()
        .is_file()
    {
        return Err("InvalidRecording".into());
    }
    let mut contents = String::new();
    File::open(path)
        .map_err(|_| "IoError")?
        .take(FILE_LIMIT + 1)
        .read_to_string(&mut contents)
        .map_err(|_| "InvalidRecording")?;
    if contents.len() as u64 > FILE_LIMIT {
        return Err("InvalidRecording".into());
    }
    let mut lines = contents.lines();
    let head: Value = serde_json::from_str(lines.next().ok_or("InvalidRecording")?)
        .map_err(|_| "InvalidRecording")?;
    if head["schemaVersion"] != 1 || head["streamId"] != "mypro-observations" {
        return Err("InvalidRecording".into());
    }
    let count = head["count"]
        .as_u64()
        .filter(|n| *n <= LIMIT as u64)
        .ok_or("InvalidRecording")?;
    let mut result = Records {
        dropped: head["dropped"].as_u64().ok_or("InvalidRecording")?,
        ..Default::default()
    };
    let mut last = 0;
    for line in lines {
        if result.rows.len() == LIMIT || line.len() > 8192 {
            return Err("InvalidRecording".into());
        }
        let row: Value = serde_json::from_str(line).map_err(|_| "InvalidRecording")?;
        let seq = row["seq"]
            .as_u64()
            .filter(|n| *n > last)
            .ok_or("InvalidRecording")?;
        if row["timeMs"].as_u64().is_none()
            || !row["kind"].as_str().is_some_and(|k| KINDS.contains(&k))
            || !row["payload"].is_object()
            || ["source", "summary", "sessionId"]
                .iter()
                .any(|k| !row[*k].is_string())
        {
            return Err("InvalidRecording".into());
        }
        result.rows.push_back(Arc::new(row));
        last = seq;
    }
    if result.rows.len() as u64 != count {
        return Err("InvalidRecording".into());
    }
    Ok(ArchiveResult {
        file: name,
        entries: result.rows.len(),
        imported: Some(result),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mypro_capture_bounds_and_read_only_roundtrip() {
        let mut live = Live {
            session: "test".into(),
            enabled: true,
            ..Default::default()
        };
        for n in 0..LIMIT + 3 {
            live.push(
                "battle",
                "FightPanel",
                "battle-enter",
                json!({"battleId":n}),
            );
        }
        assert_eq!(live.records.rows.len(), LIMIT);
        let first = page(&live.records, &json!({"after":0,"limit":1}), "live", true).unwrap();
        assert_eq!(first["gap"], true);
        assert_eq!(first["nextAfter"], 4);
        let tail = page(&live.records, &json!({"limit":2}), "live", true).unwrap();
        assert_eq!(tail["rows"].as_array().unwrap().len(), 2);
        assert!(tail["nextAfter"].is_null());
        let dir = std::env::temp_dir().join(format!(
            "mypro-observation-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let saved = export(&dir, &live.records, 1).unwrap();
        let imported = import_file(&dir, Some(saved.file.clone()))
            .unwrap()
            .imported
            .unwrap();
        assert_eq!(saved.entries, LIMIT);
        assert_eq!(imported.rows, live.records.rows);
        assert_eq!(imported.dropped, 3);
        assert!(!valid_filename("../capture-1.jsonl"));
        fs::write(dir.join(&saved.file), "{\"schemaVersion\":1}\n").unwrap();
        assert!(import_file(&dir, Some(saved.file.clone())).is_err());
        fs::remove_file(dir.join(saved.file)).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn mypro_capture_records_asset_metadata_only() {
        assert_eq!(
            asset_address(
                "https://aola.100bt.com/play/a.swf?ticket=secret#private",
                false
            )
            .as_deref(),
            Some("https://aola.100bt.com/play/a.swf")
        );
        assert_eq!(
            asset_address("file:///C:/private/user/movie.swf", false).as_deref(),
            Some("local:/movie.swf")
        );
        assert_eq!(
            asset_address("fixtures/panel.swf?account=1", true).as_deref(),
            Some("fixtures/panel.swf")
        );
        assert!(asset_address("https://x:y@example.com/login.xml", false).is_none());
        assert!(asset_address("https://aola.100bt.com/login.jsp?ticket=secret", false).is_none());
        assert!(
            asset_address(
                "https://aola.100bt.com/session/opaque-token/data.json",
                false
            )
            .is_none()
        );
        assert!(asset_address("https://aola.100bt.com/Auth.xml", false).is_none());
    }

    #[test]
    fn mypro_commands_isolate_imports_from_live_capture() {
        let dir = std::env::temp_dir().join(format!(
            "mypro-command-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let mut service = ToolsService::new(dir.clone());
        let request = |service: &mut ToolsService, name: &str, p: Value| {
            service.command(name, &p).unwrap().unwrap()
        };
        let completed = |service: &mut ToolsService, job: &Value| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
            loop {
                let state = request(service, "capture.job", json!({"jobId":job["jobId"]}));
                if state["state"] != "running" {
                    return state;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        service.set_session("one");
        crate::mypro_features::network::tests::verify_battle_feed(|name| {
            request(&mut service, name, json!({}))
        });
        request(&mut service, "capture.configure", json!({"enabled":true}));
        for kind in ["resource", "panel", "resource"] {
            ruffle_core::mypro_observer::observe(
                kind,
                "https://aola.100bt.com/play/a.swf?ticket=private",
                "AlertPanel",
            );
        }
        let selection = request(
            &mut service,
            "capture.export",
            json!({"kind":"panel","fromSeq":1,"toSeq":3}),
        );
        let saved = completed(&mut service, &selection);
        assert_eq!(saved["state"], "completed");
        assert_eq!(saved["rows"], 1);
        assert!(
            !service
                .dir
                .join(saved["file"].as_str().unwrap())
                .with_extension("part")
                .exists()
        );
        assert!(
            !fs::read_to_string(service.dir.join(saved["file"].as_str().unwrap()))
                .unwrap()
                .contains("private")
        );
        assert!(
            service
                .command("capture.import", &json!({"file":"../capture-1.jsonl"}))
                .unwrap()
                .is_err()
        );
        service.set_session("two");
        let import = request(
            &mut service,
            "capture.import",
            json!({"file":saved["file"]}),
        );
        assert_eq!(completed(&mut service, &import)["state"], "completed");
        let history = request(&mut service, "capture.page", json!({"source":"import"}));
        assert_eq!(history["rows"][0]["source"], "Panel_Helper.loadResAndShow");
        assert_eq!(history["rows"][0]["sessionId"], "one");
        assert_eq!(history["rows"][0]["seq"], 2);
        assert!(
            request(&mut service, "capture.page", json!({"source":"live"}))["rows"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            request(&mut service, "battle.snapshot", json!({}))["state"],
            "unknown"
        );
        request(&mut service, "capture.configure", json!({"enabled":false}));
        ruffle_core::mypro_observer::observe("resource", "https://aola.100bt.com/play/off.swf", "");
        assert!(
            request(&mut service, "capture.page", json!({"source":"live"}))["rows"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        fs::remove_file(service.dir.join(saved["file"].as_str().unwrap())).unwrap();
        fs::remove_dir(&service.dir).unwrap();
        fs::remove_dir(dir.join("mypro")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
